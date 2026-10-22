//! 正算恒等式、反算往返一致性、单调性、按总成本正算的空位检测与大范围遍历测试。

use labor_fee_core::{
    forward, from_total_cost, CalcError, Config, CostGapReason, Reverser, RoundingMode,
    SurchargeItem, SurchargeRounding,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

/// 测试用的多种口径组合（第一个为默认口径）。
fn configs() -> Vec<(&'static str, Config)> {
    let base = Config::default();
    let mut not_equal = base.clone();
    not_equal.vat.levy_at_threshold = false;
    let mut no_threshold = base.clone();
    no_threshold.vat.threshold_enabled = false;
    // 3% 征收率、三项全开（7% / 3% / 2%）、合计后舍入、不享受减征
    let mut itemized = base.clone();
    itemized.vat.rate = dec!(0.03);
    itemized.rounding.surcharge = SurchargeRounding::Total;
    itemized.surcharge.items = three_items();
    itemized.surcharge.relief_enabled = false;
    let mut half_even = base.clone();
    half_even.rounding.mode = RoundingMode::HalfEven;
    // 不享受减征（旧版默认）
    let mut no_relief = base.clone();
    no_relief.surcharge.relief_enabled = false;
    // 3% 征收率、三项全开、减征 30%、合计后舍入
    let mut relief30 = base.clone();
    relief30.vat.rate = dec!(0.03);
    relief30.rounding.surcharge = SurchargeRounding::Total;
    relief30.surcharge.items = three_items();
    relief30.surcharge.relief_rate = dec!(0.3);
    // 仅征收教育费附加（城市维护建设税、地方教育附加选“否”）
    let mut edu_only = base.clone();
    edu_only.surcharge.items[0].enabled = false;
    edu_only.surcharge.items[2].enabled = false;
    // 1% 征收率、三项全开（7% / 3% / 2%）、减半征收、各项分别按分舍入
    let mut three = base.clone();
    three.surcharge.items = three_items();
    vec![
        ("默认", base),
        ("门槛不含本数", not_equal),
        ("无门槛", no_threshold),
        ("3%三项全开合计舍入", itemized),
        ("银行家舍入", half_even),
        ("不享受减征", no_relief),
        ("3%三项全开减征30%合计舍入", relief30),
        ("仅教育费附加", edu_only),
        ("1%三项全开减半逐项舍入", three),
    ]
}

/// 城市维护建设税 7%、教育费附加 3%、地方教育附加 2%，全部启用。
fn three_items() -> Vec<SurchargeItem> {
    vec![
        SurchargeItem::new("城市维护建设税", dec!(0.07), true),
        SurchargeItem::new("教育费附加", dec!(0.03), true),
        SurchargeItem::new("地方教育附加", dec!(0.02), true),
    ]
}

/// 非默认口径（多项附加税费）下按企业总支出逐分测算：可达则与正算一致，
/// 否则不可达原因正确（起征点区间恰为 [起征点, 起征点对应总支出 − 0.01]），两侧均可达。
fn check_from_cost_sweep(cfg: &Config, upto_cents: i64) {
    let thr = cfg.vat.threshold;
    let thr_cost = forward(cfg, thr).unwrap().total_cost;
    let (mut threshold_gaps, mut rounding_gaps) = (0i64, 0);
    for c in 0..=upto_cents {
        let cost = cents(c);
        match from_total_cost(cfg, cost) {
            Ok(b) => {
                assert_eq!(b.total_cost, cost);
                assert_eq!(forward(cfg, b.net_income).unwrap(), b);
            }
            Err(CalcError::CostUnreachable(g)) => {
                assert!(g.lower.total_cost < cost && cost < g.upper.total_cost);
                assert_eq!(g.upper.net_income - g.lower.net_income, dec!(0.01));
                assert_eq!(
                    forward(cfg, g.lower.net_income).unwrap().total_cost,
                    g.lower.total_cost
                );
                assert_eq!(
                    forward(cfg, g.upper.net_income).unwrap().total_cost,
                    g.upper.total_cost
                );
                match g.reason {
                    CostGapReason::Threshold => {
                        threshold_gaps += 1;
                        assert!(cost >= thr && cost < thr_cost, "总支出 {cost}");
                        assert_eq!(g.upper.net_income, thr);
                    }
                    CostGapReason::Rounding => {
                        rounding_gaps += 1;
                        assert!(cost >= thr_cost, "总支出 {cost}");
                    }
                }
            }
            Err(e) => panic!("{e}"),
        }
    }
    let expect = ((thr_cost - thr) * dec!(100))
        .trunc()
        .to_string()
        .parse::<i64>()
        .unwrap();
    assert_eq!(threshold_gaps, expect);
    assert!(rounding_gaps > 0);
}

#[test]
fn from_total_cost_three_items_every_cent() {
    // 7%/3%/2% 三项全开：1% 征收率逐项舍入、3% 征收率合计舍入
    for (name, cfg) in configs() {
        if name.contains("三项") || name == "不享受减征" {
            check_from_cost_sweep(&cfg, 3_000_000);
        }
    }
}

#[test]
fn three_items_threshold_values() {
    let (_, cfg) = configs().pop().unwrap();
    // 1000 → 增值税 10.00，附加（减半）0.35 + 0.15 + 0.10 = 0.60，企业总支出 1010.60
    let b = forward(&cfg, dec!(1000)).unwrap();
    assert_eq!(b.surcharge_total, dec!(0.60));
    assert_eq!(b.surcharge_before_relief, dec!(1.20));
    assert_eq!(b.total_cost, dec!(1010.60));
    // 不可达区间随配置变化：减半 [1000.00, 1010.59]；不享受减征 [1000.00, 1011.19]
    for (relief, upper) in [(true, dec!(1010.60)), (false, dec!(1011.20))] {
        let mut c = cfg.clone();
        c.surcharge.relief_enabled = relief;
        match from_total_cost(&c, upper - dec!(0.01)) {
            Err(CalcError::CostUnreachable(g)) => {
                assert_eq!(g.reason, CostGapReason::Threshold);
                assert_eq!(
                    (g.lower.total_cost, g.upper.total_cost),
                    (dec!(999.99), upper)
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            from_total_cost(&c, upper).unwrap().net_income,
            dec!(1000.00)
        );
    }
}

/// 校验反算：精确、正算一致、取最小不含税收入。
fn check_exact_min(rev: &Reverser, target: Decimal) -> labor_fee_core::ReverseOutcome {
    let cfg = rev.config();
    let r = rev.reverse(target).unwrap();
    assert!(r.exact, "到手 {target} 应精确可达");
    let fwd = forward(cfg, r.breakdown.net_income).unwrap();
    assert_eq!(fwd, r.breakdown, "反算结果再正算应完全一致");
    assert_eq!(fwd.take_home, target);
    let n = r.breakdown.net_income;
    if n > Decimal::ZERO {
        // 到手单调不减（见 monotone_every_cent），前一分到手更小即证明取到最小收入
        let prev = forward(cfg, n - dec!(0.01)).unwrap().take_home;
        assert!(prev < target, "到手 {target}：{n} 不是最小不含税收入");
    }
    r
}

#[test]
fn identities_every_cent_to_200k() {
    // 不含税收入 0 ~ 200000 逐分：总成本 = 三项之和，到手 = 收入 − 个税，门槛按收入判断
    let cfg = Config::default();
    for c in 0..=20_000_000i64 {
        let i = cents(c);
        let b = forward(&cfg, i).unwrap();
        assert_eq!(b.net_income, i);
        assert_eq!(
            b.total_cost,
            b.net_income + b.vat + b.surcharge_total,
            "收入 {i}"
        );
        assert_eq!(b.take_home, b.net_income - b.iit.tax, "收入 {i}");
        assert_eq!(b.vat_exempt, c < 100_000, "收入 {i}");
        if !b.vat_exempt {
            assert_eq!(
                b.vat,
                (i * dec!(0.01)).round_dp_with_strategy(
                    2,
                    rust_decimal::RoundingStrategy::MidpointAwayFromZero
                )
            );
        }
    }
}

#[test]
fn monotone_every_cent() {
    // 各口径：到手随不含税收入单调不减、每分最多增加 0.01（故任意到手可达、无解区间不存在）；
    // 总成本严格递增
    for (name, cfg) in configs() {
        let mut prev = forward(&cfg, dec!(0)).unwrap();
        let mut plateaus = 0u64;
        for c in 1..=30_000_000i64 {
            let b = forward(&cfg, cents(c)).unwrap();
            let d = b.take_home - prev.take_home;
            assert!(
                d == dec!(0) || d == dec!(0.01),
                "[{name}] 收入 {} 到手变化 {d}",
                cents(c)
            );
            if d.is_zero() {
                plateaus += 1;
            }
            assert!(b.total_cost > prev.total_cost, "[{name}] 收入 {}", cents(c));
            prev = b;
            if name != "默认" && c >= 7_000_000 {
                break;
            }
        }
        assert!(plateaus > 0, "[{name}] 应存在多个收入对应同一到手的平台");
    }
}

#[test]
fn reverse_roundtrip_to_200k() {
    // 到手 0.01 ~ 200000，每隔 0.37 元
    let rev = Reverser::new(&Config::default()).unwrap();
    let mut c = 1i64;
    while c <= 20_000_000 {
        check_exact_min(&rev, cents(c));
        c += 37;
    }
}

#[test]
fn reverse_every_cent_near_boundaries() {
    // 1000 门槛、个税 800/4000/20000/50000 分段附近逐分
    for (_, cfg) in configs() {
        let rev = Reverser::new(&cfg).unwrap();
        let mut centers: Vec<Decimal> = rev.segment_starts();
        centers.extend([dec!(800), dec!(1000), dec!(4000), dec!(25000), dec!(62500)]);
        for center in centers {
            let t0 = (forward(&cfg, center).unwrap().take_home * dec!(100))
                .trunc()
                .to_string()
                .parse::<i64>()
                .unwrap();
            for t in (t0 - 1000).max(0)..=t0 + 1000 {
                let r = check_exact_min(&rev, cents(t));
                assert!(r.other_solutions.is_empty());
            }
        }
    }
}

#[test]
fn threshold_table() {
    let rev = Reverser::new(&Config::default()).unwrap();
    for (t, net, cost) in [
        (dec!(959.98), dec!(999.97), dec!(999.97)),
        (dec!(959.99), dec!(999.99), dec!(999.99)),
        (dec!(960.00), dec!(1000.00), dec!(1010.30)),
        (dec!(960.01), dec!(1000.01), dec!(1010.31)),
    ] {
        let r = check_exact_min(&rev, t);
        assert_eq!(r.breakdown.net_income, net);
        assert_eq!(r.breakdown.total_cost, cost);
    }
}

#[test]
fn from_total_cost_every_cent_to_70k() {
    // 总成本 0 ~ 70000 逐分：可达则与正算一致，否则报空位（原因、两侧最近可达总成本）
    let cfg = Config::default();
    let (mut threshold_gaps, mut rounding_gaps) = (0, 0);
    for c in 0..=7_000_000i64 {
        let cost = cents(c);
        match from_total_cost(&cfg, cost) {
            Ok(b) => {
                assert_eq!(b.total_cost, cost);
                assert_eq!(forward(&cfg, b.net_income).unwrap(), b);
            }
            Err(CalcError::CostUnreachable(g)) => {
                assert!(g.lower.total_cost < cost && cost < g.upper.total_cost);
                assert_eq!(g.upper.net_income - g.lower.net_income, dec!(0.01));
                assert_eq!(
                    forward(&cfg, g.lower.net_income).unwrap().total_cost,
                    g.lower.total_cost
                );
                assert_eq!(
                    forward(&cfg, g.upper.net_income).unwrap().total_cost,
                    g.upper.total_cost
                );
                match g.reason {
                    CostGapReason::Threshold => {
                        threshold_gaps += 1;
                        assert!((100_000..=101_029).contains(&c), "总成本 {cost}");
                    }
                    CostGapReason::Rounding => rounding_gaps += 1,
                }
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert_eq!(
        threshold_gaps, 1030,
        "门槛空白区间 1000.00 ~ 1010.29 共 1030 个分"
    );
    assert!(rounding_gaps > 0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    #[test]
    fn prop_roundtrip_from_net(n in 0i64..1_000_000_000, idx in 0usize..9) {
        // 任取不含税收入 → 到手 → 反算：精确，且不大于原收入（取最小）
        let (_, cfg) = configs().swap_remove(idx);
        let rev = Reverser::new(&cfg).unwrap();
        let b = forward(&cfg, cents(n)).unwrap();
        let r = check_exact_min(&rev, b.take_home);
        prop_assert!(r.breakdown.net_income <= b.net_income);
    }

    #[test]
    fn prop_roundtrip_from_target(t in 0i64..500_000_000, idx in 0usize..9) {
        let (_, cfg) = configs().swap_remove(idx);
        let rev = Reverser::new(&cfg).unwrap();
        check_exact_min(&rev, cents(t));
    }

    #[test]
    fn prop_monotonic(a in 0i64..1_000_000_000, gap in 0i64..10_000_000, idx in 0usize..9) {
        let (_, cfg) = configs().swap_remove(idx);
        let fa = forward(&cfg, cents(a)).unwrap();
        let fb = forward(&cfg, cents(a + gap)).unwrap();
        prop_assert!(fb.take_home >= fa.take_home);
        prop_assert!(fb.iit.tax >= fa.iit.tax);
        prop_assert!(fb.vat >= fa.vat);
        if gap > 0 {
            prop_assert!(fb.total_cost > fa.total_cost);
        }
    }

    #[test]
    fn prop_from_total_cost(n in 0i64..1_000_000_000, delta in 0i64..3, idx in 0usize..9) {
        // 可达总成本反推回同一收入；不可达总成本报空位且两侧可达
        let (_, cfg) = configs().swap_remove(idx);
        let b = forward(&cfg, cents(n)).unwrap();
        prop_assert_eq!(from_total_cost(&cfg, b.total_cost).unwrap(), b.clone());
        let cost = b.total_cost + cents(delta);
        match from_total_cost(&cfg, cost) {
            Ok(x) => prop_assert_eq!(x.total_cost, cost),
            Err(CalcError::CostUnreachable(g)) => {
                prop_assert!(g.lower.total_cost < cost && cost < g.upper.total_cost);
            }
            Err(e) => prop_assert!(false, "{}", e),
        }
    }

    #[test]
    fn prop_relief_any_config(
        n in 0i64..100_000_000,
        relief in 0i64..=100,
        mask in 1u8..8,
        enabled: bool,
        total in any::<bool>(),
    ) {
        // 任意减征比例 × 征收项组合 × 舍入方式：减征前后关系、各项合计、反算可逆
        let mut cfg = Config::default();
        cfg.surcharge.items = three_items();
        for (i, it) in cfg.surcharge.items.iter_mut().enumerate() {
            it.enabled = mask & (1 << i) != 0;
        }
        cfg.surcharge.relief_enabled = enabled;
        cfg.surcharge.relief_rate = Decimal::new(relief, 2);
        if total {
            cfg.rounding.surcharge = SurchargeRounding::Total;
        }
        let b = forward(&cfg, cents(n)).unwrap();
        prop_assert!(b.surcharge_total <= b.surcharge_before_relief);
        prop_assert_eq!(b.surcharge_before_relief - b.surcharge_relief, b.surcharge_total);
        let sum: Decimal = b.surcharges.iter().map(|l| l.amount).sum();
        prop_assert_eq!(sum, b.surcharge_total);
        for l in &b.surcharges {
            prop_assert_eq!(l.amount_before_relief - l.relief_amount, l.amount);
        }
        if !enabled || relief == 0 {
            prop_assert_eq!(b.surcharge_relief, Decimal::ZERO);
        }
        prop_assert_eq!(from_total_cost(&cfg, b.total_cost).unwrap(), b.clone());
        let rev = Reverser::new(&cfg).unwrap();
        check_exact_min(&rev, b.take_home);
    }
}
