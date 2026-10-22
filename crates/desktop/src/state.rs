//! 桌面程序的状态与操作逻辑（与界面绘制分离，便于测试）。所有计算都调用 labor-fee-core / labor-fee-batch。

use crate::settings::{
    item_index, key, Dialog, Prefs, SettingsDraft, SettingsStore, SettingsWindow, ThemeChoice,
};
use labor_fee_batch::{InputKind, Output};
use labor_fee_core::{
    forward, from_total_cost, Breakdown, CalcError, Config, CostGap, CostGapReason, Decimal,
    ReverseOutcome, Reverser,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::mpsc::{Receiver, TryRecvError};

/// 批量结果预览的行数。
pub const PREVIEW_ROWS: usize = 50;

/// 页签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Forward,
    Reverse,
    FromCost,
    Batch,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Forward, Tab::Reverse, Tab::FromCost, Tab::Batch];

    /// 导航中的短标题。
    pub fn title(self) -> &'static str {
        match self {
            Tab::Forward => "收入计算",
            Tab::Reverse => "实发测算",
            Tab::FromCost => "支出测算",
            Tab::Batch => "批量测算",
        }
    }

    /// 页面标题（全称）。
    pub fn full_title(self) -> &'static str {
        match self {
            Tab::Forward => "按不含增值税收入计算",
            Tab::Reverse => "按税后实发金额测算",
            Tab::FromCost => "按企业总支出测算",
            Tab::Batch => "批量测算",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tab::Forward => crate::icons::FORWARD,
            Tab::Reverse => crate::icons::REVERSE,
            Tab::FromCost => crate::icons::COST,
            Tab::Batch => crate::icons::BATCH,
        }
    }

    /// 页面标题下的一句话说明。
    pub fn description(self) -> &'static str {
        match self {
            Tab::Forward => {
                "以不含增值税收入为准，计算增值税及附加税费、企业总支出、个人所得税与税后实发金额"
            }
            Tab::Reverse => "根据税后实发金额，测算对应的不含增值税收入及各项金额",
            Tab::FromCost => "根据企业总支出，测算对应的不含增值税收入；金额不可达时说明原因",
            Tab::Batch => "读取表格中的金额列，逐行测算并追加结果列，导出为新文件",
        }
    }
}

/// 菜单 / 快捷键触发的操作（由界面在本帧末尾执行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    OpenBatchFile,
    Settings,
    Help,
    About,
    /// 关闭窗口（Cmd/Ctrl+W）：先关闭最上层的子窗口，没有子窗口时最小化主窗口。
    CloseWindow,
    /// 最小化主窗口（macOS Cmd+M）。
    Minimize,
    Quit,
}

/// “关闭窗口”作用的对象。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseTarget {
    /// 关闭设置窗口（有未生效的修改时弹出放弃确认；确认框已打开时返回设置窗口）。
    Settings,
    About,
    Help,
    /// 没有子窗口：最小化主窗口（不退出程序）。
    MainWindow,
}

/// 状态行的级别（决定颜色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Error,
}

/// 状态说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub level: Level,
    pub text: String,
}

impl Status {
    pub fn ok(text: impl Into<String>) -> Self {
        Status {
            level: Level::Ok,
            text: text.into(),
        }
    }
    pub fn warn(text: impl Into<String>) -> Self {
        Status {
            level: Level::Warn,
            text: text.into(),
        }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Status {
            level: Level::Error,
            text: text.into(),
        }
    }
}

/// 结果表的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub value: String,
}

/// 单笔计算结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// 尚未计算。
    None,
    /// 计算成功：全部字段 + 校验状态。
    Done {
        fields: Vec<Field>,
        status: Status,
        breakdown: Box<Breakdown>,
    },
    /// 企业总成本落在空位：原因及两侧最近的有效值。
    Gap(Box<CostGap>),
    /// 输入或计算错误。
    Error(String),
}

/// 最近计算记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub input: Decimal,
    pub net_income: Decimal,
    pub total_cost: Decimal,
    pub take_home: Decimal,
}

/// 最近计算记录的条数上限。
pub const HISTORY_LEN: usize = 6;

