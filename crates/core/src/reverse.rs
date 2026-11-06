//! 反算：实际到手 → 不含税收入 → 全部金额。
//!
//! 实际到手 = 不含税收入 − 个税，是不含税收入的分段函数：个税各档（800/4000/20000/50000）处连续但斜率变化，
//! 逐步舍入到分又会造成“相邻几个分的不含税收入得到同一到手”的平台。算法：
//! 1. 按分段键（减除费用方式 / 预扣率档次 / 是否征收增值税）把不含税收入划分为若干区间；
//! 2. 在每个区间内对“分”做二分，定位到手首次 ≥ 目标的位置；
//! 3. 再二分定位精确解平台的末端，完整保留平台边界，不依赖扫描窗口；
//! 4. 有精确解则按 `reverse.prefer` 取最小（默认）或最大不含税收入，其它区间的精确解列入 `other_solutions`；
//!    无精确解则取差额绝对值最小者（同差额取较小收入），标记“不可精确达到”并给出差额。
//!
//! 默认口径下到手随不含税收入单调不减、每分最多增加 0.01，因此任意到手都有精确解（测试已遍历验证）。
//! 无解只会在自定义口径（例如改动减除比例造成到手跳变）下出现。

use crate::config::{Config, Prefer};
use crate::error::CalcError;
use crate::forward::{
    eval_net, from_ticks, iit_taxable_income, to_ticks, validate_amount, Breakdown,
};
use rust_decimal::Decimal;
use serde::Serialize;

/// 反算结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReverseOutcome {
    /// 目标实际到手。
    pub target: Decimal,
    /// 选定的不含税收入对应的完整正算结果。
    pub breakdown: Breakdown,
    /// 正算到手是否与目标完全一致。
    pub exact: bool,
    /// 差额 = 正算到手 − 目标（精确时为 0）。
    pub diff: Decimal,
    /// 选定解所在区间内，能精确得到目标的最小与最大不含税收入（仅精确时有值）。
    pub exact_net_income_range: Option<(Decimal, Decimal)>,
    /// 其它不相连区间也能精确得到目标的不含税收入。
    pub other_solutions: Vec<Decimal>,
}

impl ReverseOutcome {
    /// 中文状态说明。
    pub fn status_text(&self) -> String {
        if self.exact {
            "精确匹配".to_string()
        } else {
            format!("无法精确匹配，差额 {}", self.diff)
        }
    }
}

struct Ctx<'a> {
    cfg: &'a Config,
}

impl Ctx<'_> {
    fn dec(&self, ticks: i64) -> Decimal {
        from_ticks(self.cfg, ticks)
    }
    fn eval(&self, ticks: i64) -> Result<Breakdown, CalcError> {
        eval_net(self.cfg, self.dec(ticks))
    }
    fn th(&self, ticks: i64) -> Result<Decimal, CalcError> {
        Ok(self.eval(ticks)?.take_home)
    }
    /// 分段键：(是否按比例减除费用, 预扣率档次, 是否征收增值税)。随不含税收入按字典序单调不减。
    fn key(&self, ticks: i64) -> Result<(bool, usize, bool), CalcError> {
        let b = self.eval(ticks)?;
        let iit = &self.cfg.iit;
        let pct = b.iit.income > iit.fixed_deduction_limit;
        // Breakdown amounts are rescaled for display. A fractional configured
        // fixed deduction may put the actual taxable amount just above a
        // bracket boundary even when its displayed amount equals that boundary.
        let income = self.dec(ticks);
        let taxable = iit_taxable_income(self.cfg, income)?;
        let idx = iit
            .brackets
            .iter()
            .position(|br| br.upper.is_none_or(|u| taxable <= u))
            .unwrap_or(iit.brackets.len());
        Ok((pct, idx, !b.vat_exempt))
    }
}

struct RegionResult {
    exact_min: Option<i64>,
    exact_max: Option<i64>,
    nearest: (i64, Decimal),
}

// Within a region both configured rates are < 1. Raising income by one tick
// raises rounded percentage deduction by zero or one tick, hence taxable
// income by zero or one tick. Rounded tax also rises by at most one tick.
// Fixed deduction has the same property. This holds for all rounding modes,
// including half-even (the raw increments are strictly less than one tick).
// Thus nested rounding cannot create downward microsteps inside a region.
fn first_at_least(ctx: &Ctx, lo: i64, hi: i64, value: Decimal) -> Result<i64, CalcError> {
    if ctx.th(lo)? >= value {
        return Ok(lo);
    }
    if ctx.th(hi)? < value {
        return Ok(hi);
    }
    let (mut l, mut h) = (lo, hi);
    while l < h {
        let m = l + (h - l) / 2;
        if ctx.th(m)? >= value {
            h = m;
        } else {
            l = m + 1;
        }
    }
    Ok(l)
}

