use labor_fee_core::{forward, Config, Prefer, Reverser};
use rust_decimal_macros::dec;

#[test]
fn maximum_is_taken_across_all_segments() {
    let mut cfg = Config::default();
    cfg.iit.percent_deduction_rate = dec!(0.1);
    cfg.reverse.max_net_income = dec!(4000.01);
    let result = Reverser::new(&cfg).unwrap().reverse(dec!(3350)).unwrap();
    assert!(result.exact);
    assert_eq!(result.breakdown.net_income, dec!(3987.50));
}

#[test]
fn full_plateau_is_independent_of_scan_window() {
    let mut cfg = Config::default();
    cfg.iit.brackets[0].rate = dec!(0.99);
    cfg.reverse.prefer = Prefer::MaxCost;
    cfg.reverse.search_window = 1;
    let result = Reverser::new(&cfg).unwrap().reverse(dec!(800)).unwrap();
    assert_eq!(result.breakdown.net_income, dec!(800.50));
    assert_eq!(
        result.exact_net_income_range,
        Some((dec!(800), dec!(800.50)))
    );
    assert_eq!(forward(&cfg, dec!(800.51)).unwrap().take_home, dec!(800.01));
}

use labor_fee_core::{Bracket, CalcError, Decimal, RoundingMode};
use proptest::prelude::*;

// Exhaust the entire small input grid; the oracle deliberately knows nothing
// about the production segmentation or binary searches.
fn check_grid(cfg: &Config) {
    let scale = cfg.rounding.scale;
    let values: Vec<_> = (0..=120)
        .map(|t| forward(cfg, Decimal::new(t, scale)).unwrap().take_home)
        .collect();
    let maximum = *values.iter().max().unwrap();
    let rev = Reverser::new(cfg).unwrap();
    for target_tick in 0..=122 {
        let target = Decimal::new(target_tick, scale);
        let got = rev.reverse(target);
        if target > maximum {
            assert!(
                matches!(got, Err(CalcError::TargetTooLarge { max_take_home, .. }) if max_take_home == maximum)
            );
            continue;
        }
        let got = got.unwrap();
        let exact: Vec<_> = values
            .iter()
            .enumerate()
            .filter(|(_, v)| **v == target)
            .map(|(t, _)| t)
            .collect();
        assert_eq!(got.exact, !exact.is_empty());
        if exact.is_empty() {
            let best = values
                .iter()
                .enumerate()
                .min_by_key(|(t, v)| ((**v - target).abs(), *t))
                .unwrap()
                .0;
            assert_eq!(got.breakdown.net_income, Decimal::new(best as i64, scale));
            assert_eq!(got.diff, values[best] - target);
            assert_eq!(got.exact_net_income_range, None);
        } else {
            let pick = match cfg.reverse.prefer {
                Prefer::MinCost => exact[0],
                Prefer::MaxCost => *exact.last().unwrap(),
            };
            assert_eq!(got.breakdown.net_income, Decimal::new(pick as i64, scale));
            let mut a = pick;
            let mut b = pick;
            while a > 0 && values[a - 1] == target {
                a -= 1;
            }
            while b < 120 && values[b + 1] == target {
                b += 1;
            }
            assert_eq!(
                got.exact_net_income_range,
                Some((Decimal::new(a as i64, scale), Decimal::new(b as i64, scale)))
            );
            let mut representatives = Vec::new();
            let mut t = 0;
            while t <= 120 {
                if values[t] != target {
                    t += 1;
                    continue;
                }
                let start = t;
                while t < 120 && values[t + 1] == target {
                    t += 1;
                }
                if start != a {
                    representatives.push(Decimal::new(
                        match cfg.reverse.prefer {
                            Prefer::MinCost => start,
                            Prefer::MaxCost => t,
                        } as i64,
                        scale,
                    ));
                }
                t += 1;
            }
            assert_eq!(got.other_solutions, representatives);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(80))]
    #[test]
    fn custom_config_matches_exhaustive_oracle(
        scale in 0u32..=6, mode in 0usize..4, max_cost in any::<bool>(),
        deduction in 0i64..100, limit in 1i64..120, fixed in 0i64..80,
        rate1 in 0i64..100, rate2 in 0i64..100, quick in 0i64..40,
        threshold in 0i64..120, vat in 0i64..100, relief in 0i64..=100,
    ) {
        let mut cfg = Config::default();
        cfg.rounding.scale = scale;
        cfg.rounding.mode = [RoundingMode::HalfUp, RoundingMode::HalfEven, RoundingMode::Down, RoundingMode::Up][mode];
        cfg.reverse.max_net_income = Decimal::new(120, scale);
        cfg.reverse.search_window = 1;
        cfg.reverse.prefer = if max_cost { Prefer::MaxCost } else { Prefer::MinCost };
        cfg.iit.fixed_deduction_limit = Decimal::new(limit, scale);
        cfg.iit.fixed_deduction = Decimal::new(fixed, scale + 1);
        cfg.iit.percent_deduction_rate = Decimal::new(deduction, 2);
        cfg.iit.brackets = vec![Bracket { upper: Some(Decimal::new(40, scale)), rate: Decimal::new(rate1, 2), quick_deduction: Decimal::ZERO }, Bracket { upper: None, rate: Decimal::new(rate2, 2), quick_deduction: Decimal::new(quick, scale + 1) }];
        cfg.vat.threshold = Decimal::new(threshold, scale);
        cfg.vat.rate = Decimal::new(vat, 2);
        cfg.surcharge.relief_rate = Decimal::new(relief, 2);
        check_grid(&cfg);
    }
}

