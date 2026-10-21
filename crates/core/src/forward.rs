use crate::config::{
    Config, CostBasis, IncomeBase, SurchargeBase, SurchargeRounding, ThresholdBasis, ThresholdGap,
};
use crate::error::CalcError;
use rust_decimal::Decimal;
use serde::Serialize;

/// 附加税一个税目的计算结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SurchargeLine {
    pub name: String,
    /// 实际税率（已考虑减半）。
    pub rate: Decimal,
    pub amount: Decimal,
}

/// 劳务报酬个税（预扣预缴）明细。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IitBreakdown {
    /// 收入额（计算个税的每次收入）。
    pub income: Decimal,
    /// 减除费用。
    pub deduction: Decimal,
    /// 应纳税所得额。
    pub taxable_income: Decimal,
    /// 预扣率。
    pub rate: Decimal,
    /// 速算扣除数。
    pub quick_deduction: Decimal,
    /// 代扣个税。
    pub tax: Decimal,
}

/// 正算结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Breakdown {
    /// 企业总成本。
    pub total_cost: Decimal,
    /// 是否不征增值税和附加税（未达征收门槛，或门槛跳变段按不征收处理）。
    pub vat_exempt: bool,
    /// 不含税开票收入。
    pub net_revenue: Decimal,
    /// 增值税。
    pub vat: Decimal,
    /// 附加税明细。
    pub surcharges: Vec<SurchargeLine>,
    /// 附加税合计。
    pub surcharge_total: Decimal,
    /// 劳务报酬个税明细。
    pub iit: IitBreakdown,
    /// 实际到手金额。
    pub take_home: Decimal,
}

fn check_amount(cfg: &Config, amount: Decimal) -> Result<Decimal, CalcError> {
    if amount.is_sign_negative() && !amount.is_zero() {
        return Err(CalcError::NegativeAmount(amount));
    }
    if cfg.round(amount) != amount {
        return Err(CalcError::PrecisionExceeded {
            amount,
            scale: cfg.rounding.scale,
        });
    }
    let mut a = amount;
    a.rescale(cfg.rounding.scale);
    Ok(a)
}

pub(crate) fn validate_amount(cfg: &Config, amount: Decimal) -> Result<Decimal, CalcError> {
    check_amount(cfg, amount)
}

/// 金额 → 最小单位整数（分）。
pub(crate) fn to_ticks(cfg: &Config, d: Decimal) -> Result<i64, CalcError> {
    let m = d * Decimal::from(10i64.pow(cfg.rounding.scale));
    i64::try_from(m.trunc()).map_err(|_| CalcError::Overflow)
}

/// 最小单位整数（分）→ 金额。
pub(crate) fn from_ticks(cfg: &Config, t: i64) -> Decimal {
    Decimal::new(t, cfg.rounding.scale)
}

fn surcharges(cfg: &Config, vat: Decimal, net: Decimal) -> (Vec<SurchargeLine>, Decimal) {
    let base = match cfg.surcharge.base {
        SurchargeBase::Vat => vat,
        SurchargeBase::NetRevenue => net,
    };
    let rates = cfg.effective_surcharge_rates();
    let raw: Vec<Decimal> = rates.iter().map(|r| base * r).collect();
    let (amounts, total) = match cfg.rounding.surcharge {
        SurchargeRounding::PerItem => {
            let a: Vec<Decimal> = raw.iter().map(|x| cfg.round(*x)).collect();
            let t = a.iter().copied().sum();
            (a, t)
        }
        SurchargeRounding::Total => {
            let total = cfg.round(raw.iter().copied().sum());
            // 明细逐项舍入，末项倒挤，保证明细之和等于合计
            let mut a: Vec<Decimal> = raw.iter().map(|x| cfg.round(*x)).collect();
            if let Some(last) = a.last_mut() {
                let others: Decimal = raw.iter().take(raw.len() - 1).map(|x| cfg.round(*x)).sum();
                *last = total - others;
            }
            (a, total)
        }
    };
    let lines = cfg
        .surcharge
        .items
        .iter()
        .zip(rates)
        .zip(amounts)
        .map(|((it, rate), amount)| SurchargeLine {
            name: it.name.clone(),
            rate,
            amount,
        })
        .collect();
    (lines, total)
}

