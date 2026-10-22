//! 设置：主题偏好与计算口径的编辑、校验与持久化。
//!
//! 保存位置为平台标准配置目录下的 `labor-fee-desktop/`：
//! - Linux：`~/.config/labor-fee-desktop/`（遵循 `$XDG_CONFIG_HOME`）
//! - macOS：`~/Library/Application Support/labor-fee-desktop/`
//! - Windows：`%APPDATA%\labor-fee-desktop\`
//!
//! 其中 `preferences.toml` 保存主题，`rules.toml` 保存口径（只有与内置默认不同才写入，恢复默认时删除）。
//! 用户只在“设置”窗口里逐项编辑，不需要接触这些文件。

use eframe::egui::ThemePreference;
use labor_fee_core::{
    Bracket, Config, Decimal, Prefer, Reverser, RoundingMode, SurchargeBase, SurchargeItem,
    SurchargeRounding,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// 主题选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    /// 跟随系统（默认）。
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    pub fn title(self) -> &'static str {
        match self {
            ThemeChoice::System => "跟随系统",
            ThemeChoice::Light => "浅色",
            ThemeChoice::Dark => "深色",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            ThemeChoice::System => crate::icons::SYSTEM,
            ThemeChoice::Light => crate::icons::SUN,
            ThemeChoice::Dark => crate::icons::MOON,
        }
    }

    /// 顶栏快捷切换的下一个选项。
    pub fn next(self) -> Self {
        match self {
            ThemeChoice::System => ThemeChoice::Light,
            ThemeChoice::Light => ThemeChoice::Dark,
            ThemeChoice::Dark => ThemeChoice::System,
        }
    }
}

impl From<ThemeChoice> for ThemePreference {
    fn from(t: ThemeChoice) -> Self {
        match t {
            ThemeChoice::System => ThemePreference::System,
            ThemeChoice::Light => ThemePreference::Light,
            ThemeChoice::Dark => ThemePreference::Dark,
        }
    }
}

/// 界面偏好。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeChoice,
}

/// 设置的存储位置。
#[derive(Debug, Clone)]
pub struct SettingsStore {
    pub dir: PathBuf,
}

impl SettingsStore {
    /// 平台标准配置目录；取不到（极少见）时返回 None，此时设置只在本次运行有效。
    pub fn platform() -> Option<Self> {
        dirs::config_dir().map(|d| SettingsStore {
            dir: d.join("labor-fee-desktop"),
        })
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        SettingsStore { dir: dir.into() }
    }

    pub fn prefs_path(&self) -> PathBuf {
        self.dir.join("preferences.toml")
    }

    pub fn rules_path(&self) -> PathBuf {
        self.dir.join("rules.toml")
    }

    /// 读取偏好；文件不存在或损坏时用默认值。
    pub fn load_prefs(&self) -> Prefs {
        std::fs::read_to_string(self.prefs_path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// 读取已保存的口径：Ok(None) 表示没有保存过（用内置默认）。
    pub fn load_rules(&self) -> Result<Option<Config>, String> {
        let p = self.rules_path();
        match std::fs::read_to_string(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读取已保存的设置失败：{e}")),
            Ok(t) => Config::from_toml_str(&t).map(Some).map_err(|e| {
                format!(
                    "已保存的设置无效（{}），已改用内置默认参数：{e}",
                    p.display()
                )
            }),
        }
    }

    pub fn save_prefs(&self, prefs: &Prefs) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("创建设置目录失败：{e}"))?;
        let text = toml::to_string_pretty(prefs).map_err(|e| e.to_string())?;
        write_atomic(&self.prefs_path(), &text)
    }

    /// 保存计算参数；等于内置默认时删除文件（以后内置默认更新也能自动跟上）。
    pub fn save_rules(&self, cfg: &Config) -> Result<(), String> {
        let p = self.rules_path();
        if *cfg == Config::default() {
            return match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    Err(format!("删除旧设置失败：{e}"))
                }
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("创建设置目录失败：{e}"))?;
        let text = format!(
            "# 劳务报酬计算工具的计算参数（由程序“设置”窗口生成，请在程序内修改）\n{}",
            cfg.to_toml_string()
        );
        write_atomic(&p, &text)
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("保存设置失败：{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("保存设置失败：{e}"))
}

// ---------------- 口径确认状态 ----------------

/// 口径项的确认状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    /// 用户已确认。
    Confirmed,
    /// 已按公开资料核实。
    Verified,
    /// 待确认。
    Pending,
    /// 实现参数（不涉及税务口径）。
    Implementation,
    /// 已改为非默认值（未经确认）。
    Modified,
}

