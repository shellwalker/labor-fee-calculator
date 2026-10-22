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
    SurchargeRounding, UrbanRegion, URBAN_CONSTRUCTION_TAX,
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

// ---------------- 设置草稿（设置窗口的编辑状态） ----------------

/// 设置窗口左侧导航的一级分组（标题，不可点击）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// 计税口径：税种、税（费）率、起征点、减除费用、预扣率表等税收政策参数。
    Tax,
    /// 计算规则：舍入与测算求解等计算实现规则。
    Calc,
    /// 通用：外观偏好与设置存储。
    General,
}

impl Group {
    pub const ALL: [Group; 3] = [Group::Tax, Group::Calc, Group::General];

    pub fn title(self) -> &'static str {
        match self {
            Group::Tax => "计税口径",
            Group::Calc => "计算规则",
            Group::General => "通用",
        }
    }

    pub fn sections(self) -> &'static [Section] {
        match self {
            Group::Tax => &[Section::Vat, Section::Surcharge, Section::Iit],
            Group::Calc => &[Section::Rounding, Section::Solving],
            Group::General => &[Section::Appearance, Section::Storage],
        }
    }
}

/// 设置窗口的页（左侧导航的二级项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Vat,
    Surcharge,
    Iit,
    /// 金额舍入：小数位、舍入方式、附加税费舍入方式。
    Rounding,
    /// 测算求解：按税后实发金额测算（含批量）的多解取值与搜索参数。
    Solving,
    Appearance,
    /// 设置存储：保存位置说明。
    Storage,
}