/// 单笔计算页签（正算 / 反算 / 按总成本）。
#[derive(Debug, Clone)]
pub struct SingleTab {
    pub input: String,
    pub outcome: Outcome,
    pub history: Vec<HistoryEntry>,
}

impl Default for SingleTab {
    fn default() -> Self {
        SingleTab {
            input: String::new(),
            outcome: Outcome::None,
            history: Vec::new(),
        }
    }
}

impl SingleTab {
    fn record(&mut self) {
        let (Ok(input), Outcome::Done { breakdown: b, .. }) =
            (parse_amount(&self.input), &self.outcome)
        else {
            return;
        };
        let e = HistoryEntry {
            input,
            net_income: b.net_income,
            total_cost: b.total_cost,
            take_home: b.take_home,
        };
        self.history.retain(|h| h.input != e.input);
        self.history.insert(0, e);
        self.history.truncate(HISTORY_LEN);
    }
}

/// 批量页签。
#[derive(Default)]
pub struct BatchTab {
    /// 输入文件路径（可手工输入，也可用“选择文件…”）。
    pub path: String,
    pub kind: InputKind,
    /// 列标题，空则用输入列类型的默认标题。
    pub column: String,
    pub result: Option<BatchResult>,
    pub message: Option<Status>,
    running: Option<Receiver<Result<BatchResult, String>>>,
}

/// 一次批量计算的结果。
pub struct BatchResult {
    pub input: PathBuf,
    pub kind: InputKind,
    pub output: Output,
}

impl BatchResult {
    /// 统计说明。
    pub fn summary_text(&self) -> String {
        let s = &self.output.summary;
        let mut t = format!("共 {} 行：成功 {}", s.rows, s.exact);
        if self.kind == InputKind::TakeHome {
            t += &format!("，无法精确匹配 {}", s.inexact);
        }
        if self.kind == InputKind::TotalCost {
            t += &format!("，总支出不可达 {}", s.unreachable_cost);
        }
        t += &format!("，数据错误 {}，空白 {}", s.errors, s.empty);
        t
    }

    /// 默认输出文件名（与 CLI 一致：原名_反算结果 / 原名_计算结果）。
    pub fn default_file_name(&self) -> String {
        let name = self.input.to_string_lossy();
        labor_fee_batch::output_file_name_for(&name, self.kind)
            .unwrap_or_else(|_| format!("结果.{}", self.output.format.output_extension()))
    }

    /// 默认保存位置：输入文件同目录。
    pub fn default_save_path(&self) -> PathBuf {
        self.input.with_file_name(self.default_file_name())
    }
}

/// 口径来源说明：内置默认。
pub const SOURCE_DEFAULT: &str = "内置默认参数";
/// 口径来源说明：设置窗口保存的口径。
pub const SOURCE_SAVED: &str = "已保存的设置";

/// 整个程序的状态。
pub struct AppState {
    pub tab: Tab,
    cfg: Config,
    reverser: Reverser,
    pub forward: SingleTab,
    pub reverse: SingleTab,
    pub from_cost: SingleTab,
    pub batch: BatchTab,
    /// 当前口径来源说明。
    pub source: String,
    /// 顶部提示（如启动时加载设置失败）。
    pub notice: Option<Status>,
    pub prefs: Prefs,
    /// 设置的保存位置；None 表示不持久化（测试或取不到配置目录）。
    pub store: Option<SettingsStore>,
    /// 打开中的设置窗口。
    pub settings: Option<SettingsWindow>,
    pub show_about: bool,
    pub show_help: bool,
    /// 是否展开试算规则面板。
    pub show_rules: bool,
    /// 待执行的菜单操作。
    pub actions: Vec<Action>,
    /// 是否使用原生菜单（macOS）；为 true 时不绘制窗口内菜单栏、不处理菜单快捷键。
    pub native_menu: bool,
}

/// 解析金额输入：允许空白、千分位逗号、¥、元。
pub fn parse_amount(s: &str) -> Result<Decimal, String> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ',' | '，' | '¥' | '￥' | '元'))
        .collect();
    if cleaned.is_empty() {
        return Err("请输入金额".into());
    }
    Decimal::from_str(&cleaned).map_err(|_| format!("无法解析金额：{}", s.trim()))
}

