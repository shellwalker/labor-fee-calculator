use crate::config::{Config, SurchargeBase, SurchargeRounding};
use crate::error::{CalcError, CostGap, CostGapReason, ReachableCost};
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
    /// 收入额（计算个税的每次收入，即不含税收入）。
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

/// 正算结果：全部金额都由不含税收入唯一确定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Breakdown {
    /// 不含税收入（核心变量）。
    pub net_income: Decimal,
    /// 是否因未达征收门槛而不征增值税和附加税。
    pub vat_exempt: bool,
    /// 增值税。
    pub vat: Decimal,
    /// 附加税明细。
    pub surcharges: Vec<SurchargeLine>,
    /// 附加税合计。
    pub surcharge_total: Decimal,
    /// 企业总成本 = 不含税收入 + 增值税 + 附加税。
    pub total_cost: Decimal,
    /// 劳务报酬个税明细。
    pub iit: IitBreakdown,
    /// 实际到手 = 不含税收入 − 代扣个税。
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

/// 由不含税收入计算全部金额（`net` 已校验）。
pub(crate) fn eval_net(cfg: &Config, net: Decimal) -> Breakdown {
    let levied = cfg.levied(net);
    let vat = if levied {
        cfg.round(net * cfg.vat.rate)
    } else {
        Decimal::ZERO
    };
    let (lines, sur_total) = surcharges(cfg, vat, if levied { net } else { Decimal::ZERO });
    let iit = iit(cfg, net);
    let fix = |d: Decimal| {
        let mut x = d;
        x.rescale(cfg.rounding.scale);
        x
    };
    Breakdown {
        net_income: fix(net),
        vat_exempt: !levied,
        vat: fix(vat),
        surcharges: lines
            .into_iter()
            .map(|mut l| {
                l.amount = fix(l.amount);
                l
            })
            .collect(),
        surcharge_total: fix(sur_total),
        total_cost: fix(net + vat + sur_total),
        take_home: fix(net - iit.tax),
        iit: IitBreakdown {
            income: fix(iit.income),
            deduction: fix(iit.deduction),
            taxable_income: fix(iit.taxable_income),
            quick_deduction: fix(iit.quick_deduction),
            tax: fix(iit.tax),
            ..iit
        },
    }
}

/// 正算：不含税收入 → 增值税、附加税、企业总成本、个税、实际到手。
///
/// `net_income` 必须非负，且小数位不超过 `rounding.scale`。
pub fn forward(cfg: &Config, net_income: Decimal) -> Result<Breakdown, CalcError> {
    let net = check_amount(cfg, net_income)?;
    Ok(eval_net(cfg, net))
}

