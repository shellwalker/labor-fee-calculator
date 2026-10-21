//! 反算 ↔ 正算 往返一致性、单调性与大范围遍历测试。

use labor_fee_core::{
    forward, Config, CostBasis, IncomeBase, Reverser, RoundingMode, SurchargeItem,
    SurchargeRounding, ThresholdBasis, ThresholdGap,
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

/// 阶段一口径：价税合计、无门槛、到手扣附加税。
fn legacy() -> Config {
    let mut c = Config::default();
    c.cost.basis = CostBasis::VatInclusive;
    c.vat.threshold_enabled = false;
    c.vat.threshold_basis = ThresholdBasis::TotalCost;
    c.take_home.deduct_surcharge = true;
    c
}

/// 测试用的多种口径组合（第一个为默认口径）。
fn configs() -> Vec<(&'static str, Config)> {
    let base = Config::default();
    let mut not_equal = base.clone();
    not_equal.vat.levy_at_threshold = false;
    let mut gap_exempt = base.clone();
    gap_exempt.vat.threshold_gap = ThresholdGap::Exempt;
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
    let mut worker_pays = base.clone();
    worker_pays.take_home.deduct_surcharge = true;
    worker_pays.iit.deduct_surcharge = true;
    worker_pays.vat.threshold_basis = ThresholdBasis::TotalCost;
    let legacy_plain = legacy();
    let mut legacy_cost_thr = legacy();
    legacy_cost_thr.vat.threshold_enabled = true;
    legacy_cost_thr.vat.threshold = dec!(500);
    let mut legacy_net_thr = legacy();
    legacy_net_thr.vat.threshold_enabled = true;
    legacy_net_thr.vat.threshold = dec!(1000);
    legacy_net_thr.vat.threshold_basis = ThresholdBasis::NetRevenue;
    vec![
        ("默认（企业承担税费，门槛1000含本数）", base),
        ("门槛不含本数", not_equal),
        ("跳变段按不征收", gap_exempt),
        ("3%分项减半合计舍入", itemized),
        ("个税基数=总成本+银行家舍入", gross),
        ("到手扣附加税+门槛按总成本", worker_pays),
        ("价税合计无门槛", legacy_plain),
        ("价税合计+按总成本起征点500", legacy_cost_thr),
        ("价税合计+按收入门槛1000", legacy_net_thr),
    ]
}

/// 校验单个目标：精确则正算回去完全一致；否则差额与正算一致且非零。
fn check(rev: &Reverser, target: Decimal) -> labor_fee_core::ReverseOutcome {
    let cfg = rev.config();
    let r = rev.reverse(target).unwrap();
    let fwd = forward(cfg, r.breakdown.total_cost).unwrap();
    assert_eq!(fwd, r.breakdown, "反算结果再正算应完全一致");
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

#[test]
fn split_identity_every_cent_to_70k() {
    // 总成本 0 ~ 70000 逐分正算：三项之和恒等于总成本
    for (name, cfg) in configs().into_iter().take(3) {
        for c in 0..=7_000_000i64 {
            let b = forward(&cfg, cents(c)).unwrap();
            assert_eq!(
                b.net_revenue + b.vat + b.surcharge_total,
                cents(c),
                "[{name}] 总成本 {}",
                cents(c)
            );
            assert_eq!(b.total_cost, cents(c));
        }
    }
}

#[test]
fn default_threshold_every_cent() {
    let cfg = Config::default();
    let rev = Reverser::new(&cfg).unwrap();
    // 到手 900.00 ~ 1100.00 逐分：全部精确可达；有两解时取最小总成本（不征收一侧）
    for t in 90_000..=110_000 {
        let r = check(&rev, cents(t));
        assert!(r.exact, "到手 {} 应可达", cents(t));
        let b = &r.breakdown;
        if t <= 95_999 {
            assert!(b.vat_exempt, "到手 {} 应取不征收一侧", cents(t));
        }
        for o in &r.other_solutions {
            assert!(*o > b.total_cost);
            assert_eq!(forward(&cfg, *o).unwrap().take_home, cents(t));
        }
    }
    // 总成本 990.00 ~ 1030.00 逐分正算：跳变段 [1000.00, 1010.29] 按征收、收入 < 1000
    for c in 99_000..=103_000 {
        let b = forward(&cfg, cents(c)).unwrap();
        assert_eq!(b.net_revenue + b.vat + b.surcharge_total, cents(c));
        assert_eq!(b.vat_exempt, c < 100_000, "总成本 {}", cents(c));
        if (100_000..=101_029).contains(&c) {
            assert!(b.net_revenue < dec!(1000), "总成本 {}", cents(c));
        } else if c >= 101_030 {
            assert!(b.net_revenue >= dec!(1000), "总成本 {}", cents(c));
        }
        let r = check(&rev, b.take_home);
        assert!(r.exact);
        assert!(r.breakdown.total_cost <= b.total_cost);
    }
}

/// 暴力表：总成本 0..=max（逐分）的到手；
/// 返回每个到手（分）对应的最小总成本，以及可安全比较的到手上限。
fn brute_min_cost(cfg: &Config, max_cost_cents: i64) -> (Vec<Option<i64>>, i64) {
    let ths: Vec<(i64, i64)> = (0..=max_cost_cents)
        .map(|c| (c, to_cents(forward(cfg, cents(c)).unwrap().take_home)))
        .collect();
    let max_th = ths.iter().map(|x| x.1).max().unwrap();
    let mut table = vec![None; (max_th + 1) as usize];
    for &(c, t) in &ths {
        let slot = &mut table[t as usize];
        if slot.is_none() {
            *slot = Some(c);
        }
    }
    // 只有不超过“上限附近到手最小值”的目标才能保证表外没有更小的解
    let safe = ths[ths.len() - 2000..].iter().map(|x| x.1).min().unwrap();
    (table, safe)
}

#[test]
fn brute_force_minimal_cost_all_configs() {
    // 总成本逐分遍历（默认口径到 7 万，覆盖应纳税所得额 50000 档；其它口径到 3 万，覆盖 20000 档），
    // 对比反算选出的最小总成本
    for (i, (name, cfg)) in configs().into_iter().enumerate() {
        let rev = Reverser::new(&cfg).unwrap();
        let max = if i == 0 { 7_000_000 } else { 3_000_000 };
        let (table, safe) = brute_min_cost(&cfg, max);
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
                None => {
                    assert!(!r.exact, "[{name}] 到手 {} 不可达却报精确", cents(t));
                    // 默认口径下任意到手都必须可达
                    assert!(i != 0, "[默认] 到手 {} 不可达", cents(t));
                }
            }
        }
    }
}