impl Section {
    pub const ALL: [Section; 7] = [
        Section::Vat,
        Section::Surcharge,
        Section::Iit,
        Section::Rounding,
        Section::Solving,
        Section::Appearance,
        Section::Storage,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Vat => "增值税",
            Section::Surcharge => "附加税费",
            Section::Iit => "个人所得税",
            Section::Rounding => "金额舍入",
            Section::Solving => "测算求解",
            Section::Appearance => "外观",
            Section::Storage => "设置存储",
        }
    }

    /// 某个字段标识所在的页。
    pub fn of_key(k: &str) -> Option<Section> {
        Some(if k.starts_with("vat.") {
            Section::Vat
        } else if k.starts_with("surcharge.") {
            Section::Surcharge
        } else if k.starts_with("iit.") {
            Section::Iit
        } else if k.starts_with("rounding.") {
            Section::Rounding
        } else if k.starts_with("reverse.") {
            Section::Solving
        } else if k.starts_with("appearance.") {
            Section::Appearance
        } else {
            return None;
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDraft {
    pub name: String,
    /// 费率（百分数，如 "3"）。
    pub rate: String,
    /// 是否征收。
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
    /// 是否享受附加税费减征优惠（全局，作用于全部征收项目）。
    pub relief_enabled: bool,
    /// 减征比例（百分数，如 "50"）。
    pub relief_rate: String,
    /// 城市维护建设税所选的纳税人所在地档位（None：项目税率不属于任一档位）。
    pub urban_region: Option<UrbanRegion>,
    /// 城市维护建设税各档税率（百分数，顺序同 `UrbanRegion::ALL`：市区、县城和镇、其他）。
    pub urban_rates: [String; 3],
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
}

/// 附加税费项目是否与默认一致：名称、是否征收相同，费率按数值比较（“7”与“7.0”视为相同）。
pub fn item_same(a: &ItemDraft, b: &ItemDraft) -> bool {
    a.name.trim() == b.name.trim()
        && a.enabled == b.enabled
        && parse_dec(&a.rate).is_some()
        && parse_dec(&a.rate) == parse_dec(&b.rate)
}

pub fn items_same(a: &[ItemDraft], b: &[ItemDraft]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| item_same(x, y))
}

pub fn pct_text(d: Decimal) -> String {
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
    pub const ITEMS_REMOVED: &str = "surcharge.items.removed";
    pub const THRESHOLD_ENABLED: &str = "vat.threshold_enabled";
    pub const LEVY_AT: &str = "vat.levy_at_threshold";
    pub const BASE: &str = "surcharge.base";
    /// 减征优惠（是否享受 + 减征比例，一行确认）。
    pub const RELIEF: &str = "surcharge.relief";
    pub const MODE: &str = "rounding.mode";
    pub const SURCHARGE_ROUNDING: &str = "rounding.surcharge";
    pub const PREFER: &str = "reverse.prefer";
    pub const BRACKETS: &str = "iit.brackets";
    pub const SCALE: &str = "rounding.scale";
    pub const SEARCH_WINDOW: &str = "reverse.search_window";
    pub const MAX_NET: &str = "reverse.max_net_income";
    pub const THEME: &str = "appearance.theme";
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
            relief_enabled: cfg.surcharge.relief_enabled,
            relief_rate: pct_text(cfg.surcharge.relief_rate),
            urban_region: cfg.surcharge.urban_construction.region,
            urban_rates: UrbanRegion::ALL
                .map(|r| pct_text(cfg.surcharge.urban_construction.rate(r))),
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
        cfg.surcharge.relief_enabled = self.relief_enabled;
        match rate(&self.relief_rate) {
            Some(r) if r >= Decimal::ZERO && r <= Decimal::ONE => cfg.surcharge.relief_rate = r,
            _ => err(key::RELIEF, "减征比例请输入 0～100 之间的百分数，如 50"),
        }
        cfg.surcharge.items.clear();
        let urban_key = self
            .urban_index()
            .map(|i| format!("{}.{i}", key::ITEMS))
            .unwrap_or_else(|| key::GENERAL.to_string());
        let mut tiers = [Decimal::ZERO; 3];
        for (j, t) in self.urban_rates.iter().enumerate() {
            match rate(t) {
                Some(r) if r >= Decimal::ZERO && r < Decimal::ONE => tiers[j] = r,
                _ => err(
                    &urban_key,
                    &format!(
                        "“{}”的税率请输入 0～100 之间的百分数",
                        UrbanRegion::ALL[j].label()
                    ),
                ),
            }
        }
        cfg.surcharge.urban_construction.region = self.urban_region;
        for (j, r) in UrbanRegion::ALL.into_iter().enumerate() {
            cfg.surcharge.urban_construction.set_rate(r, tiers[j]);
        }
        for (i, it) in self.items.iter().enumerate() {
            let k = format!("{}.{i}", key::ITEMS);
            let name = it.name.trim();
            if name.is_empty() {
                err(&k, "项目名称不得为空");
            }
            if name == URBAN_CONSTRUCTION_TAX {
                if let Some(r) = self.urban_region {
                    // 选定档位：税率取该档税率
                    cfg.surcharge.items.push(SurchargeItem::new(
                        name,
                        tiers[r.index()],
                        it.enabled,
                    ));
                    continue;
                }
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

    /// 显示“已修改”标签的各行（字段标识），与设置界面逐行一致。
    ///
    /// 附加税费项目逐项计：与默认同位置同名的标准项目按“是否征收 + 费率数值”比较，
    /// 自行添加的项目每项计 1；删除了默认项目时另计 1 行（`surcharge.items.removed`）。
    pub fn modified_rows(&self) -> Vec<String> {
        let d = self.defaults();
        let mut v: Vec<String> = Vec::new();
        let mut add = |k: &str, changed: bool| {
            if changed {
                v.push(k.to_string());
            }
        };
        add(key::VAT_RATE, !same_value(&self.vat_rate, &d.vat_rate));
        add(
            key::THRESHOLD_ENABLED,
            self.threshold_enabled != d.threshold_enabled,
        );
        add(key::THRESHOLD, !same_value(&self.threshold, &d.threshold));
        add(key::LEVY_AT, self.levy_at_threshold != d.levy_at_threshold);
        add(key::BASE, self.surcharge_base != d.surcharge_base);
        add(key::RELIEF, !relief_same(self, &d));
        for i in 0..self.items.len() {
            add(&format!("{}.{i}", key::ITEMS), self.item_modified(i));
        }
        add(key::ITEMS_REMOVED, self.standard_items_removed());
        add(
            key::FIXED_LIMIT,
            !same_value(&self.fixed_limit, &d.fixed_limit),
        );
        add(
            key::FIXED_DEDUCTION,
            !same_value(&self.fixed_deduction, &d.fixed_deduction),
        );
        add(
            key::PCT_DEDUCTION,
            !same_value(&self.pct_deduction, &d.pct_deduction),
        );
        add(key::BRACKETS, self.brackets != d.brackets);
        add(key::SCALE, self.scale.trim() != d.scale);
        add(key::MODE, self.mode != d.mode);
        add(
            key::SURCHARGE_ROUNDING,
            self.surcharge_rounding != d.surcharge_rounding,
        );
        add(key::PREFER, self.prefer != d.prefer);
        add(
            key::SEARCH_WINDOW,
            self.search_window.trim() != d.search_window,
        );
        add(
            key::MAX_NET,
            !same_value(&self.max_net_income, &d.max_net_income),
        );
        v
    }

    /// 某一行是否显示“已修改”。
    pub fn is_modified(&self, k: &str) -> bool {
        self.modified_rows().iter().any(|r| r == k)
    }

    /// 城市维护建设税项目的序号。
    pub fn urban_index(&self) -> Option<usize> {
        self.items
            .iter()
            .position(|it| it.name.trim() == URBAN_CONSTRUCTION_TAX)
    }

    /// 城市维护建设税的档位与各档税率是否与 `other` 相同（税率按数值比较）。
    pub fn urban_same(&self, other: &SettingsDraft) -> bool {
        self.urban_region == other.urban_region
            && self
                .urban_rates
                .iter()
                .zip(&other.urban_rates)
                .all(|(a, b)| same_value(a, b))
    }

    /// 第 `i` 个附加税费项目是否与默认不同（自行添加的项目恒为已修改）。
    /// 城市维护建设税另比较所选档位与各档税率。
    pub fn item_modified(&self, i: usize) -> bool {
        let d = self.defaults();
        match (self.items.get(i), self.standard_item(i, &d)) {
            (Some(it), Some(std_item)) => {
                !item_same(it, std_item)
                    || (it.name == URBAN_CONSTRUCTION_TAX && !self.urban_same(&d))
            }
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// 第 `i` 个项目若与默认同位置同名，返回对应的默认项目。
    pub fn standard_item<'a>(&self, i: usize, d: &'a SettingsDraft) -> Option<&'a ItemDraft> {
        d.items
            .get(i)
            .filter(|x| self.items.get(i).is_some_and(|it| it.name == x.name))
    }

    /// 是否删除（或移动）了默认项目。
    pub fn standard_items_removed(&self) -> bool {
        let d = self.defaults();
        (0..d.items.len()).any(|i| self.standard_item(i, &d).is_none())
    }

    /// 与内置默认不同的参数项数（不含主题），等于界面上“已修改”标签的数量。
    pub fn modified_count(&self) -> usize {
        self.modified_rows().len()
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

// ---------------- 逐项确认（编辑态 / 生效值） ----------------

/// 两个附加税费项目的输入是否相同（名称、是否征收、费率数值）。
fn item_input_same(a: &ItemDraft, b: &ItemDraft) -> bool {
    a.name.trim() == b.name.trim() && a.enabled == b.enabled && same_value(&a.rate, &b.rate)
}

/// 减征优惠是否相同：是否享受相同，且减征比例数值相同。
fn relief_same(a: &SettingsDraft, b: &SettingsDraft) -> bool {
    a.relief_enabled == b.relief_enabled && same_value(&a.relief_rate, &b.relief_rate)
}

fn brackets_same(a: &[BracketDraft], b: &[BracketDraft]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).enumerate().all(|(i, (x, y))| {
            (i + 1 == a.len() || same_value(&x.upper, &y.upper))
                && same_value(&x.rate, &y.rate)
                && same_value(&x.quick, &y.quick)
        })
}

/// 附加税费项目的字段标识中的序号（`surcharge.items.3` → 3）。
pub fn item_index(k: &str) -> Option<usize> {
    k.strip_prefix(key::ITEMS)?.strip_prefix('.')?.parse().ok()
}

/// 某个错误字段是否属于设置行 `row`（行本身或其下的子字段，如预扣率表的各档）。
fn key_in_row(err_key: &str, row: &str) -> bool {
    err_key == row
        || err_key
            .strip_prefix(row)
            .is_some_and(|rest| rest.starts_with('.'))
}

impl SettingsDraft {
    /// 全部设置行的字段标识（每行一组 √ ×）。
    pub fn row_keys(&self) -> Vec<String> {
        let mut v: Vec<String> = [
            key::VAT_RATE,
            key::THRESHOLD_ENABLED,
            key::THRESHOLD,
            key::LEVY_AT,
            key::BASE,
            key::RELIEF,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        v.extend((0..self.items.len()).map(|i| format!("{}.{i}", key::ITEMS)));
        v.push(key::ITEMS_REMOVED.to_string());
        v.extend(
            [
                key::FIXED_LIMIT,
                key::FIXED_DEDUCTION,
                key::PCT_DEDUCTION,
                key::BRACKETS,
                key::SCALE,
                key::MODE,
                key::SURCHARGE_ROUNDING,
                key::PREFER,
                key::SEARCH_WINDOW,
                key::MAX_NET,
                key::THEME,
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        v
    }

    /// 设置行 `k` 的输入与 `other` 是否相同（数值按数值比较）。
    pub fn row_same(&self, other: &SettingsDraft, k: &str) -> bool {
        if let Some(i) = item_index(k) {
            return match (self.items.get(i), other.items.get(i)) {
                (Some(a), Some(b)) => {
                    item_input_same(a, b)
                        && (a.name.trim() != URBAN_CONSTRUCTION_TAX || self.urban_same(other))
                }
                (None, None) => true,
                _ => false,
            };
        }
        match k {
            key::VAT_RATE => same_value(&self.vat_rate, &other.vat_rate),
            key::THRESHOLD_ENABLED => self.threshold_enabled == other.threshold_enabled,
            key::THRESHOLD => same_value(&self.threshold, &other.threshold),
            key::LEVY_AT => self.levy_at_threshold == other.levy_at_threshold,
            key::BASE => self.surcharge_base == other.surcharge_base,
            key::RELIEF => relief_same(self, other),
            key::ITEMS_REMOVED => self.standard_items_removed() == other.standard_items_removed(),
            key::FIXED_LIMIT => same_value(&self.fixed_limit, &other.fixed_limit),
            key::FIXED_DEDUCTION => same_value(&self.fixed_deduction, &other.fixed_deduction),
            key::PCT_DEDUCTION => same_value(&self.pct_deduction, &other.pct_deduction),
            key::BRACKETS => brackets_same(&self.brackets, &other.brackets),
            key::SCALE => self.scale.trim() == other.scale.trim(),
            key::MODE => self.mode == other.mode,
            key::SURCHARGE_ROUNDING => self.surcharge_rounding == other.surcharge_rounding,
            key::PREFER => self.prefer == other.prefer,
            key::SEARCH_WINDOW => self.search_window.trim() == other.search_window.trim(),
            key::MAX_NET => same_value(&self.max_net_income, &other.max_net_income),
            key::THEME => self.theme == other.theme,
            _ => true,
        }
    }

    /// 把 `src` 中设置行 `k` 的值复制到本草稿。新增的附加税费项目须按顺序确认。
    pub fn copy_row(&mut self, src: &SettingsDraft, k: &str) -> Result<(), String> {
        if let Some(i) = item_index(k) {
            let Some(it) = src.items.get(i) else {
                return Ok(());
            };
            if it.name.trim() == URBAN_CONSTRUCTION_TAX {
                self.urban_region = src.urban_region;
                self.urban_rates = src.urban_rates.clone();
            }
            return match i.cmp(&self.items.len()) {
                std::cmp::Ordering::Less => {
                    self.items[i] = it.clone();
                    Ok(())
                }
                std::cmp::Ordering::Equal => {
                    self.items.push(it.clone());
                    Ok(())
                }
                std::cmp::Ordering::Greater => Err("请先确认上方新增的项目".into()),
            };
        }
        match k {
            key::VAT_RATE => self.vat_rate = src.vat_rate.clone(),
            key::THRESHOLD_ENABLED => self.threshold_enabled = src.threshold_enabled,
            key::THRESHOLD => self.threshold = src.threshold.clone(),
            key::LEVY_AT => self.levy_at_threshold = src.levy_at_threshold,
            key::BASE => self.surcharge_base = src.surcharge_base,
            key::RELIEF => {
                self.relief_enabled = src.relief_enabled;
                self.relief_rate = src.relief_rate.clone();
            }
            key::ITEMS_REMOVED => {
                self.items = src.items.clone();
                self.urban_region = src.urban_region;
                self.urban_rates = src.urban_rates.clone();
            }
            key::FIXED_LIMIT => self.fixed_limit = src.fixed_limit.clone(),
            key::FIXED_DEDUCTION => self.fixed_deduction = src.fixed_deduction.clone(),
            key::PCT_DEDUCTION => self.pct_deduction = src.pct_deduction.clone(),
            key::BRACKETS => self.brackets = src.brackets.clone(),
            key::SCALE => self.scale = src.scale.clone(),
            key::MODE => self.mode = src.mode,
            key::SURCHARGE_ROUNDING => self.surcharge_rounding = src.surcharge_rounding,
            key::PREFER => self.prefer = src.prefer,
            key::SEARCH_WINDOW => self.search_window = src.search_window.clone(),
            key::MAX_NET => self.max_net_income = src.max_net_income.clone(),
            key::THEME => self.theme = src.theme,
            _ => {}
        }
        Ok(())
    }
}

// ---------------- 附加税费合计比例 ----------------

/// 附加税费合计比例中的一项（征收项目）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryLine {
    pub name: String,
    /// 法定费率（税率）。
    pub statutory: Decimal,
    /// 减征比例（未享受减征为 0）。
    pub relief: Decimal,
    /// 实际征收率 = 法定费率 × (1 − 减征比例)。
    pub actual: Decimal,
}

/// 附加税费合计比例（占计税依据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurchargeSummary {
    pub lines: Vec<SummaryLine>,
    /// 减征前合计（各征收项目法定费率之和）。
    pub statutory_total: Decimal,
    /// 减征后合计（实际征收率之和）。
    pub actual_total: Decimal,
    /// 适用的减征比例（未享受减征为 0）。
    pub relief: Decimal,
}

/// 城市维护建设税税率对应的纳税人所在地（`URBAN_CONSTRUCTION_RATES` 的序号）；非法定税率返回 None。
pub fn urban_region(rate: &str) -> Option<usize> {
    let r = parse_dec(rate)? / Decimal::ONE_HUNDRED;
    labor_fee_core::URBAN_CONSTRUCTION_RATES
        .iter()
        .position(|(_, x)| *x == r)
}

impl SettingsDraft {
    /// 按本草稿计算附加税费合计比例；费率或减征比例无法解析时返回 None。
    pub fn surcharge_summary(&self) -> Option<SurchargeSummary> {
        let mut sc = Config::default().surcharge;
        sc.relief_enabled = self.relief_enabled;
        sc.relief_rate = parse_dec(&self.relief_rate)? / Decimal::ONE_HUNDRED;
        if sc.relief_rate < Decimal::ZERO || sc.relief_rate > Decimal::ONE {
            return None;
        }
        sc.items.clear();
        for it in &self.items {
            let r = match self.urban_region {
                Some(region) if it.name.trim() == URBAN_CONSTRUCTION_TAX => {
                    parse_dec(&self.urban_rates[region.index()])?
                }
                _ => parse_dec(&it.rate)?,
            } / Decimal::ONE_HUNDRED;
            sc.items
                .push(SurchargeItem::new(it.name.trim(), r, it.enabled));
        }
        Some(SurchargeSummary {
            lines: sc
                .enabled_items()
                .map(|i| SummaryLine {
                    name: i.name.clone(),
                    statutory: i.rate,
                    relief: sc.applied_relief(),
                    actual: sc.actual_rate(i.rate),
                })
                .collect(),
            statutory_total: sc.statutory_total_rate(),
            actual_total: sc.actual_total_rate(),
            relief: sc.applied_relief(),
        })
    }
}

/// 设置窗口上的二次确认对话框。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    /// 全部恢复默认（危险操作）。
    ResetAll,
    /// 关闭窗口时存在尚未生效的修改。
    DiscardOnClose,
}

/// 打开中的设置窗口：编辑态（界面上的输入）与生效值（已应用并保存）。
///
/// 每一行的输入与生效值不同时处于“未生效”状态，点击 √（或 Enter）生效并立即保存，
/// 点击 ×（或 Esc）放弃本项修改、恢复为生效值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsWindow {
    /// 界面上的输入（含当前分页 `section`）。
    pub edit: SettingsDraft,
    /// 当前生效的设置（与主界面计算所用参数一致）。
    pub applied: SettingsDraft,
    /// 打开中的二次确认对话框。
    pub dialog: Option<Dialog>,
    /// 保存失败等提示。
    pub message: Option<String>,
}

impl SettingsWindow {
    pub fn new(cfg: &Config, theme: ThemeChoice) -> Self {
        Self::from_draft(SettingsDraft::from_config(cfg, theme))
    }

    /// 以给定草稿同时作为输入与生效值（测试、截图用）。
    pub fn from_draft(d: SettingsDraft) -> Self {
        SettingsWindow {
            edit: d.clone(),
            applied: d,
            dialog: None,
            message: None,
        }
    }

    /// 输入与生效值不同（尚未生效）的设置行。
    pub fn pending_rows(&self) -> Vec<String> {
        self.edit
            .row_keys()
            .into_iter()
            .filter(|k| !self.edit.row_same(&self.applied, k))
            .collect()
    }

    pub fn is_pending(&self, k: &str) -> bool {
        !self.edit.row_same(&self.applied, k)
    }

    pub fn pending_count(&self) -> usize {
        self.pending_rows().len()
    }

    /// 某页尚未生效的行数。
    pub fn section_pending(&self, s: Section) -> usize {
        self.pending_rows()
            .iter()
            .filter(|k| Section::of_key(k) == Some(s))
            .count()
    }

    /// 某页生效值与内置默认不同（标“已修改”）的行数。
    pub fn section_modified(&self, s: Section) -> usize {
        self.applied
            .modified_rows()
            .iter()
            .filter(|k| Section::of_key(k) == Some(s))
            .count()
    }

    /// 生效值加上设置行 `k` 的输入：确认该行后的完整设置。
    pub fn candidate(&self, k: &str) -> Result<SettingsDraft, String> {
        let mut c = self.applied.clone();
        c.copy_row(&self.edit, k)?;
        Ok(c)
    }

    /// 各未生效行确认后的校验错误（字段标识 → 说明）；有错误的行 √ 不可用。
    pub fn row_errors(&self) -> Errors {
        let mut out = Errors::new();
        for k in self.pending_rows() {
            if k == key::THEME {
                continue;
            }
            match self.candidate(&k) {
                Err(e) => {
                    out.insert(k, e);
                }
                Ok(c) => {
                    if let Err(errs) = c.build() {
                        for (ek, m) in errs {
                            if key_in_row(&ek, &k) {
                                out.entry(ek).or_insert(m);
                            } else {
                                // 与其他参数组合后不成立（如测算搜索上限与预扣率表）：记在本行
                                out.entry(k.clone()).or_insert(m);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// 设置行 `k` 不能生效的原因（无则可以确认）。
    pub fn row_error(&self, k: &str) -> Option<String> {
        let errs = self.row_errors();
        errs.iter()
            .find(|(ek, _)| key_in_row(ek, k))
            .map(|(_, m)| m.clone())
    }

    /// 放弃设置行 `k` 的修改：恢复为生效值；尚未生效的新增项目直接移除。
    pub fn discard(&mut self, k: &str) {
        if let Some(i) = item_index(k) {
            if i >= self.applied.items.len() {
                if i < self.edit.items.len() {
                    self.edit.items.remove(i);
                }
                return;
            }
        }
        let applied = self.applied.clone();
        let _ = self.edit.copy_row(&applied, k);
    }

    /// 放弃全部未生效的修改。
    pub fn discard_all(&mut self) {
        let section = self.edit.section;
        self.edit = self.applied.clone();
        self.edit.section = section;
    }

    /// 生效值更新后同步：设置行 `k` 的输入改为新的生效值（规范写法），其他行的未生效输入保留。
    pub fn set_applied(&mut self, applied: SettingsDraft, k: &str) {
        self.applied = applied;
        self.applied.section = self.edit.section;
        let a = self.applied.clone();
        let _ = self.edit.copy_row(&a, k);
    }
}
