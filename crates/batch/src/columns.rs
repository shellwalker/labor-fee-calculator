//! 导出列：各批量方式可导出的列及默认顺序，导出列的选择与排序（[`ColumnLayout`]），
//! 以及命令行 `--columns` 的解析。
//!
//! 三种批量方式的导出列略有不同：只有“按税后实发金额测算”提供“差额”列（计算结果 − 目标税后实发金额）；
//! 附加税费明细列只列示参数中征收的项目（城市维护建设税、教育费附加、地方教育附加）。

use crate::InputKind;
use labor_fee_core::Config;

/// 一个导出列。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExportColumn {
    /// 原表各列（输入表格原有的全部列，作为一个整体按原顺序输出）。
    Source,
    /// 不含增值税收入。
    NetIncome,
    /// 增值税税额。
    Vat,
    /// 某一项增值税附加税费（按项目名称，如“城市维护建设税”）。
    Surcharge(String),
    /// 附加税费减征额。
    SurchargeRelief,
    /// 增值税附加税费合计。
    SurchargeTotal,
    /// 企业总支出。
    TotalCost,
    /// 每次收入（个人所得税计税收入）。
    Income,
    /// 减除费用。
    Deduction,
    /// 预扣预缴应纳税所得额。
    TaxableIncome,
    /// 预扣率。
    Rate,
    /// 速算扣除数。
    QuickDeduction,
    /// 应预扣预缴税额。
    IitTax,
    /// 税后实发金额。
    TakeHome,
    /// 测算状态。
    Status,
    /// 差额（仅按税后实发金额测算）。
    Diff,
    /// 备注。
    Notes,
}

const SURCHARGE_PREFIX: &str = "surcharge:";

impl ExportColumn {
    /// 稳定的英文标识（保存设置与命令行使用）；附加税费项目为 `surcharge:<项目名称>`。
    pub fn key(&self) -> String {
        let k = match self {
            ExportColumn::Source => "source",
            ExportColumn::NetIncome => "net_income",
            ExportColumn::Vat => "vat",
            ExportColumn::Surcharge(n) => return format!("{SURCHARGE_PREFIX}{n}"),
            ExportColumn::SurchargeRelief => "surcharge_relief",
            ExportColumn::SurchargeTotal => "surcharge_total",
            ExportColumn::TotalCost => "total_cost",
            ExportColumn::Income => "income",
            ExportColumn::Deduction => "deduction",
            ExportColumn::TaxableIncome => "taxable_income",
            ExportColumn::Rate => "rate",
            ExportColumn::QuickDeduction => "quick_deduction",
            ExportColumn::IitTax => "iit_tax",
            ExportColumn::TakeHome => "take_home",
            ExportColumn::Status => "status",
            ExportColumn::Diff => "diff",
            ExportColumn::Notes => "notes",
        };
        k.to_string()
    }

    /// 列标题（与导出文件的标题行一致；原表各列为界面上的说明名称）。
    pub fn title(&self) -> &str {
        match self {
            ExportColumn::Source => "原表各列",
            ExportColumn::NetIncome => "不含增值税收入",
            ExportColumn::Vat => "增值税税额",
            ExportColumn::Surcharge(n) => n,
            ExportColumn::SurchargeRelief => "附加税费减征额",
            ExportColumn::SurchargeTotal => "增值税附加税费合计",
            ExportColumn::TotalCost => "企业总支出",
            ExportColumn::Income => "每次收入",
            ExportColumn::Deduction => "减除费用",
            ExportColumn::TaxableIncome => "预扣预缴应纳税所得额",
            ExportColumn::Rate => "预扣率",
            ExportColumn::QuickDeduction => "速算扣除数",
            ExportColumn::IitTax => "应预扣预缴税额",
            ExportColumn::TakeHome => "税后实发金额",
            ExportColumn::Status => "测算状态",
            ExportColumn::Diff => "差额",
            ExportColumn::Notes => "备注",
        }
    }