fn pct(d: Decimal) -> String {
    format!("{}%", (d * Decimal::ONE_HUNDRED).normalize())
}

/// 结果表字段（三个单笔页签共用）。
pub fn fields(b: &Breakdown) -> Vec<Field> {
    let f = |label: &str, value: String| Field {
        label: label.to_string(),
        value,
    };
    let mut v = vec![
        f("不含增值税收入", b.net_income.to_string()),
        f("增值税税额", b.vat.to_string()),
    ];
    for l in &b.surcharges {
        v.push(f(
            &format!("  {}（{}）", l.name, pct(l.rate)),
            l.amount.to_string(),
        ));
    }
    v.push(f("增值税附加税费合计", b.surcharge_total.to_string()));
    v.extend([
        f("企业总支出", b.total_cost.to_string()),
        f("减除费用", b.iit.deduction.to_string()),
        f("预扣预缴应纳税所得额", b.iit.taxable_income.to_string()),
        f("预扣率", pct(b.iit.rate)),
        f("速算扣除数", b.iit.quick_deduction.to_string()),
        f("应预扣预缴税额", b.iit.tax.to_string()),
        f("税后实发金额", b.take_home.to_string()),
    ]);
    v
}

/// 正算恒等式自检：总成本 = 三项之和，到手 = 收入 − 个税。
fn identity_ok(b: &Breakdown) -> bool {
    b.total_cost == b.net_income + b.vat + b.surcharge_total
        && b.take_home == b.net_income - b.iit.tax
}

fn exempt_note(b: &Breakdown) -> &'static str {
    if b.vat_exempt {
        "；未达到增值税起征点，不征收增值税及附加税费"
    } else {
        ""
    }
}

/// 空位原因的简短说明。
pub fn gap_reason_text(g: &CostGap) -> String {
    match g.reason {
        CostGapReason::Threshold => format!(
            "原因：增值税起征点（{} 元）形成的不可达区间",
            g.threshold
                .map(|t| t.normalize().to_string())
                .unwrap_or_default()
        ),
        CostGapReason::Rounding => "原因：增值税税额及附加税费按分舍入".to_string(),
    }
}

impl AppState {
    /// 以给定口径创建；`source` 为口径来源说明。
    pub fn new(cfg: Config, source: impl Into<String>) -> Result<Self, CalcError> {
        let reverser = Reverser::new(&cfg)?;
        Ok(AppState {
            tab: Tab::Forward,
            cfg,
            reverser,
            forward: SingleTab::default(),
            reverse: SingleTab::default(),
            from_cost: SingleTab::default(),
            batch: BatchTab::default(),
            source: source.into(),
            notice: None,
            prefs: Prefs::default(),
            store: None,
            settings: None,
            show_about: false,
            show_help: false,
            show_rules: true,
            actions: Vec::new(),
            native_menu: false,
        })
    }

    /// 内置默认口径（不持久化）。
    pub fn with_default_config() -> Self {
        Self::new(Config::default(), SOURCE_DEFAULT).expect("默认配置有效")
    }

    /// 从设置目录读取主题与口径（启动时用）。已保存的口径无效时改用内置默认并提示。
    pub fn with_store(store: SettingsStore) -> Self {
        let mut st = Self::with_default_config();
        st.prefs = store.load_prefs();
        match store.load_rules() {
            Ok(None) => {}
            Ok(Some(cfg)) => {
                if let Err(e) = st.set_config(cfg, SOURCE_SAVED) {
                    st.notice = Some(Status::error(format!(
                        "已保存的设置无法使用，已改用内置默认参数：{e}"
                    )));
                }
            }
            Err(e) => st.notice = Some(Status::error(e)),
        }
        st.store = Some(store);
        st
    }

    /// 当前口径是否就是内置默认。
    pub fn is_default_config(&self) -> bool {
        self.cfg == Config::default()
    }

    /// 打开设置窗口（以当前口径为初值）。
    pub fn open_settings(&mut self) {
        if self.settings.is_none() {
            self.settings = Some(SettingsWindow::new(&self.cfg, self.prefs.theme));
        }
    }

