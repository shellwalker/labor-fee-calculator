//! 反算：实际到手 → 企业总成本。
//!
//! 正算对任意总成本（精确到分）都给出完整拆分，因此反算在总成本的分位网格上搜索。
//! 实际到手关于总成本是分段函数：个税各档（800/4000/20000/50000）处连续但斜率变化；
//! 征收门槛处到手向下跳变（开始征收增值税和附加税）；逐步舍入到分又会造成局部 ±1 分的抖动和
//! “多个总成本对应同一到手”的平台。算法：
//! 1. 按分段键（是否征收 / 减除费用方式 / 预扣率档次）把总成本划分为若干近似单调的区间；
//! 2. 在每个区间内对“分”做二分，定位到手首次 ≥ 目标的位置；
//! 3. 以该位置为中心按 0.01 上下逐分扫描（窗口见 `reverse.search_window`），
//!    正算校准：收集所有正算到手与目标完全相等的总成本；
//! 4. 有精确解则按 `reverse.prefer` 取最小（默认）或最大总成本，其它区间的精确解列入 `other_solutions`；
//!    无精确解则取正算到手与目标差额绝对值最小者（同差额取较小总成本），标记“不可精确达到”并给出差额。

use crate::config::{Config, Prefer};
use crate::error::CalcError;
use crate::forward::{eval_cost, from_ticks, to_ticks, validate_amount, Breakdown};
use rust_decimal::Decimal;
use serde::Serialize;

/// 反算结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReverseOutcome {
    /// 目标实际到手。
    pub target: Decimal,
    /// 选定的企业总成本对应的完整正算结果。
    pub breakdown: Breakdown,
    /// 正算到手是否与目标完全一致。
    pub exact: bool,
    /// 差额 = 正算到手 − 目标（精确时为 0）。
    pub diff: Decimal,
    /// 选定解所在区间内，能精确得到目标的最小与最大总成本（仅精确时有值）。
    pub exact_cost_range: Option<(Decimal, Decimal)>,
    /// 其它区间（如征收门槛另一侧）也能精确得到目标的总成本。
    pub other_solutions: Vec<Decimal>,
}

impl ReverseOutcome {
    /// 中文状态说明。
    pub fn status_text(&self) -> String {
        if self.exact {
            if self.other_solutions.is_empty() {
                "精确".to_string()
            } else {
                format!("精确（存在多解，另有总成本 {:?}）", self.other_solutions)
            }
        } else {
            format!("不可精确达到，差额 {}", self.diff)
        }
    }
}

/// 在企业总成本的分位网格上搜索（每个分都是有效总成本）。
struct Ctx<'a> {
    cfg: &'a Config,
}

impl Ctx<'_> {
    fn dec(&self, ticks: i64) -> Decimal {
        from_ticks(self.cfg, ticks)
    }
    fn eval(&self, ticks: i64) -> Breakdown {
        eval_cost(self.cfg, self.dec(ticks))
    }
    fn th(&self, ticks: i64) -> Decimal {
        self.eval(ticks).take_home
    }
    /// 分段键：(是否征收增值税, 是否按比例减除费用, 预扣率档次)。随总成本按字典序（近似）单调不减。
    fn key(&self, ticks: i64) -> (bool, bool, usize) {
        let b = self.eval(ticks);
        let iit = &self.cfg.iit;
        let pct = b.iit.income > iit.fixed_deduction_limit;
        let idx = iit
            .brackets
            .iter()
            .position(|br| br.upper.is_none_or(|u| b.iit.taxable_income <= u))
            .unwrap_or(iit.brackets.len());
        (!b.vat_exempt, pct, idx)
    }
}

struct RegionResult {
    exact_min: Option<i64>,
    exact_max: Option<i64>,
    nearest: (i64, Decimal),
}