impl Tag {
    pub fn text(self) -> &'static str {
        match self {
            Tag::Confirmed => "已确认",
            Tag::Verified => "已核实",
            Tag::Pending => "待确认",
            Tag::Implementation => "实现参数",
            Tag::Modified => "已修改",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tag::Confirmed => "用户已确认的参数",
            Tag::Verified => "已对照公开政策文件核实",
            Tag::Pending => "可配置假设，尚未确认；正式结算前请财务人员确认",
            Tag::Implementation => "计算实现参数，不涉及税收政策",
            Tag::Modified => "已改为与内置默认不同的值，尚未确认",
        }
    }
}

// ---------------- 设置草稿（设置窗口的编辑状态） ----------------

/// 设置窗口的分组。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Vat,
    Surcharge,
    Iit,
    Rounding,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Vat,
        Section::Surcharge,
        Section::Iit,
        Section::Rounding,
        Section::Appearance,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Appearance => "外观",
            Section::Vat => "增值税",
            Section::Surcharge => "附加税费",
            Section::Iit => "个人所得税",
            Section::Rounding => "舍入与测算",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDraft {
    pub name: String,
    /// 费率（百分数，如 "3"）。
    pub rate: String,
    /// 是否计征。
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BracketDraft {
    /// 上限（元，含本数）；最后一档忽略。
    pub upper: String,
    /// 预扣率（百分数）。
    pub rate: String,
    pub quick: String,
}

/// 设置窗口中可编辑的全部字段（文本形式，百分比以百分数显示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsDraft {
    pub section: Section,
    pub theme: ThemeChoice,
    pub vat_rate: String,
    pub threshold_enabled: bool,
    pub threshold: String,
    pub levy_at_threshold: bool,
    pub surcharge_base: SurchargeBase,
    pub items: Vec<ItemDraft>,
    pub fixed_limit: String,
    pub fixed_deduction: String,
    pub pct_deduction: String,
    pub brackets: Vec<BracketDraft>,
    pub scale: String,
    pub mode: RoundingMode,
    pub surcharge_rounding: SurchargeRounding,
    pub prefer: Prefer,
    pub search_window: String,
    pub max_net_income: String,
    /// 保存失败等提示。
    pub message: Option<String>,
}

/// 附加税费项目是否与默认一致：名称、是否计征相同，费率按数值比较（“7”与“7.0”视为相同）。
pub fn item_same(a: &ItemDraft, b: &ItemDraft) -> bool {
    a.name.trim() == b.name.trim()
        && a.enabled == b.enabled
        && parse_dec(&a.rate).is_some()
        && parse_dec(&a.rate) == parse_dec(&b.rate)
}

pub fn items_same(a: &[ItemDraft], b: &[ItemDraft]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| item_same(x, y))
}

fn pct_text(d: Decimal) -> String {
    (d * Decimal::ONE_HUNDRED).normalize().to_string()
}

fn num_text(d: Decimal) -> String {
    d.normalize().to_string()
}

/// 字段标识（用于错误提示定位与单项恢复默认）。
pub mod key {
    pub const VAT_RATE: &str = "vat.rate";
    pub const THRESHOLD: &str = "vat.threshold";
    pub const FIXED_LIMIT: &str = "iit.fixed_deduction_limit";
    pub const FIXED_DEDUCTION: &str = "iit.fixed_deduction";
    pub const PCT_DEDUCTION: &str = "iit.percent_deduction_rate";
    pub const ITEMS: &str = "surcharge.items";
    pub const BRACKETS: &str = "iit.brackets";
    pub const SCALE: &str = "rounding.scale";
    pub const SEARCH_WINDOW: &str = "reverse.search_window";
    pub const MAX_NET: &str = "reverse.max_net_income";
    pub const GENERAL: &str = "_";
}

/// 校验结果：字段 → 错误说明。
pub type Errors = BTreeMap<String, String>;

fn parse_dec(s: &str) -> Option<Decimal> {
    let c: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ',' | '，' | '%' | '％' | '元'))
        .collect();
    Decimal::from_str(&c).ok()
}