fn search_region(ctx: &Ctx, lo: i64, hi: i64, target: Decimal) -> Result<RegionResult, CalcError> {
    let pos = first_at_least(ctx, lo, hi, target)?;
    let value = ctx.th(pos)?;
    if value == target {
        let (mut l, mut h) = (pos, hi);
        while l < h {
            let m = l + (h - l) / 2 + (h - l) % 2;
            if ctx.th(m)? <= target {
                l = m;
            } else {
                h = m - 1;
            }
        }
        return Ok(RegionResult {
            exact_min: Some(pos),
            exact_max: Some(l),
            nearest: (pos, Decimal::ZERO),
        });
    }
    // If the closest value is below target its plateau can be arbitrarily
    // long. Find its first tick too, preserving the lower-income tie rule.
    let mut nearest = (first_at_least(ctx, lo, pos, value)?, value - target);
    if pos > lo {
        let below = ctx.th(pos - 1)?;
        let tick = first_at_least(ctx, lo, pos - 1, below)?;
        let diff = below - target;
        if (diff.abs(), tick) < (nearest.1.abs(), nearest.0) {
            nearest = (tick, diff);
        }
    }
    Ok(RegionResult {
        exact_min: None,
        exact_max: None,
        nearest,
    })
}

/// 可复用的反算器：按配置预先求出分段区间，批量反算时避免重复计算。
#[derive(Debug, Clone)]
pub struct Reverser {
    cfg: Config,
    /// 分段区间（不含税收入，以最小金额单位计的闭区间）。
    regions: Vec<(i64, i64)>,
}

