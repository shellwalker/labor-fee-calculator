//! 试算规则说明：根据当前生效的口径动态生成，显示在正算 / 反算 / 按总成本页右侧。

use crate::settings::{prefer_text, rounding_mode_text, surcharge_base_text};
use labor_fee_core::{forward, Config, Decimal, Prefer};

/// 政策依据。
pub const POLICY_NAME: &str = "国家税务总局公告 2018 年第 61 号";
pub const POLICY_DETAIL: &str =
    "《个人所得税扣缴申报管理办法（试行）》第八条及个人所得税预扣率表二";
pub const POLICY_URL: &str =
    "https://www.chinatax.gov.cn/n810219/n810744/n3752930/n3752974/c3963396/content.html";
pub const POLICY_URL_GOV: &str = "https://www.gov.cn/gongbao/content/2019/content_5386991.htm";

/// 一组规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleGroup {
    pub title: &'static str,
    pub lines: Vec<String>,
}

/// 预扣率表的一行（显示用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BracketRow {
    pub range: String,
    pub rate: String,
    pub quick: String,
}

/// 真空区间（按总成本无对应收入的一段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VacuumGap {
    pub below_net: Decimal,
    pub below_cost: Decimal,
    pub at_net: Decimal,
    pub at_cost: Decimal,
}

impl VacuumGap {
    /// 空白段的首尾（含）。
    pub fn range(&self, unit: Decimal) -> (Decimal, Decimal) {
        (self.below_cost + unit, self.at_cost - unit)
    }
}

pub fn pct(d: Decimal) -> String {
    format!("{}%", (d * Decimal::ONE_HUNDRED).normalize())
}

fn yuan(d: Decimal) -> String {
    format!("{} 元", d.normalize())
}

/// 门槛处的真空区间（未启用门槛或税率为 0 时没有）。
pub fn vacuum_gap(cfg: &Config) -> Option<VacuumGap> {
    if !cfg.vat.threshold_enabled || cfg.vat.rate.is_zero() {
        return None;
    }
    let unit = cfg.unit();
    // 第一个征收的不含税收入
    let at = if cfg.vat.levy_at_threshold {
        cfg.vat.threshold
    } else {
        cfg.vat.threshold + unit
    };
    let at = cfg.round(at);
    let below = at - unit;
    if below < Decimal::ZERO {
        return None;
    }
    let a = forward(cfg, at).ok()?;
    let b = forward(cfg, below).ok()?;
    (a.total_cost - b.total_cost > unit).then_some(VacuumGap {
        below_net: b.net_income,
        below_cost: b.total_cost,
        at_net: a.net_income,
        at_cost: a.total_cost,
    })
}

pub fn bracket_rows(cfg: &Config) -> Vec<BracketRow> {
    let mut lower: Option<Decimal> = None;
    cfg.iit
        .brackets
        .iter()
        .map(|b| {
            let range = match (lower, b.upper) {
                (None, Some(u)) => format!("≤ {}", u.normalize()),
                (Some(l), Some(u)) => format!("{}～{}", l.normalize(), u.normalize()),
                (Some(l), None) => format!("> {}", l.normalize()),
                (None, None) => "全部".to_string(),
            };
            lower = b.upper;
            BracketRow {
                range,
                rate: pct(b.rate),
                quick: b.quick_deduction.normalize().to_string(),
            }
        })
        .collect()
}

/// 增值税征收条件的描述。
pub fn vat_condition(cfg: &Config) -> String {
    if !cfg.vat.threshold_enabled {
        return "不设征收门槛，全部征收".into();
    }
    let op = if cfg.vat.levy_at_threshold {
        "≥"
    } else {
        ">"
    };
    format!(
        "不含税收入 {op} {} 才征收，低于门槛增值税、附加税均为 0",
        yuan(cfg.vat.threshold)
    )
}

/// 附加税公式。
pub fn surcharge_formula(cfg: &Config) -> String {
    let base = surcharge_base_text(cfg.surcharge.base);
    let items: Vec<String> = cfg
        .surcharge
        .items
        .iter()
        .zip(cfg.effective_surcharge_rates())
        .map(|(it, r)| {
            if cfg.surcharge.items.len() == 1 {
                pct(r)
            } else {
                format!("{} {}", it.name, pct(r))
            }
        })
        .collect();
    let mut s = if items.is_empty() {
        "不征附加税".to_string()
    } else {
        format!("附加税 = {base} × {}", items.join(" + "))
    };
    if cfg.surcharge.halve {
        s += "（已减半）";
    }
    s
}

/// 全部规则（`page_note` 为本页特有说明，显示在最前）。
pub fn rule_groups(cfg: &Config) -> Vec<RuleGroup> {
    let iit = &cfg.iit;
    let mut groups = vec![
        RuleGroup {
            title: "增值税与附加税（企业承担）",
            lines: vec![
                format!("增值税 = 不含税收入 × {}", pct(cfg.vat.rate)),
                vat_condition(cfg),
                surcharge_formula(cfg),
                "企业总成本 = 不含税收入 + 增值税 + 附加税".into(),
            ],
        },
        RuleGroup {
            title: "个税（居民个人劳务报酬预扣预缴）",
            lines: vec![
                format!(
                    "每次收入 ≤ {} 减除费用 {}；> {} 减除 {}",
                    yuan(iit.fixed_deduction_limit),
                    yuan(iit.fixed_deduction),
                    yuan(iit.fixed_deduction_limit),
                    pct(iit.percent_deduction_rate)
                ),
                "应纳税所得额 = 不含税收入 − 减除费用".into(),
                "个税 = 应纳税所得额 × 预扣率 − 速算扣除数（见下表）".into(),
                "实际到手 = 不含税收入 − 个税（附加税不从到手扣）".into(),
            ],
        },
        RuleGroup {
            title: "舍入与反算",
            lines: vec![
                format!(
                    "每一步金额按“{}”保留 {} 位小数{}",
                    rounding_mode_text(cfg.rounding.mode),
                    cfg.rounding.scale,
                    if cfg.rounding.scale == 2 {
                        "（到分）"
                    } else {
                        ""
                    }
                ),
                format!(
                    "反算：多个不含税收入得到同一到手时，{}",
                    prefer_text(cfg.reverse.prefer)
                ),
                "每笔单独计算，不做按月合并".into(),
            ],
        },
    ];
    if cfg.reverse.prefer == Prefer::MaxCost {
        groups[2].lines[1] += "（非默认）";
    }
    if let Some(g) = vacuum_gap(cfg) {
        let (lo, hi) = g.range(cfg.unit());
        groups.push(RuleGroup {
            title: "真空区间（按总成本）",
            lines: vec![
                format!(
                    "不含税收入 {} → 总成本 {}；{} → {}",
                    g.below_net, g.below_cost, g.at_net, g.at_cost
                ),
                format!("总成本 {lo}～{hi} 没有对应的收入，正常业务不会出现，输入时会提示原因"),
            ],
        });
    }
    groups
}