    /// 请求关闭设置窗口：有未生效的修改时先弹出确认，返回是否已关闭。
    pub fn request_close_settings(&mut self) -> bool {
        match &mut self.settings {
            None => true,
            Some(w) if w.pending_count() > 0 => {
                w.dialog = Some(Dialog::DiscardOnClose);
                false
            }
            Some(_) => {
                self.settings = None;
                true
            }
        }
    }

    /// “关闭窗口”（Cmd/Ctrl+W）此刻作用的对象。
    pub fn close_target(&self) -> CloseTarget {
        if self.settings.is_some() {
            CloseTarget::Settings
        } else if self.show_about {
            CloseTarget::About
        } else if self.show_help {
            CloseTarget::Help
        } else {
            CloseTarget::MainWindow
        }
    }

    /// 执行“关闭窗口”：关闭最上层的子窗口并返回 true；没有子窗口时返回 false（由界面最小化主窗口）。
    pub fn close_front_window(&mut self) -> bool {
        match self.close_target() {
            CloseTarget::Settings => {
                match self.settings.as_mut().and_then(|w| w.dialog.take()) {
                    // 确认框已打开：关闭确认框，回到设置窗口
                    Some(_) => {}
                    None => {
                        self.request_close_settings();
                    }
                }
                true
            }
            CloseTarget::About => {
                self.show_about = false;
                true
            }
            CloseTarget::Help => {
                self.show_help = false;
                true
            }
            CloseTarget::MainWindow => false,
        }
    }

    /// 关闭设置窗口并放弃未生效的修改（已生效的设置不受影响）。
    pub fn close_settings(&mut self) {
        self.settings = None;
    }

    /// 应用一份完整的设置草稿：校验、写入 rules.toml / preferences.toml，并让主界面立即采用。
    fn apply_draft(&mut self, draft: &SettingsDraft) -> Result<(), String> {
        let cfg = draft.build().map_err(|errs| {
            errs.into_values()
                .next()
                .unwrap_or_else(|| "设置无效".into())
        })?;
        let theme = draft.theme;
        if let Some(store) = &self.store {
            store.save_rules(&cfg)?;
            store.save_prefs(&Prefs { theme })?;
        }
        let source = if cfg == Config::default() {
            SOURCE_DEFAULT
        } else {
            SOURCE_SAVED
        };
        if cfg != self.cfg || self.source != source {
            self.set_config(cfg, source).map_err(|e| e.to_string())?;
        }
        self.prefs.theme = theme;
        self.notice = None;
        Ok(())
    }

    /// 以 `candidate` 为新的生效值并同步设置行 `k`；失败时在窗口内提示。
    fn commit(&mut self, candidate: SettingsDraft, k: &str) -> bool {
        match self.apply_draft(&candidate) {
            Ok(()) => {
                let applied = SettingsDraft::from_config(&self.cfg, self.prefs.theme);
                if let Some(w) = &mut self.settings {
                    w.set_applied(applied, k);
                    w.message = None;
                }
                true
            }
            Err(e) => {
                if let Some(w) = &mut self.settings {
                    w.message = Some(e);
                }
                false
            }
        }
    }

    /// 确认设置行 `k`（√ / Enter）：校验通过则立即生效并保存，返回 true。
    pub fn confirm_setting(&mut self, k: &str) -> bool {
        let Some(w) = &self.settings else {
            return false;
        };
        if !w.is_pending(k) {
            return true;
        }
        if let Some(e) = w.row_error(k) {
            if let Some(w) = &mut self.settings {
                w.message = Some(e);
            }
            return false;
        }
        match w.candidate(k) {
            Ok(c) => self.commit(c, k),
            Err(e) => {
                if let Some(w) = &mut self.settings {
                    w.message = Some(e);
                }
                false
            }
        }
    }

    /// 放弃设置行 `k` 的修改（× / Esc）。
    pub fn discard_setting(&mut self, k: &str) {
        if let Some(w) = &mut self.settings {
            w.discard(k);
            w.message = None;
        }
    }