impl SettingsDraft {
    pub fn from_config(cfg: &Config, theme: ThemeChoice) -> Self {
        SettingsDraft {
            section: Section::Vat,
            theme,
            vat_rate: pct_text(cfg.vat.rate),
            threshold_enabled: cfg.vat.threshold_enabled,
            threshold: num_text(cfg.vat.threshold),
            levy_at_threshold: cfg.vat.levy_at_threshold,
            surcharge_base: cfg.surcharge.base,
            items: cfg
                .surcharge
                .items
                .iter()
                .map(|i| ItemDraft {
                    name: i.name.clone(),
                    rate: pct_text(i.rate),
                    enabled: i.enabled,
                })
                .collect(),
            fixed_limit: num_text(cfg.iit.fixed_deduction_limit),
            fixed_deduction: num_text(cfg.iit.fixed_deduction),
            pct_deduction: pct_text(cfg.iit.percent_deduction_rate),
            brackets: cfg
                .iit
                .brackets
                .iter()
                .map(|b| BracketDraft {
                    upper: b.upper.map(num_text).unwrap_or_default(),
                    rate: pct_text(b.rate),
                    quick: num_text(b.quick_deduction),
                })
                .collect(),
            scale: cfg.rounding.scale.to_string(),
            mode: cfg.rounding.mode,
            surcharge_rounding: cfg.rounding.surcharge,
            prefer: cfg.reverse.prefer,
            search_window: cfg.reverse.search_window.to_string(),
            max_net_income: num_text(cfg.reverse.max_net_income),
            message: None,
        }
    }

    /// 内置默认值的草稿（主题保持当前选择）。
    pub fn defaults(&self) -> Self {
        let mut d = Self::from_config(&Config::default(), ThemeChoice::default());
        d.section = self.section;
        d
    }

    /// 全部恢复为内置默认（含主题）。
    pub fn reset_all(&mut self) {
        let section = self.section;
        *self = Self::from_config(&Config::default(), ThemeChoice::default());
        self.section = section;
    }