fn zero_surcharges(cfg: &Config) -> (Vec<SurchargeLine>, Decimal) {
    let (mut lines, _) = surcharges(cfg, Decimal::ZERO, Decimal::ZERO);
    for l in &mut lines {
        l.amount = Decimal::ZERO;
    }
    (lines, Decimal::ZERO)
}

fn iit(cfg: &Config, income: Decimal) -> IitBreakdown {
    let c = &cfg.iit;
    let income = income.max(Decimal::ZERO);
    let deduction = if income <= c.fixed_deduction_limit {
        c.fixed_deduction.min(income)
    } else {
        cfg.round(income * c.percent_deduction_rate)
    };
    let taxable_income = (income - deduction).max(Decimal::ZERO);
    let bracket = c
        .brackets
        .iter()
        .find(|b| b.upper.is_none_or(|u| taxable_income <= u))
        .expect("最后一档无上限");
    let tax = cfg
        .round(taxable_income * bracket.rate - bracket.quick_deduction)
        .max(Decimal::ZERO);
    IitBreakdown {
        income,
        deduction,
        taxable_income,
        rate: bracket.rate,
        quick_deduction: bracket.quick_deduction,
        tax,
    }
}

/// 由已确定的总成本、不含税收入、增值税、附加税计算个税与到手。
fn compose(
    cfg: &Config,
    cost: Decimal,
    net: Decimal,
    vat: Decimal,
    sur: (Vec<SurchargeLine>, Decimal),
    vat_exempt: bool,
) -> Breakdown {
    let (lines, sur_total) = sur;
    let base = match cfg.iit.income_base {
        IncomeBase::NetRevenue => net,
        IncomeBase::TotalCost => cost,
    };
    let income = if cfg.iit.deduct_surcharge {
        base - sur_total
    } else {
        base
    };
    let iit = iit(cfg, income);
    let mut take_home = net - iit.tax;
    if cfg.take_home.deduct_surcharge {
        take_home -= sur_total;
    }
    // 统一金额的小数位，便于展示与序列化（不改变数值）
    let fix = |d: Decimal| {
        let mut x = d;
        x.rescale(cfg.rounding.scale);
        x
    };
    Breakdown {
        total_cost: fix(cost),
        vat_exempt,
        net_revenue: fix(net),
        vat: fix(vat),
        surcharges: lines
            .into_iter()
            .map(|mut l| {
                l.amount = fix(l.amount);
                l
            })
            .collect(),
        surcharge_total: fix(sur_total),
        iit: IitBreakdown {
            income: fix(iit.income),
            deduction: fix(iit.deduction),
            taxable_income: fix(iit.taxable_income),
            quick_deduction: fix(iit.quick_deduction),
            tax: fix(iit.tax),
            ..iit
        },
        take_home: fix(take_home),
    }
}