    /// 单项恢复默认：内置默认值总是有效，直接生效并保存（同时放弃本项未生效的输入）。
    pub fn reset_setting(&mut self, k: &str) -> bool {
        let Some(w) = &self.settings else {
            return false;
        };
        let def = w.applied.defaults();
        let mut c = w.applied.clone();
        if let Some(i) = item_index(k) {
            match w.applied.standard_item(i, &def) {
                Some(it) => c.items[i] = it.clone(),
                None => return false,
            }
        } else {
            let _ = c.copy_row(&def, k);
        }
        let ok = self.commit(c, k);
        if ok && k == key::ITEMS_REMOVED {
            if let Some(w) = &mut self.settings {
                w.edit.items = w.applied.items.clone();
            }
        }
        ok
    }

    /// 删除附加税费项目：尚未生效的新增项目直接移除；已生效的自定义项目删除后立即生效。
    pub fn delete_item(&mut self, i: usize) -> bool {
        let Some(w) = &mut self.settings else {
            return false;
        };
        if i >= w.applied.items.len() {
            if i < w.edit.items.len() {
                w.edit.items.remove(i);
            }
            return true;
        }
        let mut c = w.applied.clone();
        c.items.remove(i);
        let ok = self.apply_draft(&c);
        match ok {
            Ok(()) => {
                let applied = SettingsDraft::from_config(&self.cfg, self.prefs.theme);
                if let Some(w) = &mut self.settings {
                    w.edit.items.remove(i);
                    w.applied = applied;
                    w.applied.section = w.edit.section;
                    w.message = None;
                }
                true
            }
            Err(e) => {
                if let Some(w) = &mut self.settings {
                    w.message = Some(e);
                }
                false
            }
        }
    }

    /// 全部恢复默认：计算参数与外观偏好恢复为内置默认并立即生效，
    /// rules.toml 删除（即内置默认）、preferences.toml 写为默认。未生效的输入一并放弃。
    pub fn reset_all_settings(&mut self) -> bool {
        let section = self.settings.as_ref().map(|w| w.edit.section);
        let theme = ThemeChoice::default();
        let saved = self.store.as_ref().map_or(Ok(()), |store| {
            store
                .save_rules(&Config::default())
                .and_then(|_| store.save_prefs(&Prefs { theme }))
        });
        if let Err(e) = saved {
            if let Some(w) = &mut self.settings {
                w.message = Some(e);
                w.dialog = None;
            }
            return false;
        }
        self.set_config(Config::default(), SOURCE_DEFAULT)
            .expect("默认配置有效");
        self.prefs.theme = theme;
        self.notice = None;
        if let Some(w) = &mut self.settings {
            *w = SettingsWindow::new(&self.cfg, theme);
            if let Some(s) = section {
                w.edit.section = s;
                w.applied.section = s;
            }
        }
        true
    }

    /// 顶栏快捷切换主题（立即保存）。
    pub fn set_theme(&mut self, theme: ThemeChoice) {
        self.prefs.theme = theme;
        if let Some(store) = &self.store {
            if let Err(e) = store.save_prefs(&self.prefs) {
                self.notice = Some(Status::error(e));
            }
        }
    }

    /// 当前应显示的主题（设置窗口打开时预览输入中的选择，确认后才保存）。
    pub fn effective_theme(&self) -> ThemeChoice {
        self.settings
            .as_ref()
            .map_or(self.prefs.theme, |w| w.edit.theme)
    }

