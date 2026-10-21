//! 反算 ↔ 正算 往返一致性、单调性与大范围遍历测试。

use labor_fee_core::{
    forward, Config, CostBasis, IncomeBase, Reverser, RoundingMode, SurchargeBase, SurchargeItem,
    SurchargeRounding,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn to_cents(d: Decimal) -> i64 {
    i64::try_from((d * dec!(100)).trunc()).unwrap()
}

/// 测试用的多种口径组合。
fn configs() -> Vec<(&'static str, Config)> {
    let base = Config::default();
    let mut threshold = base.clone();
    threshold.vat.threshold_enabled = true;
    threshold.vat.threshold = dec!(500);
    let mut cost_b = base.clone();
    cost_b.cost.basis = CostBasis::VatAndSurchargeInclusive;
    let mut net_base = base.clone();
    net_base.surcharge.base = SurchargeBase::NetRevenue;
    net_base.iit.deduct_surcharge = true;
    let mut itemized = base.clone();
    itemized.vat.rate = dec!(0.03);
    itemized.surcharge.halve = true;
    itemized.rounding.surcharge = SurchargeRounding::Total;
    itemized.surcharge.items = vec![
        SurchargeItem {
            name: "城市维护建设税".into(),
            rate: dec!(0.07),
        },
        SurchargeItem {
            name: "教育费附加".into(),
            rate: dec!(0.03),
        },
        SurchargeItem {
            name: "地方教育附加".into(),
            rate: dec!(0.02),
        },
    ];
    let mut gross = base.clone();
    gross.iit.income_base = IncomeBase::TotalCost;
    gross.rounding.mode = RoundingMode::HalfEven;
    let mut monthly = base.clone();
    monthly.vat.threshold_enabled = true;
    monthly.vat.threshold = dec!(20000);
    monthly.vat.threshold_inclusive = true;
    vec![
        ("默认", base),
        ("按次起征点500", threshold),
        ("总成本含附加税", cost_b),
        ("附加税基数=收入且个税扣附加税", net_base),
        ("3%分项减半合计舍入", itemized),
        ("个税基数=总成本+银行家舍入", gross),
        ("起征点20000含本数", monthly),
    ]
}

/// 校验单个目标：精确则正算回去完全一致；否则差额与正算一致且非零。
fn check(rev: &Reverser, target: Decimal) -> labor_fee_core::ReverseOutcome {
    let cfg = rev.config();
    let r = rev.reverse(target).unwrap();
    let fwd = forward(cfg, r.breakdown.total_cost).unwrap();
    assert_eq!(fwd, r.breakdown);
    assert_eq!(fwd.take_home - target, r.diff, "目标 {target}");
    assert_eq!(r.exact, r.diff.is_zero(), "目标 {target}");
    r
}

#[test]
fn default_config_all_targets_exact_up_to_200k() {
    // 0.01 到 20 万，每隔 0.37 元；默认口径下所有到手金额均应精确可达
    let rev = Reverser::new(&Config::default()).unwrap();
    let mut c = 1i64;
    let mut n = 0;
    while c <= 20_000_000 {
        let r = check(&rev, cents(c));
        assert!(r.exact, "到手 {} 应精确可达", cents(c));
        c += 37;
        n += 1;
    }
    assert!(n > 500_000);
}

/// 暴力表：总成本 0..=max（逐分）的到手；返回每个到手（分）对应的最小总成本。
fn brute_min_cost(cfg: &Config, max_cost_cents: i64) -> (Vec<Option<i64>>, i64) {
    let ths: Vec<i64> = (0..=max_cost_cents)
        .map(|c| to_cents(forward(cfg, cents(c)).unwrap().take_home))
        .collect();
    let max_th = *ths.iter().max().unwrap();
    let mut table = vec![None; (max_th + 1) as usize];
    for (c, &t) in ths.iter().enumerate() {
        let slot = &mut table[t as usize];
        if slot.is_none() {
            *slot = Some(c as i64);
        }
    }
    // 只有不超过“上限处到手最小值”的目标才能保证表外没有更小的解
    let safe = ths[ths.len() - 2000..].iter().copied().min().unwrap();
    (table, safe)
}

#[test]
fn brute_force_minimal_cost_all_configs() {
    // 总成本逐分遍历到 7 万（覆盖应纳税所得额 50000 档），对比反算选出的最小总成本
    for (name, cfg) in configs() {
        let rev = Reverser::new(&cfg).unwrap();
        let (table, safe) = brute_min_cost(&cfg, 7_000_000);
        let mut targets: Vec<i64> = (0..safe).step_by(97).collect();
        // 分段边界附近逐分
        for start in rev.segment_starts() {
            let t0 = to_cents(forward(&cfg, start).unwrap().take_home);
            targets.extend((t0 - 300).max(0)..=(t0 + 300).min(safe));
        }
        for t in targets {
            let r = check(&rev, cents(t));
            match table[t as usize] {
                Some(min_c) => {
                    assert!(r.exact, "[{name}] 到手 {} 可达却未找到", cents(t));
                    assert_eq!(
                        to_cents(r.breakdown.total_cost),
                        min_c,
                        "[{name}] 到手 {} 最小总成本不符",
                        cents(t)
                    );
                }
                None => assert!(!r.exact, "[{name}] 到手 {} 不可达却报精确", cents(t)),
            }
        }
    }
}

#[test]
fn boundary_targets_every_cent() {
    // 收入 800/4000、应纳税所得额 20000/50000 及起征点附近逐分
    for (name, cfg) in configs() {
        let rev = Reverser::new(&cfg).unwrap();
        for income in [
            dec!(800),
            dec!(4000),
            dec!(25000),
            dec!(62500),
            dec!(500),
            dec!(20000),
        ] {
            let center = to_cents(income * (Decimal::ONE + cfg.vat.rate));
            for c in (center - 500).max(0)..=center + 500 {
                let t = forward(&cfg, cents(c)).unwrap().take_home;
                let r = check(&rev, t);
                assert!(r.exact, "[{name}] 总成本 {} 的到手 {t} 应可达", cents(c));
                assert!(r.breakdown.total_cost <= cents(c) || !r.other_solutions.is_empty());
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn prop_roundtrip_from_cost(c in 0i64..1_000_000_000, idx in 0usize..7) {
        // 任取总成本 → 到手 → 反算，必须精确，且所选总成本不大于原总成本（取最小解）
        let (_, cfg) = configs().swap_remove(idx);
        let rev = Reverser::new(&cfg).unwrap();
        let t = forward(&cfg, cents(c)).unwrap().take_home;
        let r = check(&rev, t);
        prop_assert!(r.exact);
        prop_assert!(r.breakdown.total_cost <= cents(c));
    }

    #[test]
    fn prop_roundtrip_from_target(t in 0i64..500_000_000) {
        let rev = Reverser::new(&Config::default()).unwrap();
        let r = check(&rev, cents(t));
        prop_assert!(r.exact);
        prop_assert_eq!(forward(rev.config(), r.breakdown.total_cost).unwrap().take_home, cents(t));
    }

    #[test]
    fn prop_monotonic(a in 0i64..1_000_000_000, gap in 0i64..10_000_000, idx in 0usize..7) {
        let (_, cfg) = configs().swap_remove(idx);
        let b = a + gap;
        let fa = forward(&cfg, cents(a)).unwrap();
        let fb = forward(&cfg, cents(b)).unwrap();
        if fa.vat_exempt == fb.vat_exempt {
            // 同一增值税区间内：逐步舍入只会造成最多 1 分的局部回落，相隔 ≥1 元则严格不减
            prop_assert!(fb.take_home >= fa.take_home - dec!(0.01));
            if gap >= 100 {
                prop_assert!(fb.take_home >= fa.take_home);
            }
            prop_assert!(fb.iit.tax >= fa.iit.tax - dec!(0.01));
            prop_assert!(fb.vat >= fa.vat - dec!(0.01));
        }
        prop_assert!(fb.total_cost >= fa.total_cost);
    }
}