/// 总成本含全部税费口径下，把应征收的总成本拆成 (不含税收入, 增值税, 附加税)，三项之和恒等于总成本。
fn split_taxed(cfg: &Config, cost: Decimal) -> (Decimal, Decimal, (Vec<SurchargeLine>, Decimal)) {
    let rate = cfg.vat.rate;
    let sr: Decimal = cfg.effective_surcharge_rates().iter().copied().sum();
    let k = rate
        + match cfg.surcharge.base {
            SurchargeBase::Vat => rate * sr,
            SurchargeBase::NetRevenue => sr,
        };
    let est = cost / (Decimal::ONE + k);
    // 给定增值税 v：附加税按估算收入取，收入倒挤
    let candidate = |v: Decimal| {
        let sur = surcharges(cfg, v, cfg.round(est));
        let net = cost - v - sur.1;
        (net, v, sur)
    };
    let consistent = |(net, v, sur): &(Decimal, Decimal, (Vec<SurchargeLine>, Decimal))| {
        *net >= Decimal::ZERO
            && cfg.round(*net * rate) == *v
            && surcharges(cfg, *v, *net).1 == sur.1
    };
    let v0 = cfg.round(est * rate);
    let first = candidate(v0);
    if consistent(&first) || !cfg.cost.consistent_search {
        return first;
    }
    let u = cfg.unit();
    for v in [v0 - u, v0 + u] {
        if v < Decimal::ZERO {
            continue;
        }
        let c = candidate(v);
        if consistent(&c) {
            let (net, v, _) = c;
            // 自洽时附加税按最终收入重算（基数为收入时才可能有差别，自洽已保证合计相等）
            return (net, v, surcharges(cfg, v, net));
        }
    }
    first
}

/// 计算总成本 `cost` 对应的完整结果（`cost` 已校验）。
pub(crate) fn eval_cost(cfg: &Config, cost: Decimal) -> Breakdown {
    let rate = cfg.vat.rate;
    // 不征收时 不含税收入 = 总成本，此时门槛判断金额就是总成本
    let exempt_by_cost = !cfg.levied(cost);
    let exempt = || compose(cfg, cost, cost, Decimal::ZERO, zero_surcharges(cfg), true);
    if exempt_by_cost {
        return exempt();
    }
    let (net, vat, sur) = match cfg.cost.basis {
        CostBasis::VatInclusive => {
            // 价税分离：不含税收入 = 总成本 / (1 + 征收率)，增值税 = 总成本 − 不含税收入
            let net = cfg.round(cost / (Decimal::ONE + rate));
            let vat = cost - net;
            (net, vat, surcharges(cfg, vat, net))
        }
        CostBasis::VatAndSurchargeInclusive => split_taxed(cfg, cost),
    };
    // 门槛跳变段：按不征收时收入已达门槛，按征收拆分后收入又低于门槛
    if cfg.vat.threshold_basis == ThresholdBasis::NetRevenue
        && cfg.vat.threshold_gap == ThresholdGap::Exempt
        && !cfg.levied(net)
    {
        return exempt();
    }
    compose(cfg, cost, net, vat, sur, false)
}

