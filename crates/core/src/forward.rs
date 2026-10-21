use crate::config::{Config, CostBasis, IncomeBase, SurchargeBase, SurchargeRounding};
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
    /// 是否因未达起征点免征增值税（同时免附加税）。
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

/// 正算：企业总成本 → 各项税费与实际到手。
///
/// `total_cost` 必须非负，且小数位不超过 `rounding.scale`。
pub fn forward(cfg: &Config, total_cost: Decimal) -> Result<Breakdown, CalcError> {
    let cost = check_amount(cfg, total_cost)?;
    Ok(forward_unchecked(cfg, cost))
}

pub(crate) fn forward_unchecked(cfg: &Config, cost: Decimal) -> Breakdown {
    let vat_exempt = cfg.vat_exempt(cost);
    let rate = cfg.vat.rate;
    let (net, vat, lines, sur_total) = if vat_exempt {
        let (lines, _) = surcharges(cfg, Decimal::ZERO, Decimal::ZERO);
        let lines = lines
            .into_iter()
            .map(|mut l| {
                l.amount = Decimal::ZERO;
                l
            })
            .collect();
        (cost, Decimal::ZERO, lines, Decimal::ZERO)
    } else {
        match cfg.cost.basis {
            CostBasis::VatInclusive => {
                // 价税分离：不含税收入 = 总成本 / (1 + 征收率)，增值税 = 总成本 − 不含税收入
                let net = cfg.round(cost / (Decimal::ONE + rate));
                let vat = cost - net;
                let (lines, t) = surcharges(cfg, vat, net);
                (net, vat, lines, t)
            }
            CostBasis::VatAndSurchargeInclusive => {
                let sr: Decimal = cfg.effective_surcharge_rates().iter().copied().sum();
                let k = match cfg.surcharge.base {
                    SurchargeBase::Vat => rate * sr,
                    SurchargeBase::NetRevenue => sr,
                };
                let net0 = cfg.round(cost / (Decimal::ONE + rate + k));
                let vat = cfg.round(net0 * rate);
                let (lines, t) = surcharges(cfg, vat, net0);
                // 不含税收入倒挤，保证 不含税收入 + 增值税 + 附加税 = 总成本
                (cost - vat - t, vat, lines, t)
            }
        }
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn example_10100() {
        let b = forward(&cfg(), dec!(10100)).unwrap();
        assert_eq!(b.net_revenue, dec!(10000.00));
        assert_eq!(b.vat, dec!(100.00));
        assert_eq!(b.surcharge_total, dec!(3.00));
        assert_eq!(b.iit.income, dec!(10000.00));
        assert_eq!(b.iit.deduction, dec!(2000.00));
        assert_eq!(b.iit.taxable_income, dec!(8000.00));
        assert_eq!(b.iit.rate, dec!(0.2));
        assert_eq!(b.iit.tax, dec!(1600.00));
        assert_eq!(b.take_home, dec!(8397.00));
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

    /// 由不含税收入构造恰好价税分离出该收入的总成本。
    fn cost_for_net(net: Decimal) -> Decimal {
        net * dec!(1.01)
    }

    fn iit_at_net(net: Decimal) -> IitBreakdown {
        let b = forward(&cfg(), cost_for_net(net).round_dp(2)).unwrap();
        assert_eq!(b.net_revenue, net, "构造的总成本应分离出指定收入");
        b.iit
    }

    #[test]
    fn boundary_800() {
        assert_eq!(iit_at_net(dec!(799.99)).tax, dec!(0));
        assert_eq!(iit_at_net(dec!(800.00)).tax, dec!(0));
        let i = iit_at_net(dec!(800.05));
        assert_eq!(i.taxable_income, dec!(0.05));
        assert_eq!(i.tax, dec!(0.01));
    }

    #[test]
    fn boundary_4000() {
        let a = iit_at_net(dec!(4000.00));
        assert_eq!(a.deduction, dec!(800));
        assert_eq!(a.taxable_income, dec!(3200.00));
        assert_eq!(a.tax, dec!(640.00));
        let b = iit_at_net(dec!(4000.01));
        assert_eq!(b.deduction, dec!(800.00));
        assert_eq!(b.taxable_income, dec!(3200.01));
        assert_eq!(b.tax, dec!(640.00));
        let c = iit_at_net(dec!(3999.99));
        assert_eq!(c.deduction, dec!(800));
        assert_eq!(c.taxable_income, dec!(3199.99));
    }

    #[test]
    fn boundary_20000_taxable() {
        // 应纳税所得额 20000 ⇔ 收入 25000
        let a = iit_at_net(dec!(25000.00));
        assert_eq!(a.taxable_income, dec!(20000.00));
        assert_eq!(a.rate, dec!(0.2));
        assert_eq!(a.tax, dec!(4000.00));
        let b = iit_at_net(dec!(25000.02));
        assert_eq!(b.taxable_income, dec!(20000.02));
        assert_eq!(b.rate, dec!(0.3));
        assert_eq!(b.quick_deduction, dec!(2000));
        assert_eq!(b.tax, dec!(4000.01));
    }

    #[test]
    fn boundary_50000_taxable() {
        // 应纳税所得额 50000 ⇔ 收入 62500
        let a = iit_at_net(dec!(62500.00));
        assert_eq!(a.taxable_income, dec!(50000.00));
        assert_eq!(a.rate, dec!(0.3));
        assert_eq!(a.tax, dec!(13000.00));
        let b = iit_at_net(dec!(62500.02));
        assert_eq!(b.rate, dec!(0.4));
        assert_eq!(b.quick_deduction, dec!(7000));
        assert_eq!(b.tax, dec!(13000.01));
    }

    #[test]
    fn vat_threshold_exempt() {
        let mut c = cfg();
        c.vat.threshold_enabled = true;
        c.vat.threshold = dec!(500);
        let a = forward(&c, dec!(499.99)).unwrap();
        assert!(a.vat_exempt);
        assert_eq!(a.vat, dec!(0));
        assert_eq!(a.take_home, dec!(499.99));
        let b = forward(&c, dec!(500.00)).unwrap();
        assert!(!b.vat_exempt);
        assert_eq!(b.net_revenue, dec!(495.05));
        assert_eq!(b.vat, dec!(4.95));
        assert_eq!(b.surcharge_total, dec!(0.15));
        assert_eq!(b.take_home, dec!(494.90));
        c.vat.threshold_inclusive = true;
        assert!(forward(&c, dec!(500.00)).unwrap().vat_exempt);
        assert!(!forward(&c, dec!(500.01)).unwrap().vat_exempt);
    }

    #[test]
    fn cost_includes_surcharge_identity() {
        let mut c = cfg();
        c.cost.basis = CostBasis::VatAndSurchargeInclusive;
        for cost in [dec!(1), dec!(1234.56), dec!(10103), dec!(99999.99)] {
            let b = forward(&c, cost).unwrap();
            assert_eq!(b.net_revenue + b.vat + b.surcharge_total, cost);
        }
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
        let b = forward(&c, dec!(10100)).unwrap();
        let amts: Vec<_> = b.surcharges.iter().map(|l| l.amount).collect();
        assert_eq!(amts, vec![dec!(3.50), dec!(1.50), dec!(1.00)]);
        assert_eq!(b.surcharge_total, dec!(6.00));
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
        let b = forward(&c, dec!(101)).unwrap(); // vat = 1.00
        let s: Decimal = b.surcharges.iter().map(|l| l.amount).sum();
        assert_eq!(s, b.surcharge_total);
        assert_eq!(b.surcharge_total, dec!(0.05));
    }
}