fn search_region(ctx: &Ctx, lo: i64, hi: i64, target: Decimal, window: i64) -> RegionResult {
    // 二分：区间内到手首次 ≥ 目标的位置（区间内近似单调）
    let pos = if ctx.th(hi) < target {
        hi
    } else if ctx.th(lo) >= target {
        lo
    } else {
        let (mut l, mut h) = (lo, hi); // 不变式：th(l) < target ≤ th(h)
        while h - l > 1 {
            let m = l + (h - l) / 2;
            if ctx.th(m) >= target {
                h = m;
            } else {
                l = m;
            }
        }
        h
    };
    // 按 0.01 上下逐分校准扫描
    let a = (pos - window).max(lo);
    let b = (pos + window).min(hi);
    let mut exact_min = None;
    let mut exact_max = None;
    let mut nearest: Option<(i64, Decimal)> = None;
    for t in a..=b {
        let d = ctx.th(t) - target;
        if d.is_zero() {
            exact_min.get_or_insert(t);
            exact_max = Some(t);
        }
        if nearest.is_none_or(|(_, nd)| d.abs() < nd.abs()) {
            nearest = Some((t, d));
        }
    }
    RegionResult {
        exact_min,
        exact_max,
        nearest: nearest.expect("窗口非空"),
    }
}

/// 可复用的反算器：按配置预先求出分段区间，批量反算时避免重复计算。
#[derive(Debug, Clone)]
pub struct Reverser {
    cfg: Config,
    /// 分段区间（企业总成本，以最小金额单位计的闭区间）。
    regions: Vec<(i64, i64)>,
    max_ticks: i64,
}

impl Reverser {
    /// 校验配置并计算分段区间。
    pub fn new(cfg: &Config) -> Result<Self, CalcError> {
        cfg.validate()?;
        let ctx = Ctx { cfg };
        let max_ticks = to_ticks(cfg, cfg.round(cfg.reverse.max_cost))?;

        let valid = [(0i64, max_ticks)];
        // 按分段键切分区间
        let mut regions = Vec::new();
        for (vlo, vhi) in valid {
            let mut lo = vlo;
            loop {
                let k = ctx.key(lo);
                if ctx.key(vhi) <= k {
                    regions.push((lo, vhi));
                    break;
                }
                let (mut l, mut h) = (lo, vhi);
                while h - l > 1 {
                    let m = l + (h - l) / 2;
                    if ctx.key(m) > k {
                        h = m;
                    } else {
                        l = m;
                    }
                }
                regions.push((lo, h - 1));
                lo = h;
            }
        }
        Ok(Reverser {
            cfg: cfg.clone(),
            regions,
            max_ticks,
        })
    }

    /// 配置。
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// 各分段区间起点对应的企业总成本，用于诊断与测试。
    pub fn segment_starts(&self) -> Vec<Decimal> {
        let ctx = Ctx { cfg: &self.cfg };
        self.regions
            .iter()
            .map(|r| ctx.eval(r.0).total_cost)
            .collect()
    }