/// 正算：企业总成本 → 各项税费与实际到手。
///
/// `total_cost` 必须非负，且小数位不超过 `rounding.scale`。
/// 结果恒满足 不含税收入 + 增值税 + 附加税 = 总成本（价税合计口径下附加税不计入总成本）。
pub fn forward(cfg: &Config, total_cost: Decimal) -> Result<Breakdown, CalcError> {
    let cost = check_amount(cfg, total_cost)?;
    Ok(eval_cost(cfg, cost))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn cfg() -> Config {
        Config::default()
    }

    /// 阶段一的口径：价税合计、无门槛、到手扣附加税（用于验证可选口径仍然正确）。
    pub fn legacy() -> Config {
        let mut c = Config::default();
        c.cost.basis = CostBasis::VatInclusive;
        c.vat.threshold_enabled = false;
        c.vat.threshold_basis = ThresholdBasis::TotalCost;
        c.take_home.deduct_surcharge = true;
        c
    }

    /// 四舍五入到分（远离零）。
    fn r2(d: Decimal) -> Decimal {
        d.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
    }

    /// 默认口径下由不含税收入构造总成本。
    fn cost_for_net(net: Decimal) -> Decimal {
        if net >= dec!(1000) {
            let vat = r2(net * dec!(0.01));
            net + vat + r2(vat * dec!(0.03))
        } else {
            net
        }
    }

    fn at_net(net: Decimal) -> Breakdown {
        let b = forward(&cfg(), cost_for_net(net)).unwrap();
        assert_eq!(b.net_revenue, net, "构造的总成本应对应指定收入");
        b
    }

    #[test]
    fn default_example_10103() {
        let b = forward(&cfg(), dec!(10103)).unwrap();
        assert_eq!(b.net_revenue, dec!(10000.00));
        assert_eq!(b.vat, dec!(100.00));
        assert_eq!(b.surcharge_total, dec!(3.00));
        assert_eq!(b.iit.income, dec!(10000.00));
        assert_eq!(b.iit.deduction, dec!(2000.00));
        assert_eq!(b.iit.taxable_income, dec!(8000.00));
        assert_eq!(b.iit.rate, dec!(0.2));
        assert_eq!(b.iit.tax, dec!(1600.00));
        assert_eq!(b.take_home, dec!(8400.00), "附加税由企业承担，不从到手扣");
    }

    #[test]
    fn threshold_1000_boundary() {
        let below = at_net(dec!(999.99));
        assert!(below.vat_exempt);
        assert_eq!(below.total_cost, dec!(999.99));
        assert_eq!(below.vat, dec!(0));
        assert_eq!(below.surcharge_total, dec!(0));
        assert_eq!(below.iit.tax, dec!(40.00));
        assert_eq!(below.take_home, dec!(959.99));
        let at = at_net(dec!(1000.00));
        assert!(!at.vat_exempt, "等于门槛（含本数）即征收");
        assert_eq!(at.vat, dec!(10.00));
        assert_eq!(at.surcharge_total, dec!(0.30));
        assert_eq!(at.total_cost, dec!(1010.30));
        assert_eq!(at.take_home, dec!(960.00));
        // 不含本数
        let mut c = cfg();
        c.vat.levy_at_threshold = false;
        assert!(forward(&c, dec!(1000.00)).unwrap().vat_exempt);
        assert!(!forward(&c, dec!(1010.31)).unwrap().vat_exempt);
    }

    fn split(c: &Config, cost: Decimal) -> (Decimal, Decimal, Decimal, bool) {
        let b = forward(c, cost).unwrap();
        assert_eq!(b.net_revenue + b.vat + b.surcharge_total, cost);
        (b.net_revenue, b.vat, b.surcharge_total, b.vat_exempt)
    }

    #[test]
    fn threshold_gap_default_levy() {
        // 跳变段 1000.00 ~ 1010.29 默认按征收处理，倒挤收入略低于 1000
        let c = cfg();
        assert_eq!(
            split(&c, dec!(999.99)),
            (dec!(999.99), dec!(0), dec!(0), true)
        );
        assert_eq!(
            split(&c, dec!(1000.00)),
            (dec!(989.80), dec!(9.90), dec!(0.30), false)
        );
        assert_eq!(
            split(&c, dec!(1005.00)),
            (dec!(994.75), dec!(9.95), dec!(0.30), false)
        );
        assert_eq!(
            split(&c, dec!(1010.29)),
            (dec!(999.99), dec!(10.00), dec!(0.30), false)
        );
        assert_eq!(
            split(&c, dec!(1010.30)),
            (dec!(1000.00), dec!(10.00), dec!(0.30), false)
        );
    }

    #[test]
    fn threshold_gap_exempt_option() {
        let mut c = cfg();
        c.vat.threshold_gap = ThresholdGap::Exempt;
        assert_eq!(
            split(&c, dec!(1000.00)),
            (dec!(1000.00), dec!(0), dec!(0), true)
        );
        assert_eq!(
            split(&c, dec!(1005.00)),
            (dec!(1005.00), dec!(0), dec!(0), true)
        );
        assert_eq!(
            split(&c, dec!(1010.29)),
            (dec!(1010.29), dec!(0), dec!(0), true)
        );
        assert_eq!(
            split(&c, dec!(1010.30)),
            (dec!(1000.00), dec!(10.00), dec!(0.30), false)
        );
    }

    #[test]
    fn cost_identity_and_vat_formula() {
        let mut inconsistent = 0;
        for c in 0..300_000i64 {
            let cost = Decimal::new(c * 7 + 3, 2);
            let b = forward(&cfg(), cost).unwrap();
            assert_eq!(b.net_revenue + b.vat + b.surcharge_total, cost);
            assert_eq!(b.take_home, b.net_revenue - b.iit.tax);
            if !b.vat_exempt {
                let v = r2(b.net_revenue * dec!(0.01));
                assert!((v - b.vat).abs() <= dec!(0.01));
                if v != b.vat {
                    inconsistent += 1;
                }
                assert_eq!(b.surcharge_total, r2(b.vat * dec!(0.03)));
            }
        }
        // 逐分舍入导致的不自洽只占极少数
        assert!(inconsistent < 300_000 / 50, "不自洽 {inconsistent} 个");
    }

    #[test]
    fn consistent_search_off() {
        let mut c = cfg();
        c.cost.consistent_search = false;
        for cents in 100_000..200_000i64 {
            let cost = Decimal::new(cents, 2);
            let b = forward(&c, cost).unwrap();
            assert_eq!(b.net_revenue + b.vat + b.surcharge_total, cost);
        }
    }

    #[test]
    fn zero_cost() {
        let b = forward(&cfg(), dec!(0)).unwrap();
        assert_eq!(b.take_home, dec!(0));
        assert_eq!(b.iit.tax, dec!(0));
    }

    #[test]
    fn rejects_negative_and_precision() {
        assert!(matches!(
            forward(&cfg(), dec!(-1)),
            Err(CalcError::NegativeAmount(_))
        ));
        assert!(matches!(
            forward(&cfg(), dec!(1.001)),
            Err(CalcError::PrecisionExceeded { .. })
        ));
    }

    #[test]
    fn iit_800() {
        assert_eq!(at_net(dec!(799.99)).iit.tax, dec!(0));
        assert_eq!(at_net(dec!(800.00)).iit.tax, dec!(0));
        let i = at_net(dec!(800.05)).iit;
        assert_eq!(i.taxable_income, dec!(0.05));
        assert_eq!(i.tax, dec!(0.01));
    }

    #[test]
    fn iit_4000() {
        let a = at_net(dec!(4000.00)).iit;
        assert_eq!(a.deduction, dec!(800));
        assert_eq!(a.taxable_income, dec!(3200.00));
        assert_eq!(a.tax, dec!(640.00));
        let b = at_net(dec!(4000.01)).iit;
        assert_eq!(b.deduction, dec!(800.00));
        assert_eq!(b.taxable_income, dec!(3200.01));
        assert_eq!(b.tax, dec!(640.00));
        let c = at_net(dec!(3999.99)).iit;
        assert_eq!(c.deduction, dec!(800));
        assert_eq!(c.taxable_income, dec!(3199.99));
    }

    #[test]
    fn iit_20000_taxable() {
        // 应纳税所得额 20000 ⇔ 收入 25000
        let a = at_net(dec!(25000.00)).iit;
        assert_eq!(a.taxable_income, dec!(20000.00));
        assert_eq!(a.rate, dec!(0.2));
        assert_eq!(a.tax, dec!(4000.00));
        let b = at_net(dec!(25000.02)).iit;
        assert_eq!(b.taxable_income, dec!(20000.02));
        assert_eq!(b.rate, dec!(0.3));
        assert_eq!(b.quick_deduction, dec!(2000));
        assert_eq!(b.tax, dec!(4000.01));
    }

    #[test]
    fn iit_50000_taxable() {
        // 应纳税所得额 50000 ⇔ 收入 62500
        let a = at_net(dec!(62500.00)).iit;
        assert_eq!(a.taxable_income, dec!(50000.00));
        assert_eq!(a.rate, dec!(0.3));
        assert_eq!(a.tax, dec!(13000.00));
        let b = at_net(dec!(62500.02)).iit;
        assert_eq!(b.rate, dec!(0.4));
        assert_eq!(b.quick_deduction, dec!(7000));
        assert_eq!(b.tax, dec!(13000.01));
    }

    #[test]
    fn legacy_vat_inclusive() {
        let b = forward(&legacy(), dec!(10100)).unwrap();
        assert_eq!(b.net_revenue, dec!(10000.00));
        assert_eq!(b.vat, dec!(100.00));
        assert_eq!(b.surcharge_total, dec!(3.00));
        assert_eq!(b.take_home, dec!(8397.00));
    }

    #[test]
    fn legacy_cost_threshold() {
        let mut c = legacy();
        c.vat.threshold_enabled = true;
        c.vat.threshold = dec!(500);
        let a = forward(&c, dec!(499.99)).unwrap();
        assert!(a.vat_exempt);
        assert_eq!(a.take_home, dec!(499.99));
        let b = forward(&c, dec!(500.00)).unwrap();
        assert!(!b.vat_exempt);
        assert_eq!(b.net_revenue, dec!(495.05));
        assert_eq!(b.vat, dec!(4.95));
        assert_eq!(b.surcharge_total, dec!(0.15));
        assert_eq!(b.take_home, dec!(494.90));
        c.vat.levy_at_threshold = false;
        assert!(forward(&c, dec!(500.00)).unwrap().vat_exempt);
        assert!(!forward(&c, dec!(500.01)).unwrap().vat_exempt);
    }

    #[test]
    fn legacy_net_threshold_gap() {
        // 价税合计口径 + 门槛按不含税收入：总成本 1000.00 ~ 1009.99 价税分离后收入 < 1000
        let mut c = legacy();
        c.vat.threshold_enabled = true;
        c.vat.threshold = dec!(1000);
        c.vat.threshold_basis = ThresholdBasis::NetRevenue;
        let b = forward(&c, dec!(1005)).unwrap();
        assert!(!b.vat_exempt, "默认按征收处理");
        assert_eq!(b.net_revenue, dec!(995.05));
        assert_eq!(b.vat, dec!(9.95));
        c.vat.threshold_gap = ThresholdGap::Exempt;
        let b = forward(&c, dec!(1005)).unwrap();
        assert!(b.vat_exempt);
        assert_eq!(b.net_revenue, dec!(1005.00));
        let b = forward(&c, dec!(1010.00)).unwrap();
        assert!(!b.vat_exempt);
        assert_eq!(b.net_revenue, dec!(1000.00));
    }

    #[test]
    fn itemized_halved_surcharge() {
        let mut c = cfg();
        c.surcharge.halve = true;
        c.surcharge.items = vec![
            crate::SurchargeItem {
                name: "城市维护建设税".into(),
                rate: dec!(0.07),
            },
            crate::SurchargeItem {
                name: "教育费附加".into(),
                rate: dec!(0.03),
            },
            crate::SurchargeItem {
                name: "地方教育附加".into(),
                rate: dec!(0.02),
            },
        ];
        let b = forward(&c, dec!(10106)).unwrap();
        let amts: Vec<_> = b.surcharges.iter().map(|l| l.amount).collect();
        assert_eq!(amts, vec![dec!(3.50), dec!(1.50), dec!(1.00)]);
        assert_eq!(b.surcharge_total, dec!(6.00));
        assert_eq!(b.net_revenue, dec!(10000.00));
    }

    #[test]
    fn surcharge_total_rounding_sums() {
        let mut c = cfg();
        c.rounding.surcharge = SurchargeRounding::Total;
        c.surcharge.items = vec![
            crate::SurchargeItem {
                name: "a".into(),
                rate: dec!(0.035),
            },
            crate::SurchargeItem {
                name: "b".into(),
                rate: dec!(0.015),
            },
        ];
        let b = forward(&c, dec!(1010.50)).unwrap(); // 收入 1000，增值税 10.00
        let s: Decimal = b.surcharges.iter().map(|l| l.amount).sum();
        assert_eq!(s, b.surcharge_total);
        assert_eq!(b.surcharge_total, dec!(0.50));
    }
}