    /// 简要说明（界面悬停提示）。
    pub fn description(&self) -> &'static str {
        match self {
            ExportColumn::Source => "输入表格原有的全部列，按原顺序整体输出",
            ExportColumn::NetIncome => "劳务报酬不含增值税收入",
            ExportColumn::Vat => "增值税税额（未达到起征点时为 0）",
            ExportColumn::Surcharge(_) => "该项附加税费实际金额（已考虑减征优惠）",
            ExportColumn::SurchargeRelief => "各项附加税费减征额合计",
            ExportColumn::SurchargeTotal => "各项附加税费实际金额合计",
            ExportColumn::TotalCost => "不含增值税收入 + 增值税税额 + 附加税费合计",
            ExportColumn::Income => "个人所得税预扣预缴的每次收入",
            ExportColumn::Deduction => "每次收入不超过 4000 元减除 800 元，超过的减除 20%",
            ExportColumn::TaxableIncome => "每次收入 − 减除费用",
            ExportColumn::Rate => "居民个人劳务报酬所得预扣率",
            ExportColumn::QuickDeduction => "预扣率表对应的速算扣除数",
            ExportColumn::IitTax => "应预扣预缴个人所得税税额",
            ExportColumn::TakeHome => "不含增值税收入 − 应预扣预缴税额",
            ExportColumn::Status => "已计算、精确匹配、无法精确匹配、数据错误等",
            ExportColumn::Diff => "计算结果 − 目标税后实发金额（仅按税后实发金额测算）",
            ExportColumn::Notes => "未达到起征点、其他可匹配收入、不可达原因、数据错误原因等",
        }
    }
}

/// 某种批量方式在当前参数下可导出的全部列（默认顺序，即默认导出的列与顺序）。
pub fn available_columns(cfg: &Config, kind: InputKind) -> Vec<ExportColumn> {
    use ExportColumn::*;
    let mut v = vec![Source, NetIncome, Vat];
    v.extend(
        cfg.surcharge
            .enabled_items()
            .map(|i| Surcharge(i.name.clone())),
    );
    v.extend([
        SurchargeRelief,
        SurchargeTotal,
        TotalCost,
        Income,
        Deduction,
        TaxableIncome,
        Rate,
        QuickDeduction,
        IitTax,
        TakeHome,
        Status,
    ]);
    if kind == InputKind::TakeHome {
        v.push(Diff);
    }
    v.push(Notes);
    v
}

/// 按标识或标题查找可导出列。
pub fn find_column(available: &[ExportColumn], s: &str) -> Option<ExportColumn> {
    let s = s.trim();
    available
        .iter()
        .find(|c| c.key() == s || c.title() == s)
        .cloned()
}

/// 解析命令行 `--columns`：逗号（半角或全角）分隔的列标识或列标题，按给出的顺序导出。
pub fn parse_columns(
    spec: &str,
    cfg: &Config,
    kind: InputKind,
) -> Result<Vec<ExportColumn>, String> {
    let available = available_columns(cfg, kind);
    let mut out: Vec<ExportColumn> = Vec::new();
    for part in spec.split([',', '，']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some(c) = find_column(&available, part) else {
            let names: Vec<String> = available
                .iter()
                .map(|c| format!("{}（{}）", c.key(), c.title()))
                .collect();
            return Err(format!(
                "{}不支持导出列“{part}”；可用的列：{}",
                kind.title(),
                names.join("、")
            ));
        };
        if out.contains(&c) {
            return Err(format!("导出列“{part}”重复"));
        }
        out.push(c);
    }
    if out.is_empty() {
        return Err("未指定任何导出列".into());
    }
    Ok(out)
}

/// 导出列的选择与顺序（每种批量方式各一份）。只保存列标识，
/// 与当前参数下的可导出列合并后使用：已保存的列按保存的顺序，参数新增的列（如新征收的附加税费项目）
/// 按默认顺序插入并默认选中，参数中已不存在的列忽略。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColumnLayout {
    /// 列标识的排列顺序；为空表示默认顺序。
    pub order: Vec<String>,
    /// 不导出的列标识。
    pub hidden: Vec<String>,
}

impl ColumnLayout {
    /// 是否为默认（全部列、默认顺序）。
    pub fn is_default(&self) -> bool {
        self.order.is_empty() && self.hidden.is_empty()
    }

    /// 与可导出列合并：返回实际顺序与各列是否导出。
    pub fn entries(&self, available: &[ExportColumn]) -> Vec<(ExportColumn, bool)> {
        let mut cols: Vec<ExportColumn> = Vec::with_capacity(available.len());
        for k in &self.order {
            if let Some(c) = available.iter().find(|c| &c.key() == k) {
                if !cols.contains(c) {
                    cols.push(c.clone());
                }
            }
        }
        // 未保存过的列：插到默认顺序中前一列之后
        for (i, c) in available.iter().enumerate() {
            if cols.contains(c) {
                continue;
            }
            let pos = available[..i]
                .iter()
                .rev()
                .find_map(|prev| cols.iter().position(|x| x == prev))
                .map_or(0, |p| p + 1);
            cols.insert(pos, c.clone());
        }
        cols.into_iter()
            .map(|c| {
                let on = !self.hidden.contains(&c.key());
                (c, on)
            })
            .collect()
    }