    /// 反算：目标实际到手 → 企业总成本及全部明细。
    pub fn reverse(&self, take_home: Decimal) -> Result<ReverseOutcome, CalcError> {
        let cfg = &self.cfg;
        let target = validate_amount(cfg, take_home)?;
        let ctx = Ctx { cfg };
        let last = self.regions.last().map(|r| r.1).unwrap_or(self.max_ticks);
        let max_take_home = ctx.th(last);
        if target > max_take_home {
            return Err(CalcError::TargetTooLarge {
                target,
                max_cost: ctx.eval(last).total_cost,
                max_take_home,
            });
        }
        let window = i64::from(cfg.reverse.search_window.max(1));
        let results: Vec<RegionResult> = self
            .regions
            .iter()
            .map(|&(lo, hi)| search_region(&ctx, lo, hi, target, window))
            .collect();

        let exact: Vec<&RegionResult> = results.iter().filter(|r| r.exact_min.is_some()).collect();
        let cost_at = |t: i64| ctx.eval(t).total_cost;
        if !exact.is_empty() {
            let pick = |r: &RegionResult| match cfg.reverse.prefer {
                Prefer::MinCost => r.exact_min.unwrap_or_default(),
                Prefer::MaxCost => r.exact_max.unwrap_or_default(),
            };
            let chosen = match cfg.reverse.prefer {
                Prefer::MinCost => exact.iter().min_by_key(|r| pick(r)),
                Prefer::MaxCost => exact.iter().max_by_key(|r| pick(r)),
            }
            .copied()
            .expect("非空");
            let ticks = pick(chosen);
            // 相邻分段的精确解若连成一片视为同一解，否则列为其它解
            let (mut rmin, mut rmax) = (
                chosen.exact_min.unwrap_or(ticks),
                chosen.exact_max.unwrap_or(ticks),
            );
            let mut others: Vec<Decimal> = Vec::new();
            let mut sorted = exact.clone();
            sorted.sort_by_key(|r| r.exact_min);
            for r in sorted {
                if std::ptr::eq(r, chosen) {
                    continue;
                }
                let (a, b) = (r.exact_min.unwrap_or(0), r.exact_max.unwrap_or(0));
                if b + 1 >= rmin && a <= rmax + 1 {
                    rmin = rmin.min(a);
                    rmax = rmax.max(b);
                } else {
                    others.push(cost_at(pick(r)));
                }
            }
            let breakdown = ctx.eval(ticks);
            debug_assert_eq!(breakdown.take_home, target);
            return Ok(ReverseOutcome {
                target,
                breakdown,
                exact: true,
                diff: Decimal::new(0, cfg.rounding.scale),
                exact_cost_range: Some((cost_at(rmin), cost_at(rmax))),
                other_solutions: others,
            });
        }

        let (ticks, diff) = results
            .iter()
            .map(|r| r.nearest)
            .min_by(|a, b| a.1.abs().cmp(&b.1.abs()).then(a.0.cmp(&b.0)))
            .expect("至少一个区间");
        let mut diff = diff;
        diff.rescale(cfg.rounding.scale);
        Ok(ReverseOutcome {
            target,
            breakdown: ctx.eval(ticks),
            exact: false,
            diff,
            exact_cost_range: None,
            other_solutions: Vec::new(),
        })
    }
}