    /// 校验并生成配置。
    pub fn build(&self) -> Result<Config, Errors> {
        let mut errs = Errors::new();
        let mut err = |k: &str, m: &str| {
            errs.entry(k.to_string()).or_insert_with(|| m.to_string());
        };
        let rate = |s: &str| parse_dec(s).map(|d| d / Decimal::ONE_HUNDRED);
        let mut cfg = Config::default();

        match rate(&self.vat_rate) {
            Some(r) if r >= Decimal::ZERO && r < Decimal::ONE => cfg.vat.rate = r,
            _ => err(key::VAT_RATE, "请输入 0～100 之间的百分数，如 1"),
        }
        cfg.vat.threshold_enabled = self.threshold_enabled;
        match parse_dec(&self.threshold) {
            Some(t) if t >= Decimal::ZERO => cfg.vat.threshold = t,
            _ => err(key::THRESHOLD, "请输入不小于 0 的金额"),
        }
        cfg.vat.levy_at_threshold = self.levy_at_threshold;
        cfg.surcharge.base = self.surcharge_base;
        cfg.surcharge.items.clear();
        for (i, it) in self.items.iter().enumerate() {
            let k = format!("{}.{i}", key::ITEMS);
            let name = it.name.trim();
            if name.is_empty() {
                err(&k, "项目名称不得为空");
            }
            match rate(&it.rate) {
                Some(r) if r >= Decimal::ZERO && r < Decimal::ONE => cfg
                    .surcharge
                    .items
                    .push(SurchargeItem::new(name, r, it.enabled)),
                _ => err(&k, "费率请输入 0～100 之间的百分数"),
            }
        }
        match parse_dec(&self.fixed_limit) {
            Some(v) if v >= Decimal::ZERO => cfg.iit.fixed_deduction_limit = v,
            _ => err(key::FIXED_LIMIT, "请输入不小于 0 的金额"),
        }
        match parse_dec(&self.fixed_deduction) {
            Some(v) if v >= Decimal::ZERO => cfg.iit.fixed_deduction = v,
            _ => err(key::FIXED_DEDUCTION, "请输入不小于 0 的金额"),
        }
        match rate(&self.pct_deduction) {
            Some(r) if r >= Decimal::ZERO && r < Decimal::ONE => cfg.iit.percent_deduction_rate = r,
            _ => err(key::PCT_DEDUCTION, "请输入 0～100 之间的百分数"),
        }
        cfg.iit.brackets.clear();
        if self.brackets.is_empty() {
            err(key::BRACKETS, "预扣率表至少需要一档");
        }
        let mut prev: Option<Decimal> = None;
        for (i, b) in self.brackets.iter().enumerate() {
            let k = format!("{}.{i}", key::BRACKETS);
            let last = i + 1 == self.brackets.len();
            let upper = if last {
                None
            } else {
                match parse_dec(&b.upper) {
                    Some(u) if u > Decimal::ZERO && prev.is_none_or(|p| u > p) => {
                        prev = Some(u);
                        Some(u)
                    }
                    Some(_) => {
                        err(&k, "上限必须为正，且比上一档大");
                        None
                    }
                    None => {
                        err(&k, "请输入本档上限金额");
                        None
                    }
                }
            };
            let r = rate(&b.rate).filter(|r| *r >= Decimal::ZERO && *r < Decimal::ONE);
            let q = parse_dec(&b.quick).filter(|q| *q >= Decimal::ZERO);
            if r.is_none() {
                err(&k, "预扣率请输入 0～100 之间的百分数");
            }
            if q.is_none() {
                err(&k, "速算扣除数请输入不小于 0 的金额");
            }
            if let (Some(rate), Some(quick_deduction)) = (r, q) {
                cfg.iit.brackets.push(Bracket {
                    upper,
                    rate,
                    quick_deduction,
                });
            }
        }
        match self.scale.trim().parse::<u32>() {
            Ok(s) if s <= 6 => cfg.rounding.scale = s,
            _ => err(key::SCALE, "请输入 0～6 的整数（2 表示到分）"),
        }
        cfg.rounding.mode = self.mode;
        cfg.rounding.surcharge = self.surcharge_rounding;
        cfg.reverse.prefer = self.prefer;
        match self.search_window.trim().parse::<u32>() {
            Ok(w) if (1..=10_000).contains(&w) => cfg.reverse.search_window = w,
            _ => err(key::SEARCH_WINDOW, "请输入 1～10000 的整数"),
        }
        match parse_dec(&self.max_net_income) {
            Some(v) if v > Decimal::ZERO => cfg.reverse.max_net_income = v,
            _ => err(key::MAX_NET, "请输入大于 0 的金额"),
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        if let Err(e) = cfg.validate() {
            errs.insert(key::GENERAL.into(), e.to_string());
            return Err(errs);
        }
        if let Err(e) = Reverser::new(&cfg) {
            errs.insert(key::GENERAL.into(), e.to_string());
            return Err(errs);
        }
        Ok(cfg)
    }

    /// 与内置默认不同的参数项数（不含主题）。
    pub fn modified_count(&self) -> usize {
        let d = self.defaults();
        let same = |a: &str, b: &str| parse_dec(a).is_some() && parse_dec(a) == parse_dec(b);
        [
            same(&self.vat_rate, &d.vat_rate),
            self.threshold_enabled == d.threshold_enabled && same(&self.threshold, &d.threshold),
            self.levy_at_threshold == d.levy_at_threshold,
            self.surcharge_base == d.surcharge_base,
            items_same(&self.items, &d.items),
            same(&self.fixed_limit, &d.fixed_limit),
            same(&self.fixed_deduction, &d.fixed_deduction),
            same(&self.pct_deduction, &d.pct_deduction),
            self.brackets == d.brackets,
            self.scale.trim() == d.scale && self.mode == d.mode,
            self.surcharge_rounding == d.surcharge_rounding,
            self.prefer == d.prefer,
            self.search_window.trim() == d.search_window,
            same(&self.max_net_income, &d.max_net_income),
        ]
        .iter()
        .filter(|s| !**s)
        .count()
    }
}

/// 某个数值字段是否与默认相同（数值比较，"1" 与 "1.0" 相同）。
pub fn same_value(a: &str, b: &str) -> bool {
    a.trim() == b.trim() || (parse_dec(a).is_some() && parse_dec(a) == parse_dec(b))
}

/// 计算参数与内置默认不同的项数（状态栏用）。
pub fn modified_items(cfg: &Config) -> usize {
    SettingsDraft::from_config(cfg, ThemeChoice::default()).modified_count()
}

pub fn rounding_mode_text(m: RoundingMode) -> &'static str {
    match m {
        RoundingMode::HalfUp => "四舍五入",
        RoundingMode::HalfEven => "银行家舍入（五取偶）",
        RoundingMode::Down => "截断（舍去）",
        RoundingMode::Up => "进位",
    }
}

pub fn surcharge_rounding_text(m: SurchargeRounding) -> &'static str {
    match m {
        SurchargeRounding::PerItem => "各项目分别按分舍入后相加",
        SurchargeRounding::Total => "合计后一次舍入",
    }
}

pub fn surcharge_base_text(b: SurchargeBase) -> &'static str {
    match b {
        SurchargeBase::Vat => "实际缴纳的增值税税额",
        SurchargeBase::NetRevenue => "不含增值税收入",
    }
}

pub fn prefer_text(p: Prefer) -> &'static str {
    match p {
        Prefer::MinCost => "取最小不含增值税收入（企业总支出最小）",
        Prefer::MaxCost => "取最大不含增值税收入",
    }
}