    /// 已打开的批量文件（菜单“打开批量文件…”、拖放）：切到批量页并填入路径。
    pub fn open_batch_file(&mut self, path: &Path) {
        self.tab = Tab::Batch;
        self.batch.path = path.display().to_string();
        self.batch.result = None;
        self.batch.message = None;
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// 正算：不含税收入 → 全部金额。
    pub fn calc_forward(&mut self) {
        self.forward.outcome = match parse_amount(&self.forward.input)
            .and_then(|a| forward(&self.cfg, a).map_err(|e| e.to_string()))
        {
            Ok(b) => {
                let status = if identity_ok(&b) {
                    Status::ok(format!(
                        "校验通过：企业总支出 = 不含增值税收入 + 增值税税额 + 附加税费，税后实发金额 = 不含增值税收入 − 应预扣预缴税额{}",
                        exempt_note(&b)
                    ))
                } else {
                    Status::error("校验未通过：金额勾稽关系不成立")
                };
                Outcome::Done {
                    fields: fields(&b),
                    status,
                    breakdown: Box::new(b),
                }
            }
            Err(e) => Outcome::Error(e),
        };
        self.forward.record();
    }

    /// 反算：实际到手 → 不含税收入 → 全部金额。
    pub fn calc_reverse(&mut self) {
        self.reverse.outcome = match parse_amount(&self.reverse.input)
            .and_then(|a| self.reverser.reverse(a).map_err(|e| e.to_string()))
        {
            Ok(r) => Outcome::Done {
                fields: fields(&r.breakdown),
                status: reverse_status(&self.cfg, &r),
                breakdown: Box::new(r.breakdown),
            },
            Err(e) => Outcome::Error(e),
        };
        self.reverse.record();
    }

    /// 按企业总成本正算；空位时给出原因和两侧最近的有效总成本。
    pub fn calc_from_cost(&mut self) {
        self.from_cost.outcome = match parse_amount(&self.from_cost.input) {
            Err(e) => Outcome::Error(e),
            Ok(a) => match from_total_cost(&self.cfg, a) {
                Ok(b) => Outcome::Done {
                    status: if identity_ok(&b) && b.total_cost == a {
                        Status::ok(format!(
                            "校验通过：不含增值税收入 {} 对应的企业总支出与输入一致{}",
                            b.net_income,
                            exempt_note(&b)
                        ))
                    } else {
                        Status::error("校验未通过")
                    },
                    fields: fields(&b),
                    breakdown: Box::new(b),
                },
                Err(CalcError::CostUnreachable(g)) => Outcome::Gap(g),
                Err(e) => Outcome::Error(e.to_string()),
            },
        };
        self.from_cost.record();
    }

    /// 点击空位两侧的有效值：填入并重算。
    pub fn use_cost(&mut self, cost: Decimal) {
        self.from_cost.input = cost.to_string();
        self.calc_from_cost();
    }

    /// 更换口径，并重算已有结果。
    pub fn set_config(&mut self, cfg: Config, source: impl Into<String>) -> Result<(), CalcError> {
        let reverser = Reverser::new(&cfg)?;
        self.source = source.into();
        self.cfg = cfg;
        self.reverser = reverser;
        if self.forward.outcome != Outcome::None {
            self.calc_forward();
        }
        if self.reverse.outcome != Outcome::None {
            self.calc_reverse();
        }
        if self.from_cost.outcome != Outcome::None {
            self.calc_from_cost();
        }
        self.batch.result = None;
        self.batch.message = None;
        Ok(())
    }

    /// 从 TOML 文本加载口径。
    pub fn load_config_str(&mut self, text: &str, source: impl Into<String>) -> Result<(), String> {
        let cfg = Config::from_toml_str(text).map_err(|e| e.to_string())?;
        self.set_config(cfg, source).map_err(|e| e.to_string())
    }

    /// 从文件加载口径（命令行 `--config`，只对本次运行生效，不写入设置），结果写入顶部提示。
    pub fn load_config_file(&mut self, path: &Path) {
        let r = std::fs::read_to_string(path)
            .map_err(|e| format!("读取失败：{e}"))
            .and_then(|t| self.load_config_str(&t, format!("文件 {}", path.display())));
        self.notice = Some(match r {
            Ok(()) => Status::ok(format!(
                "本次运行使用参数文件：{}（不写入设置）",
                path.display()
            )),
            Err(e) => Status::error(format!("参数文件加载失败（仍使用原参数）：{e}")),
        });
    }

    /// 恢复内置默认口径（不改动已保存的设置）。
    pub fn reset_config(&mut self) {
        self.set_config(Config::default(), SOURCE_DEFAULT)
            .expect("默认配置有效");
        self.notice = Some(Status::ok("已恢复内置默认参数"));
    }

    fn batch_job(
        &self,
    ) -> Result<impl FnOnce() -> Result<BatchResult, String> + Send + 'static, String> {
        let path = PathBuf::from(self.batch.path.trim());
        if self.batch.path.trim().is_empty() {
            return Err("请先选择文件".into());
        }
        let name = path.to_string_lossy().to_string();
        let format = labor_fee_batch::Format::from_file_name(&name).map_err(|e| e.to_string())?;
        let cfg = self.cfg.clone();
        let kind = self.batch.kind;
        let column = Some(self.batch.column.trim().to_string()).filter(|c| !c.is_empty());
        Ok(move || {
            let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败：{e}"))?;
            let opts = labor_fee_batch::Options {
                kind,
                column,
                preview_rows: PREVIEW_ROWS,
            };
            let output =
                labor_fee_batch::process(&cfg, &bytes, format, &opts).map_err(|e| e.to_string())?;
            Ok(BatchResult {
                input: path,
                kind,
                output,
            })
        })
    }