    /// 导出的列（按顺序）。
    pub fn selected(&self, available: &[ExportColumn]) -> Vec<ExportColumn> {
        self.entries(available)
            .into_iter()
            .filter_map(|(c, on)| on.then_some(c))
            .collect()
    }

    /// 由界面上的列表生成；与默认（全部导出、默认顺序）相同时返回默认。
    pub fn from_entries(entries: &[(ExportColumn, bool)], available: &[ExportColumn]) -> Self {
        let default = entries.len() == available.len()
            && entries
                .iter()
                .zip(available)
                .all(|((c, on), a)| *on && c == a);
        if default {
            return ColumnLayout::default();
        }
        ColumnLayout {
            order: entries.iter().map(|(c, _)| c.key()).collect(),
            hidden: entries
                .iter()
                .filter(|(_, on)| !on)
                .map(|(c, _)| c.key())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ExportColumn::*;

    #[test]
    fn columns_differ_by_kind() {
        let cfg = Config::default();
        let th = available_columns(&cfg, InputKind::TakeHome);
        let ni = available_columns(&cfg, InputKind::NetIncome);
        let tc = available_columns(&cfg, InputKind::TotalCost);
        assert!(th.contains(&Diff));
        assert!(!ni.contains(&Diff) && !tc.contains(&Diff));
        assert_eq!(th.len(), ni.len() + 1);
        assert_eq!(th[0], Source);
        assert_eq!(th.last(), Some(&Notes));
        // 默认三项附加税费均征收
        assert_eq!(
            th.iter()
                .filter(|c| matches!(c, Surcharge(_)))
                .map(ExportColumn::title)
                .collect::<Vec<_>>(),
            ["城市维护建设税", "教育费附加", "地方教育附加"]
        );
        let mut cfg2 = cfg.clone();
        cfg2.surcharge.items[2].enabled = false;
        assert!(!available_columns(&cfg2, InputKind::TakeHome)
            .contains(&Surcharge("地方教育附加".into())));
    }

    #[test]
    fn parse_spec() {
        let cfg = Config::default();
        let v = parse_columns(
            "source, 不含增值税收入，iit_tax,surcharge:教育费附加,税后实发金额",
            &cfg,
            InputKind::TakeHome,
        )
        .unwrap();
        assert_eq!(
            v,
            [
                Source,
                NetIncome,
                IitTax,
                Surcharge("教育费附加".into()),
                TakeHome
            ]
        );
        let e = parse_columns("diff", &cfg, InputKind::NetIncome).unwrap_err();
        assert!(e.contains("不支持导出列“diff”"), "{e}");
        assert!(parse_columns("vat,增值税税额", &cfg, InputKind::NetIncome)
            .unwrap_err()
            .contains("重复"));
        assert!(parse_columns(" , ", &cfg, InputKind::NetIncome).is_err());
    }

    #[test]
    fn layout_merge_and_roundtrip() {
        let cfg = Config::default();
        let avail = available_columns(&cfg, InputKind::TakeHome);
        let d = ColumnLayout::default();
        assert!(d.entries(&avail).iter().all(|(_, on)| *on));
        assert_eq!(d.selected(&avail), avail);

        // 调整顺序并取消两列
        let mut e = d.entries(&avail);
        e.swap(0, 1);
        e[2].1 = false;
        e.last_mut().unwrap().1 = false;
        let l = ColumnLayout::from_entries(&e, &avail);
        assert!(!l.is_default());
        assert_eq!(l.entries(&avail), e);
        assert_eq!(l.selected(&avail).len(), avail.len() - 2);
        assert_eq!(l.selected(&avail)[0], NetIncome);

        // 恢复为默认顺序且全选 → 默认
        assert!(ColumnLayout::from_entries(&d.entries(&avail), &avail).is_default());

        // 参数变化：停征地方教育附加后该列消失；再征收时回到默认位置（教育费附加之后）并默认选中
        let mut cfg2 = cfg.clone();
        cfg2.surcharge.items[2].enabled = false;
        let avail2 = available_columns(&cfg2, InputKind::TakeHome);
        let e2 = l.entries(&avail2);
        assert_eq!(e2.len(), avail2.len());
        let l2 = ColumnLayout::from_entries(&e2, &avail2);
        let e3 = l2.entries(&avail);
        let pos = |e: &[(ExportColumn, bool)], c: &ExportColumn| {
            e.iter().position(|(x, _)| x == c).unwrap()
        };
        let local = Surcharge("地方教育附加".into());
        assert_eq!(
            pos(&e3, &local),
            pos(&e3, &Surcharge("教育费附加".into())) + 1
        );
        assert!(e3[pos(&e3, &local)].1);
    }
}