#[test]
fn boundary_costs_every_cent() {
    // 收入 800/1000/4000、应纳税所得额 20000/50000、征收门槛附近：逐分正算 → 反算
    for (name, cfg) in configs() {
        let rev = Reverser::new(&cfg).unwrap();
        let mut centers: Vec<i64> = rev.segment_starts().into_iter().map(to_cents).collect();
        centers.extend([
            80_000, 100_000, 101_030, 400_000, 2_500_000, 6_250_000, 50_000,
        ]);
        for center in centers {
            for c in (center - 500).max(0)..=center + 500 {
                let b = forward(&cfg, cents(c)).unwrap();
                let r = check(&rev, b.take_home);
                assert!(r.exact, "[{name}] 总成本 {} 的到手应可达", cents(c));
                assert!(
                    r.breakdown.total_cost <= b.total_cost || !r.other_solutions.is_empty(),
                    "[{name}] 总成本 {}",
                    cents(c)
                );
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn prop_roundtrip_from_cost(c in 0i64..1_000_000_000, idx in 0usize..9) {
        // 任取总成本 → 到手 → 反算，必须精确，且所选总成本不大于正算采用的有效总成本（取最小解）
        let (_, cfg) = configs().swap_remove(idx);
        let rev = Reverser::new(&cfg).unwrap();
        let b = forward(&cfg, cents(c)).unwrap();
        prop_assert_eq!(b.total_cost, cents(c));
        let r = check(&rev, b.take_home);
        prop_assert!(r.exact);
        prop_assert!(r.breakdown.total_cost <= b.total_cost);
    }

    #[test]
    fn prop_roundtrip_from_target(t in 0i64..500_000_000) {
        let rev = Reverser::new(&Config::default()).unwrap();
        let r = check(&rev, cents(t));
        prop_assert!(r.exact);
        prop_assert_eq!(forward(rev.config(), r.breakdown.total_cost).unwrap().take_home, cents(t));
    }

    #[test]
    fn prop_monotonic(a in 0i64..1_000_000_000, gap in 0i64..10_000_000, idx in 0usize..9) {
        let (_, cfg) = configs().swap_remove(idx);
        let b = a + gap;
        let fa = forward(&cfg, cents(a)).unwrap();
        let fb = forward(&cfg, cents(b)).unwrap();
        prop_assert!(fb.total_cost >= fa.total_cost);
        if fa.vat_exempt == fb.vat_exempt {
            // 同一征收状态内：逐步舍入只会造成最多 1 分的局部回落，相隔 ≥1 元则严格不减
            prop_assert!(fb.take_home >= fa.take_home - dec!(0.01));
            if gap >= 100 {
                prop_assert!(fb.take_home >= fa.take_home);
            }
            prop_assert!(fb.iit.tax >= fa.iit.tax - dec!(0.01));
            prop_assert!(fb.vat >= fa.vat - dec!(0.01));
        }
    }

    #[test]
    fn prop_default_split(n in 0i64..1_000_000_000, idx in 0usize..9) {
        // 任意总成本完整拆分；默认口径到手 = 不含税收入 − 个税，总成本 ≥ 1000 即征收
        let (_, cfg) = configs().swap_remove(idx);
        let b = forward(&cfg, cents(n)).unwrap();
        let sum = if cfg.cost.basis == CostBasis::VatInclusive {
            b.net_revenue + b.vat
        } else {
            b.net_revenue + b.vat + b.surcharge_total
        };
        prop_assert_eq!(sum, cents(n));
        if idx == 0 {
            prop_assert_eq!(b.take_home, b.net_revenue - b.iit.tax);
            prop_assert_eq!(b.vat_exempt, n < 100_000);
        }
    }
}