/// 辅助：按企业总成本正算，内部求出对应的不含税收入。
///
/// 企业总成本随不含税收入严格递增，但并非每个“分”都可达：征收门槛处有一段空白区间
/// （默认 999.99 → 1010.30），增值税、附加税逐分舍入也会留下单分空位。
/// 输入落在空位时返回 [`CalcError::CostUnreachable`]，说明原因并给出两侧最近的可达总成本。
pub fn from_total_cost(cfg: &Config, total_cost: Decimal) -> Result<Breakdown, CalcError> {
    let cost = check_amount(cfg, total_cost)?;
    let cost_of = |t: i64| eval_net(cfg, from_ticks(cfg, t)).total_cost;
    // 二分：满足 总成本(收入) ≤ 输入 的最大不含税收入（总成本 ≥ 收入，故收入 ≤ 输入）
    let (mut lo, mut hi) = (0i64, to_ticks(cfg, cost)?);
    while lo < hi {
        let m = lo + (hi - lo + 1) / 2;
        if cost_of(m) <= cost {
            lo = m;
        } else {
            hi = m - 1;
        }
    }
    let lower = eval_net(cfg, from_ticks(cfg, lo));
    if lower.total_cost == cost {
        return Ok(lower);
    }
    let upper = eval_net(cfg, from_ticks(cfg, lo + 1));
    let reason = if lower.vat_exempt != upper.vat_exempt {
        CostGapReason::Threshold
    } else {
        CostGapReason::Rounding
    };
    Err(CalcError::CostUnreachable(Box::new(CostGap {
        total_cost: cost,
        reason,
        lower: ReachableCost {
            net_income: lower.net_income,
            total_cost: lower.total_cost,
        },
        upper: ReachableCost {
            net_income: upper.net_income,
            total_cost: upper.total_cost,
        },
        threshold: (reason == CostGapReason::Threshold).then_some(cfg.vat.threshold),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn cfg() -> Config {
        Config::default()
    }

    fn at(net: Decimal) -> Breakdown {
        forward(&cfg(), net).unwrap()
    }

    #[test]
    fn example_10000() {
        let b = at(dec!(10000));
        assert_eq!(b.net_income, dec!(10000.00));
        assert_eq!(b.vat, dec!(100.00));
        assert_eq!(b.surcharge_total, dec!(3.00));
        assert_eq!(b.total_cost, dec!(10103.00));
        assert_eq!(b.iit.income, dec!(10000.00));
        assert_eq!(b.iit.deduction, dec!(2000.00));
        assert_eq!(b.iit.taxable_income, dec!(8000.00));
        assert_eq!(b.iit.rate, dec!(0.2));
        assert_eq!(b.iit.tax, dec!(1600.00));
        assert_eq!(b.take_home, dec!(8400.00));
    }

    #[test]
    fn threshold_1000() {
        let a = at(dec!(999.99));
        assert!(a.vat_exempt);
        assert_eq!(
            (a.vat, a.surcharge_total, a.total_cost),
            (dec!(0), dec!(0), dec!(999.99))
        );
        assert_eq!(a.take_home, dec!(959.99));
        let b = at(dec!(1000.00));
        assert!(!b.vat_exempt);
        assert_eq!(
            (b.vat, b.surcharge_total, b.total_cost),
            (dec!(10.00), dec!(0.30), dec!(1010.30))
        );
        assert_eq!(b.take_home, dec!(960.00));
        let mut c = cfg();
        c.vat.levy_at_threshold = false;
        assert!(forward(&c, dec!(1000.00)).unwrap().vat_exempt);
        assert!(!forward(&c, dec!(1000.01)).unwrap().vat_exempt);
        c.vat.threshold_enabled = false;
        assert_eq!(forward(&c, dec!(1)).unwrap().vat, dec!(0.01));
    }

    #[test]
    fn vat_half_up() {
        // 1050.50 × 1% = 10.505 → 10.51（四舍五入远离零）
        let b = at(dec!(1050.50));
        assert_eq!(b.vat, dec!(10.51));
        assert_eq!(b.surcharge_total, dec!(0.32));
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
        assert_eq!(at(dec!(799.99)).iit.tax, dec!(0));
        assert_eq!(at(dec!(800.00)).iit.tax, dec!(0));
        let i = at(dec!(800.05)).iit;
        assert_eq!(i.taxable_income, dec!(0.05));
        assert_eq!(i.tax, dec!(0.01));
    }

    #[test]
    fn iit_4000() {
        let a = at(dec!(4000.00)).iit;
        assert_eq!(
            (a.deduction, a.taxable_income, a.tax),
            (dec!(800), dec!(3200.00), dec!(640.00))
        );
        let b = at(dec!(4000.01)).iit;
        assert_eq!(
            (b.deduction, b.taxable_income, b.tax),
            (dec!(800.00), dec!(3200.01), dec!(640.00))
        );
        let c = at(dec!(3999.99)).iit;
        assert_eq!((c.deduction, c.taxable_income), (dec!(800), dec!(3199.99)));
    }

    #[test]
    fn iit_20000_taxable() {
        let a = at(dec!(25000.00)).iit;
        assert_eq!(
            (a.taxable_income, a.rate, a.tax),
            (dec!(20000.00), dec!(0.2), dec!(4000.00))
        );
        let b = at(dec!(25000.02)).iit;
        assert_eq!(
            (b.taxable_income, b.rate, b.quick_deduction, b.tax),
            (dec!(20000.02), dec!(0.3), dec!(2000), dec!(4000.01))
        );
    }

    #[test]
    fn iit_50000_taxable() {
        let a = at(dec!(62500.00)).iit;
        assert_eq!(
            (a.taxable_income, a.rate, a.tax),
            (dec!(50000.00), dec!(0.3), dec!(13000.00))
        );
        let b = at(dec!(62500.02)).iit;
        assert_eq!(
            (b.rate, b.quick_deduction, b.tax),
            (dec!(0.4), dec!(7000), dec!(13000.01))
        );
    }

    #[test]
    fn from_cost_exact() {
        for net in [
            dec!(0),
            dec!(999.99),
            dec!(1000),
            dec!(1050.50),
            dec!(10000),
            dec!(63235.29),
        ] {
            let b = at(net);
            assert_eq!(from_total_cost(&cfg(), b.total_cost).unwrap(), b);
        }
    }

    fn gap(cost: Decimal) -> CostGap {
        match from_total_cost(&cfg(), cost) {
            Err(CalcError::CostUnreachable(g)) => *g,
            other => panic!("总成本 {cost} 应报空位，实际 {other:?}"),
        }
    }

    #[test]
    fn from_cost_threshold_gap() {
        for cost in [dec!(1000.00), dec!(1005.00), dec!(1010.29)] {
            let g = gap(cost);
            assert_eq!(g.reason, CostGapReason::Threshold);
            assert_eq!(
                (g.lower.net_income, g.lower.total_cost),
                (dec!(999.99), dec!(999.99))
            );
            assert_eq!(
                (g.upper.net_income, g.upper.total_cost),
                (dec!(1000.00), dec!(1010.30))
            );
            let msg = CalcError::CostUnreachable(Box::new(g)).to_string();
            assert!(msg.contains("1000 元增值税征收门槛形成的空白区间"), "{msg}");
            assert!(
                msg.contains("不含税收入 999.99 对应总成本 999.99，1000.00 对应 1010.30"),
                "{msg}"
            );
        }
    }

    #[test]
    fn from_cost_rounding_gap() {
        // 例：1050.49 → 1061.31，1050.50 → 1061.33（增值税 10.50 → 10.51），1061.32 不可达
        let mut found = 0;
        for c in 101_030..200_000i64 {
            let cost = Decimal::new(c, 2);
            match from_total_cost(&cfg(), cost) {
                Ok(b) => assert_eq!(b.total_cost, cost),
                Err(CalcError::CostUnreachable(g)) => {
                    found += 1;
                    assert_eq!(g.reason, CostGapReason::Rounding);
                    assert!(g.lower.total_cost < cost && cost < g.upper.total_cost);
                    assert_eq!(g.upper.net_income - g.lower.net_income, dec!(0.01));
                    let msg = CalcError::CostUnreachable(g).to_string();
                    assert!(msg.contains("逐分舍入"), "{msg}");
                }
                Err(e) => panic!("{e}"),
            }
        }
        assert!(found > 0, "应存在逐分舍入造成的空位");
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
        let b = forward(&c, dec!(10000)).unwrap();
        let amts: Vec<_> = b.surcharges.iter().map(|l| l.amount).collect();
        assert_eq!(amts, vec![dec!(3.50), dec!(1.50), dec!(1.00)]);
        assert_eq!(b.surcharge_total, dec!(6.00));
        assert_eq!(b.total_cost, dec!(10106.00));
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
        let b = forward(&c, dec!(1000)).unwrap();
        let s: Decimal = b.surcharges.iter().map(|l| l.amount).sum();
        assert_eq!(s, b.surcharge_total);
        assert_eq!(b.surcharge_total, dec!(0.50));
    }
}