impl Reverser {
    /// 校验配置并计算分段区间。
    pub fn new(cfg: &Config) -> Result<Self, CalcError> {
        cfg.validate()?;
        let ctx = Ctx { cfg };
        let max_ticks = to_ticks(cfg, cfg.round(cfg.reverse.max_net_income))?;
        let mut regions = Vec::new();
        let mut lo = 0i64;
        loop {
            let k = ctx.key(lo)?;
            if ctx.key(max_ticks)? <= k {
                regions.push((lo, max_ticks));
                break;
            }
            // 二分找第一个分段键大于 k 的位置
            let (mut l, mut h) = (lo, max_ticks);
            while h - l > 1 {
                let m = l + (h - l) / 2;
                if ctx.key(m)? > k {
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
        })
    }

    /// 配置。
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// 各分段区间起点的不含税收入，用于诊断与测试。
    pub fn segment_starts(&self) -> Vec<Decimal> {
        self.regions
            .iter()
            .map(|r| from_ticks(&self.cfg, r.0))
            .collect()
    }

    /// 反算：目标实际到手 → 不含税收入及全部明细。
    pub fn reverse(&self, take_home: Decimal) -> Result<ReverseOutcome, CalcError> {
        let cfg = &self.cfg;
        let target = validate_amount(cfg, take_home)?;
        let ctx = Ctx { cfg };
        let last = self.regions.last().map(|r| r.1).unwrap_or(0);
        let max_take_home = self
            .regions
            .iter()
            .try_fold(Decimal::ZERO, |maximum, &(_, hi)| {
                Ok::<_, CalcError>(maximum.max(ctx.th(hi)?))
            })?;
        if target > max_take_home {
            return Err(CalcError::TargetTooLarge {
                target,
                max_net_income: ctx.dec(last),
                max_take_home,
            });
        }
        let mut results = Vec::with_capacity(self.regions.len());
        for &(lo, hi) in &self.regions {
            results.push(search_region(&ctx, lo, hi, target)?);
        }

        // Merge all adjacent exact plateaus before selecting a component.
        // Doing this only while visiting the chosen region misses transitive
        // adjacency when max_cost selects the final region.
        let mut exact: Vec<(i64, i64)> = Vec::new();
        for r in &results {
            if let (Some(a), Some(b)) = (r.exact_min, r.exact_max) {
                if let Some(last) = exact.last_mut() {
                    if a - last.1 <= 1 {
                        last.1 = b;
                        continue;
                    }
                }
                exact.push((a, b));
            }
        }
        if !exact.is_empty() {
            let pick = |r: &(i64, i64)| match cfg.reverse.prefer {
                Prefer::MinCost => r.0,
                Prefer::MaxCost => r.1,
            };
            let chosen = match cfg.reverse.prefer {
                Prefer::MinCost => 0,
                Prefer::MaxCost => exact.len() - 1,
            };
            let (rmin, rmax) = exact[chosen];
            let ticks = pick(&exact[chosen]);
            let others = exact
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != chosen)
                .map(|(_, r)| ctx.dec(pick(r)))
                .collect();
            let breakdown = ctx.eval(ticks)?;
            debug_assert_eq!(breakdown.take_home, target);
            return Ok(ReverseOutcome {
                target,
                breakdown,
                exact: true,
                diff: Decimal::new(0, cfg.rounding.scale),
                exact_net_income_range: Some((ctx.dec(rmin), ctx.dec(rmax))),
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
            breakdown: ctx.eval(ticks)?,
            exact: false,
            diff,
            exact_net_income_range: None,
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
    fn reverse_default() {
        let cfg = Config::default();
        let r = reverse(&cfg, dec!(8400)).unwrap();
        assert!(r.exact);
        assert_eq!(r.breakdown.net_income, dec!(10000.00));
        assert_eq!(r.breakdown.total_cost, dec!(10103.00));
        assert_eq!(forward(&cfg, r.breakdown.net_income).unwrap(), r.breakdown);
    }

    #[test]
    fn reverse_around_threshold() {
        let rev = Reverser::new(&Config::default()).unwrap();
        let a = rev.reverse(dec!(959.99)).unwrap();
        assert!(a.exact && a.breakdown.vat_exempt);
        assert_eq!(a.breakdown.net_income, dec!(999.99));
        assert_eq!(a.breakdown.total_cost, dec!(999.99));
        assert!(a.other_solutions.is_empty());
        let b = rev.reverse(dec!(960.00)).unwrap();
        assert!(b.exact && !b.breakdown.vat_exempt);
        assert_eq!(b.breakdown.net_income, dec!(1000.00));
        assert_eq!(b.breakdown.total_cost, dec!(1010.30));
        assert!(b.other_solutions.is_empty());
    }

    #[test]
    fn plateau_takes_min() {
        // 4000 以上：收入每增 0.01，应纳税所得额增 0.008，个税偶尔增 0.01，到手出现两个收入对应同一值
        let cfg = Config::default();
        let rev = Reverser::new(&cfg).unwrap();
        let r = rev.reverse(dec!(4000)).unwrap();
        let (lo, hi) = r.exact_net_income_range.unwrap();
        assert_eq!(r.breakdown.net_income, lo);
        assert!(hi >= lo);
        if hi > lo {
            let below = lo - dec!(0.01);
            assert_ne!(forward(&cfg, below).unwrap().take_home, dec!(4000));
        }
    }

    #[test]
    fn reverse_zero() {
        let r = reverse(&Config::default(), dec!(0)).unwrap();
        assert!(r.exact);
        assert_eq!(r.breakdown.net_income, dec!(0));
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
        small.reverse.max_net_income = dec!(1000);
        assert!(matches!(
            reverse(&small, dec!(5000)),
            Err(CalcError::TargetTooLarge { .. })
        ));
    }

    #[test]
    fn unreachable_marked_custom_config() {
        // 自定义口径：收入 > 4000 时减除 30%，到手在 4000 处向上跳约 80 元，中间到手不可达
        let mut cfg = Config::default();
        cfg.iit.percent_deduction_rate = dec!(0.3);
        let r = reverse(&cfg, dec!(3400)).unwrap();
        assert!(!r.exact);
        let fwd = forward(&cfg, r.breakdown.net_income).unwrap().take_home;
        assert_eq!(fwd - dec!(3400), r.diff);
    }

    #[test]
    fn downward_jump_multi_solution_custom_config() {
        // 自定义口径：收入 > 4000 时只减除 10%，到手在 4000 处向下跳，存在两段精确解
        let mut cfg = Config::default();
        cfg.iit.percent_deduction_rate = dec!(0.1);
        let r = Reverser::new(&cfg).unwrap().reverse(dec!(3350)).unwrap();
        assert!(r.exact);
        assert_eq!(r.other_solutions.len(), 1);
        assert!(r.other_solutions[0] > r.breakdown.net_income);
        for n in std::iter::once(r.breakdown.net_income).chain(r.other_solutions.clone()) {
            assert_eq!(forward(&cfg, n).unwrap().take_home, dec!(3350));
        }
    }

    #[test]
    fn segments_default() {
        let rev = Reverser::new(&Config::default()).unwrap();
        assert_eq!(
            rev.segment_starts(),
            vec![
                dec!(0.00),
                dec!(1000.00),
                dec!(4000.01),
                dec!(25000.01),
                dec!(62500.01)
            ]
        );
    }
}