    fn finish_batch(&mut self, r: Result<BatchResult, String>) {
        match r {
            Ok(res) => {
                self.batch.message = Some(Status::ok(res.summary_text()));
                self.batch.result = Some(res);
            }
            Err(e) => {
                self.batch.result = None;
                self.batch.message = Some(Status::error(e));
            }
        }
    }

    /// 同步执行批量计算（测试与小文件用）。
    pub fn run_batch_blocking(&mut self) {
        let r = self.batch_job().and_then(|job| job());
        self.finish_batch(r);
    }

    /// 在后台线程执行批量计算；`on_done` 在完成时调用（用于请求界面重绘）。
    pub fn start_batch(&mut self, on_done: impl FnOnce() + Send + 'static) {
        if self.batch.running.is_some() {
            return;
        }
        match self.batch_job() {
            Err(e) => self.finish_batch(Err(e)),
            Ok(job) => {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(job());
                    on_done();
                });
                self.batch.result = None;
                self.batch.message = None;
                self.batch.running = Some(rx);
            }
        }
    }

    /// 是否有批量计算在进行。
    pub fn batch_running(&self) -> bool {
        self.batch.running.is_some()
    }

    /// 检查后台批量计算是否完成。
    pub fn poll_batch(&mut self) {
        let Some(rx) = &self.batch.running else {
            return;
        };
        match rx.try_recv() {
            Ok(r) => {
                self.batch.running = None;
                self.finish_batch(r);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.batch.running = None;
                self.finish_batch(Err("批量测算意外中止".into()));
            }
        }
    }

    /// 保存批量结果到指定路径。
    pub fn save_batch(&mut self, path: &Path) {
        let Some(res) = &self.batch.result else {
            self.batch.message = Some(Status::error("没有可保存的结果"));
            return;
        };
        self.batch.message = Some(match std::fs::write(path, &res.output.bytes) {
            Ok(()) => Status::ok(format!("已保存：{}", path.display())),
            Err(e) => Status::error(format!("保存失败：{e}")),
        });
    }
}

fn reverse_status(cfg: &Config, r: &ReverseOutcome) -> Status {
    if !r.exact {
        return Status::warn(format!(
            "无法精确匹配：已取最接近的不含增值税收入，对应税后实发金额 {}，差额 {}（计算结果 − 目标）",
            r.breakdown.take_home, r.diff
        ));
    }
    // 自检：选中的不含税收入再正算一遍，到手与目标一致
    let again = forward(cfg, r.breakdown.net_income).ok();
    if again.as_ref() != Some(&r.breakdown) || r.breakdown.take_home != r.target {
        return Status::error("校验未通过：测算结果复核不一致");
    }
    let mut t = format!(
        "精确匹配：不含增值税收入 {} 对应税后实发金额 {}，与输入一致",
        r.breakdown.net_income, r.breakdown.take_home
    );
    if let Some((lo, hi)) = r.exact_net_income_range {
        if lo != hi {
            t += &format!("（不含增值税收入 {lo}～{hi} 对应同一税后实发金额，已取最小值）");
        }
    }
    if !r.other_solutions.is_empty() {
        let s: Vec<String> = r.other_solutions.iter().map(|d| d.to_string()).collect();
        t += &format!("；另有不含增值税收入 {} 亦可精确匹配", s.join("、"));
    }
    t += exempt_note(&r.breakdown);
    Status::ok(t)
}
