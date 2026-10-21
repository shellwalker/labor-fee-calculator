//! 反算：实际到手 → 企业总成本。
//!
//! 实际到手关于企业总成本是分段函数：个税各档（800/4000/20000/50000）处连续但斜率变化；
//! 增值税起征点处存在向下跳变；逐步舍入到分又会造成局部 ±1 分的抖动和“多个总成本对应同一到手”的平台。
//! 因此算法为：
//! 1. 按跳变点（起征点）把总成本划分为若干近似单调的区间；
//! 2. 在每个区间内对“分”做二分，定位到手首次 ≥ 目标的位置；
//! 3. 以该位置为中心按 0.01 上下逐分扫描（窗口见 `reverse.search_window`），
//!    正算校准：收集所有正算到手与目标完全相等的总成本；
//! 4. 有精确解则按 `reverse.prefer` 取最小（默认）或最大总成本，其它区间的精确解列入 `other_solutions`；
//!    无精确解则取正算到手与目标差额绝对值最小者（同差额取较小总成本），标记“不可精确达到”并给出差额。

use crate::config::{Config, Prefer};
use crate::error::CalcError;
use crate::forward::{forward_unchecked, validate_amount, Breakdown};
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
    /// 其它区间（如起征点另一侧）也能精确得到目标的总成本。
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

struct Ctx<'a> {
    cfg: &'a Config,
    scale: u32,
}

impl Ctx<'_> {
    fn dec(&self, ticks: i64) -> Decimal {
        Decimal::new(ticks, self.scale)
    }
    fn ticks(&self, d: Decimal) -> Result<i64, CalcError> {
        let m = d * Decimal::from(10i64.pow(self.scale));
        i64::try_from(m.trunc()).map_err(|_| CalcError::Overflow)
    }
    fn th(&self, ticks: i64) -> Decimal {
        forward_unchecked(self.cfg, self.dec(ticks)).take_home
    }
    /// 分段键：(是否应税, 是否按比例减除费用, 预扣率档次)。随总成本按字典序单调不减。
    fn key(&self, ticks: i64) -> (bool, bool, usize) {
        let b = forward_unchecked(self.cfg, self.dec(ticks));
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
    /// 分段区间（以最小金额单位计的闭区间）。
    regions: Vec<(i64, i64)>,
    max_ticks: i64,
}

impl Reverser {
    /// 校验配置并计算分段区间。
    pub fn new(cfg: &Config) -> Result<Self, CalcError> {
        cfg.validate()?;
        let ctx = Ctx {
            cfg,
            scale: cfg.rounding.scale,
        };
        let max_ticks = ctx.ticks(cfg.round(cfg.reverse.max_cost))?;
        let mut regions = Vec::new();
        let mut lo = 0i64;
        loop {
            let k = ctx.key(lo);
            if ctx.key(max_ticks) <= k {
                regions.push((lo, max_ticks));
                break;
            }
            // 二分找第一个分段键大于 k 的位置
            let (mut l, mut h) = (lo, max_ticks);
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

    /// 分段区间的起点（企业总成本），用于诊断与测试。
    pub fn segment_starts(&self) -> Vec<Decimal> {
        self.regions
            .iter()
            .map(|r| Decimal::new(r.0, self.cfg.rounding.scale))
            .collect()
    }

    /// 反算：目标实际到手 → 企业总成本及全部明细。
    pub fn reverse(&self, take_home: Decimal) -> Result<ReverseOutcome, CalcError> {
        let cfg = &self.cfg;
        let target = validate_amount(cfg, take_home)?;
        let ctx = Ctx {
            cfg,
            scale: cfg.rounding.scale,
        };
        let max_take_home = ctx.th(self.max_ticks);
        if target > max_take_home {
            return Err(CalcError::TargetTooLarge {
                target,
                max_cost: ctx.dec(self.max_ticks),
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
                    others.push(ctx.dec(pick(r)));
                }
            }
            let breakdown = forward_unchecked(cfg, ctx.dec(ticks));
            debug_assert_eq!(breakdown.take_home, target);
            return Ok(ReverseOutcome {
                target,
                breakdown,
                exact: true,
                diff: Decimal::ZERO,
                exact_cost_range: Some((ctx.dec(rmin), ctx.dec(rmax))),
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
            breakdown: forward_unchecked(cfg, ctx.dec(ticks)),
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
    use rust_decimal_macros::dec;

    #[test]
    fn reverse_simple() {
        let cfg = Config::default();
        let r = reverse(&cfg, dec!(8397)).unwrap();
        assert!(r.exact);
        assert_eq!(
            forward(&cfg, r.breakdown.total_cost).unwrap().take_home,
            dec!(8397)
        );
        // 取最小总成本
        let below = r.breakdown.total_cost - dec!(0.01);
        assert_ne!(forward(&cfg, below).unwrap().take_home, dec!(8397));
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
    fn threshold_jump_multi_solution() {
        let mut cfg = Config::default();
        cfg.vat.threshold_enabled = true;
        cfg.vat.threshold = dec!(500);
        // 免税区最大到手 499.99；应税区从 494.90 起。两区间重叠 => 多解；
        let r = reverse(&cfg, dec!(495.00)).unwrap();
        assert!(r.exact);
        assert_eq!(r.breakdown.total_cost, dec!(495.00));
        assert_eq!(r.other_solutions.len(), 1);
        assert_eq!(
            forward(&cfg, r.other_solutions[0]).unwrap().take_home,
            dec!(495.00)
        );
        // 高征收率 + 低起征点
        let mut c2 = Config::default();
        c2.vat.threshold_enabled = true;
        c2.vat.threshold = dec!(1);
        c2.vat.rate = dec!(0.5);
        c2.surcharge.items[0].rate = dec!(0);
        // 免税区 [0, 0.99] 到手 0..0.99；应税区从 1.00 → 不含税 0.67 起
        let r = reverse(&c2, dec!(0.99)).unwrap();
        assert!(r.exact);
        assert_eq!(r.breakdown.total_cost, dec!(0.99));
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
        // 0 起；收入超过 4000、应纳税所得额超过 20000、50000 处分段
        assert_eq!(rev.segment_starts().len(), 4);
    }
}
