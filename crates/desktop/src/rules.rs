//! 计算规则说明：根据当前生效的计算参数动态生成，显示在各测算页右侧。

use crate::settings::{prefer_text, rounding_mode_text, surcharge_base_text};
use labor_fee_core::{forward, Config, Decimal, Prefer};

/// 政策依据。
pub const POLICY_NAME: &str = "国家税务总局公告2018年第61号";
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

/// 不可达区间（企业总支出无对应收入的一段）。
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

/// 起征点处的不可达区间（未启用起征点或征收率为 0 时没有）。
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

/// 增值税起征点的描述。
pub fn vat_condition(cfg: &Config) -> String {
    if !cfg.vat.threshold_enabled {
        return "未启用起征点，全部金额均征收增值税".into();
    }
    let how = if cfg.vat.levy_at_threshold {
        "达到"
    } else {
        "超过"
    };
    format!(
        "起征点：不含增值税收入{how} {} 的征收；未达到的，增值税及附加税费均为 0",
        yuan(cfg.vat.threshold)
    )
}

/// 增值税附加税费的计算说明（每个已启用项目一行，另附合计与减征说明）。
pub fn surcharge_lines(cfg: &Config) -> Vec<String> {
    let base = surcharge_base_text(cfg.surcharge.base);
    let relief = cfg.surcharge.relief_enabled;
    let mut v: Vec<String> = cfg
        .surcharge
        .enabled_items()
        .map(|it| {
            if relief {
                format!(
                    "{} = {base} × {} × (1 − {})",
                    it.name,
                    pct(it.rate),
                    pct(cfg.surcharge.relief_rate)
                )
            } else {
                format!("{} = {base} × {}", it.name, pct(it.rate))
            }
        })
        .collect();
    if v.is_empty() {
        v.push("未启用任何附加税费项目".into());
    } else {
        let names: Vec<&str> = cfg
            .surcharge
            .enabled_items()
            .map(|i| i.name.as_str())
            .collect();
        v.push(format!(
            "增值税附加税费 = {}（{}）",
            names.join(" + "),
            match cfg.rounding.surcharge {
                labor_fee_core::SurchargeRounding::PerItem => "各项分别按分舍入",
                labor_fee_core::SurchargeRounding::Total => "合计后一次舍入",
            }
        ));
    }
    if relief {
        v.push(format!(
            "已启用减征，减征比例 {}（可配置假设，是否适用请自行判断）",
            pct(cfg.surcharge.relief_rate)
        ));
    }
    let off: Vec<&str> = cfg
        .surcharge
        .items
        .iter()
        .filter(|i| !i.enabled)
        .map(|i| i.name.as_str())
        .collect();
    if !off.is_empty() {
        v.push(format!("未启用：{}", off.join("、")));
    }
    v
}

/// 全部规则。
pub fn rule_groups(cfg: &Config) -> Vec<RuleGroup> {
    let iit = &cfg.iit;
    let mut vat_lines = vec![
        format!("增值税税额 = 不含增值税收入 × 征收率 {}", pct(cfg.vat.rate)),
        vat_condition(cfg),
    ];
    vat_lines.extend(surcharge_lines(cfg));
    vat_lines.push("企业总支出 = 不含增值税收入 + 增值税税额 + 增值税附加税费".into());
    let mut groups = vec![
        RuleGroup {
            title: "增值税及附加税费（企业承担）",
            lines: vat_lines,
        },
        RuleGroup {
            title: "个人所得税（居民个人劳务报酬所得预扣预缴）",
            lines: vec![
                format!(
                    "每次收入不超过 {} 的，减除费用 {}；{} 以上的，减除费用按 {} 计算",
                    yuan(iit.fixed_deduction_limit),
                    yuan(iit.fixed_deduction),
                    yuan(iit.fixed_deduction_limit),
                    pct(iit.percent_deduction_rate)
                ),
                "预扣预缴应纳税所得额 = 每次收入 − 减除费用（每次收入即不含增值税收入）".into(),
                "应预扣预缴税额 = 预扣预缴应纳税所得额 × 预扣率 − 速算扣除数（见下表）".into(),
                "税后实发金额 = 不含增值税收入 − 应预扣预缴税额（附加税费由企业承担，不从中扣除）"
                    .into(),
            ],
        },
        RuleGroup {
            title: "舍入与测算",
            lines: vec![
                format!(
                    "各项金额按“{}”保留 {} 位小数{}",
                    rounding_mode_text(cfg.rounding.mode),
                    cfg.rounding.scale,
                    if cfg.rounding.scale == 2 {
                        "（到分）"
                    } else {
                        ""
                    }
                ),
                format!(
                    "按税后实发金额测算时，多个不含增值税收入对应同一金额的，{}",
                    prefer_text(cfg.reverse.prefer)
                ),
                "每笔收入单独计算；同一项目连续性收入以一个月内取得的收入为一次，请合并后录入"
                    .into(),
            ],
        },
    ];
    if cfg.reverse.prefer == Prefer::MaxCost {
        groups[2].lines[1] += "（非默认）";
    }
    if let Some(g) = vacuum_gap(cfg) {
        let (lo, hi) = g.range(cfg.unit());
        groups.push(RuleGroup {
            title: "不可达区间（按企业总支出测算）",
            lines: vec![
                format!(
                    "不含增值税收入 {} → 企业总支出 {}；{} → {}",
                    g.below_net, g.below_cost, g.at_net, g.at_cost
                ),
                format!("企业总支出 {lo}～{hi} 无对应的不含增值税收入，输入时将提示原因及两侧最近的有效金额"),
            ],
        });
    }
    groups
}

/// 增值税起征点与 1% 优惠征收率依据。
pub const POLICY_VAT_NAME: &str = "财政部 税务总局公告2026年第10号";
pub const POLICY_VAT_URL: &str = "https://www.mof.gov.cn/jrttts/202602/t20260203_3983175.htm";
/// “六税两费”减征依据。
pub const POLICY_RELIEF_NAME: &str = "财政部 税务总局公告2023年第12号";
pub const POLICY_RELIEF_URL: &str =
    "https://www.gov.cn/zhengce/zhengceku/202308/content_6896379.htm";