/// 反算便捷函数（每次调用都会重新计算分段；批量请用 [`Reverser`]）。
pub fn reverse(cfg: &Config, take_home: Decimal) -> Result<ReverseOutcome, CalcError> {
    Reverser::new(cfg)?.reverse(take_home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward;
    use crate::forward::tests::legacy;
    use rust_decimal_macros::dec;

    #[test]
    fn reverse_default() {
        let cfg = Config::default();
        let r = reverse(&cfg, dec!(8400)).unwrap();
        assert!(r.exact);
        assert_eq!(r.breakdown.total_cost, dec!(10103.00));
        assert_eq!(r.breakdown.net_revenue, dec!(10000.00));
        assert_eq!(
            forward(&cfg, r.breakdown.total_cost).unwrap().take_home,
            dec!(8400)
        );
    }

    #[test]
    fn reverse_around_threshold() {
        let cfg = Config::default();
        let rev = Reverser::new(&cfg).unwrap();
        let a = rev.reverse(dec!(959.99)).unwrap();
        assert!(a.exact && a.breakdown.vat_exempt);
        assert_eq!(a.breakdown.total_cost, dec!(999.99));
        // 跳变段按征收处理时 1010.29 也能得到 959.99，取最小总成本
        assert_eq!(a.other_solutions, vec![dec!(1010.29)]);
        let b = rev.reverse(dec!(960.00)).unwrap();
        assert!(b.exact && !b.breakdown.vat_exempt);
        assert_eq!(b.breakdown.net_revenue, dec!(1000.00));
        assert_eq!(b.breakdown.total_cost, dec!(1010.30));
        assert!(b.other_solutions.is_empty());
    }

    #[test]
    fn reverse_zero() {
        let r = reverse(&Config::default(), dec!(0)).unwrap();
        assert!(r.exact);
        assert_eq!(r.breakdown.total_cost, dec!(0));
    }

    #[test]
    fn reverse_errors() {
        let cfg = Config::default();
        assert!(matches!(
            reverse(&cfg, dec!(-5)),
            Err(CalcError::NegativeAmount(_))
        ));
        assert!(matches!(
            reverse(&cfg, dec!(1.234)),
            Err(CalcError::PrecisionExceeded { .. })
        ));
        let mut small = cfg.clone();
        small.reverse.max_cost = dec!(1000);
        assert!(matches!(
            reverse(&small, dec!(5000)),
            Err(CalcError::TargetTooLarge { .. })
        ));
    }

    #[test]
    fn legacy_simple_min_cost() {
        let cfg = legacy();
        let r = reverse(&cfg, dec!(8397)).unwrap();
        assert!(r.exact);
        assert_eq!(
            forward(&cfg, r.breakdown.total_cost).unwrap().take_home,
            dec!(8397)
        );
        let below = r.breakdown.total_cost - dec!(0.01);
        assert_ne!(forward(&cfg, below).unwrap().take_home, dec!(8397));
    }

    #[test]
    fn legacy_cost_threshold_multi_solution() {
        let mut cfg = legacy();
        cfg.vat.threshold_enabled = true;
        cfg.vat.threshold = dec!(500);
        // 不征收区间到手最高 499.99；征收区间从 494.90 起，两区间重叠 => 多解
        let r = reverse(&cfg, dec!(495.00)).unwrap();
        assert!(r.exact);
        assert_eq!(r.breakdown.total_cost, dec!(495.00));
        assert_eq!(r.other_solutions.len(), 1);
        assert_eq!(
            forward(&cfg, r.other_solutions[0]).unwrap().take_home,
            dec!(495.00)
        );
    }

    #[test]
    fn unreachable_marked() {
        // 构造跳变口径：收入 > 4000 时减除 30%，在 4000 处应纳税所得额从 3200 跳到 2800.01，
        // 到手向上跳变约 80 元，中间的到手金额不可精确达到。
        let mut cfg = Config::default();
        cfg.iit.percent_deduction_rate = dec!(0.3);
        let r = reverse(&cfg, dec!(3400)).unwrap();
        assert!(!r.exact);
        assert!(!r.diff.is_zero());
        let fwd = forward(&cfg, r.breakdown.total_cost).unwrap().take_home;
        assert_eq!(fwd - dec!(3400), r.diff);
    }

    #[test]
    fn downward_jump_multi_solution() {
        // 收入 > 4000 时只减除 10%：到手在 4000 处向下跳，存在两段精确解
        let mut cfg = Config::default();
        cfg.iit.percent_deduction_rate = dec!(0.1);
        let rev = Reverser::new(&cfg).unwrap();
        let r = rev.reverse(dec!(3350)).unwrap();
        assert!(r.exact);
        assert_eq!(r.other_solutions.len(), 1);
        assert!(r.other_solutions[0] > r.breakdown.total_cost);
        for c in std::iter::once(r.breakdown.total_cost).chain(r.other_solutions.clone()) {
            assert_eq!(forward(&cfg, c).unwrap().take_home, dec!(3350));
        }
    }

    #[test]
    fn segments_default() {
        let rev = Reverser::new(&Config::default()).unwrap();
        // 0 起；总成本达到 1000（开始征收）、收入超过 4000、应纳税所得额超过 20000、50000 处分段
        assert_eq!(
            rev.segment_starts(),
            vec![
                dec!(0.00),
                dec!(1000.00),
                dec!(4041.21),
                dec!(25257.51),
                dec!(63143.76)
            ]
        );
    }
}