#[test]
fn fractional_deduction_uses_actual_bracket_boundary() {
    let mut cfg = Config::default();
    cfg.rounding.scale = 0;
    cfg.reverse.max_net_income = dec!(120);
    cfg.iit.fixed_deduction_limit = dec!(45);
    cfg.iit.fixed_deduction = dec!(4.6);
    cfg.iit.percent_deduction_rate = Decimal::ZERO;
    cfg.iit.brackets = vec![
        Bracket {
            upper: Some(dec!(40)),
            rate: Decimal::ZERO,
            quick_deduction: Decimal::ZERO,
        },
        Bracket {
            upper: None,
            rate: dec!(0.1),
            quick_deduction: Decimal::ZERO,
        },
    ];
    check_grid(&cfg);
}

#[test]
fn rounding_modes_and_scales_match_full_grid() {
    for scale in 0..=6 {
        for mode in [
            RoundingMode::HalfUp,
            RoundingMode::HalfEven,
            RoundingMode::Down,
            RoundingMode::Up,
        ] {
            for prefer in [Prefer::MinCost, Prefer::MaxCost] {
                let mut cfg = Config::default();
                cfg.rounding.scale = scale;
                cfg.rounding.mode = mode;
                cfg.reverse.prefer = prefer;
                cfg.reverse.max_net_income = Decimal::new(120, scale);
                cfg.iit.fixed_deduction_limit = Decimal::new(20, scale);
                cfg.iit.fixed_deduction = Decimal::new(15, scale + 1);
                cfg.iit.percent_deduction_rate = dec!(0.99);
                cfg.iit.brackets = vec![Bracket {
                    upper: None,
                    rate: dec!(0.99),
                    quick_deduction: Decimal::ZERO,
                }];
                check_grid(&cfg);
            }
        }
    }
}

#[test]
fn maximum_preference_merges_plateau_across_three_segments() {
    let mut cfg = Config::default();
    cfg.rounding.scale = 0;
    cfg.reverse.max_net_income = dec!(120);
    cfg.reverse.prefer = Prefer::MaxCost;
    cfg.reverse.search_window = 1;
    cfg.iit.fixed_deduction = Decimal::ZERO;
    cfg.iit.fixed_deduction_limit = dec!(40);
    cfg.iit.percent_deduction_rate = Decimal::ZERO;
    cfg.iit.brackets = vec![Bracket {
        upper: None,
        rate: dec!(0.99),
        quick_deduction: Decimal::ZERO,
    }];
    cfg.vat.threshold = dec!(20);
    let result = Reverser::new(&cfg).unwrap().reverse(Decimal::ZERO).unwrap();
    assert_eq!(result.breakdown.net_income, dec!(50));
    assert_eq!(result.exact_net_income_range, Some((dec!(0), dec!(50))));
    assert!(result.other_solutions.is_empty());
}
