//! 桌面程序的状态与操作逻辑（与界面绘制分离，便于测试）。所有计算都调用 labor-fee-core / labor-fee-batch。

use crate::batch_store::{kind_index, kind_key, KINDS};
use crate::settings::{
    item_index, Dialog, Prefs, SettingsDraft, SettingsStore, SettingsWindow, ThemeChoice,
};
use labor_fee_batch::{ColumnLayout, Computed, ExportColumn, Format, InputKind};
use labor_fee_core::{
    forward, from_total_cost, Breakdown, CalcError, Config, CostGap, CostGapReason, Decimal,
    ReverseOutcome, Reverser,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

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

/// 批量页签。`path`/`column`/`result`/`message` 为当前测算方式的输入与结果；
/// 切换方式（[`AppState::set_batch_kind`]）时与其他方式暂存的内容互换，每种方式各自保留上次的输入与结果，
/// 离开批量页再回来、切换方式再切回都不会丢失。
#[derive(Default)]
pub struct BatchTab {
    /// 输入文件路径（可手工输入，也可用“选择文件…”）。
    pub path: String,
    pub kind: InputKind,
    /// 列标题，空则用输入列类型的默认标题。
    pub column: String,
    pub result: Option<BatchResult>,
    pub message: Option<Status>,
    /// 各测算方式的导出列设置（选择与顺序），下标见 [`kind_index`]。
    pub layouts: [ColumnLayout; 3],
    /// 其他测算方式暂存的输入与结果。
    others: [ModeStash; 3],
    /// 待按当前参数重新计算的输入（参数更改后、启动时恢复上次结果）。
    pending: Option<BatchInput>,
    running: Option<RunningBatch>,
}

/// 某种测算方式暂存的输入与结果。
#[derive(Default)]
struct ModeStash {
    path: String,
    column: String,
    result: Option<BatchResult>,
    message: Option<Status>,
    pending: Option<BatchInput>,
}

struct RunningBatch {
    kind: InputKind,
    /// 开始时的参数版本；完成时参数已更改则按新参数重算。
    generation: u64,
    rx: Receiver<Result<BatchResult, String>>,
}

/// 批量计算的输入快照（原文件路径与内容）。
#[derive(Clone)]
pub struct BatchInput {
    pub path: PathBuf,
    pub bytes: Arc<Vec<u8>>,
    /// 是否为启动时恢复上次的结果。
    pub restored: bool,
}

/// 一次批量计算的结果：保存全部结果列，导出时按导出列设置生成文件，修改导出列无需重新计算。
pub struct BatchResult {
    pub input: PathBuf,
    pub kind: InputKind,
    pub computed: Computed,
    /// 输入文件内容快照（参数更改后据此重新计算，并缓存供下次启动恢复）。
    pub bytes: Arc<Vec<u8>>,
    /// 是否为启动时恢复的结果（按当前参数重新计算）。
    pub restored: bool,
}

impl BatchResult {
    /// 统计说明。
    pub fn summary_text(&self) -> String {
        let s = &self.computed.summary;
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

    /// 默认输出文件名（与 CLI 一致：原名_测算结果 / 原名_计算结果）。
    pub fn default_file_name(&self) -> String {
        let name = self.input.to_string_lossy();
        labor_fee_batch::output_file_name_for(&name, self.kind)
            .unwrap_or_else(|_| format!("结果.{}", self.computed.format.output_extension()))
    }

    /// 默认保存位置：输入文件同目录。
    pub fn default_save_path(&self) -> PathBuf {
        self.input.with_file_name(self.default_file_name())
    }

    /// 按导出列生成的预览（标题行 + 前 [`PREVIEW_ROWS`] 行）。
    pub fn preview(&self, columns: &[ExportColumn]) -> Vec<Vec<String>> {
        self.computed.preview(columns, PREVIEW_ROWS)
    }
}

/// 按文件扩展名确定导出格式：`.csv` 为 CSV，其余为 xlsx。
pub fn export_format(path: &Path) -> Format {
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
    {
        Some(e) if e == "csv" => Format::Csv,
        _ => Format::Spreadsheet,
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
    /// 参数版本（每次更换参数加一），用于判断后台批量计算是否按旧参数完成。
    cfg_generation: u64,
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
        let rate = if l.relief_rate.is_zero() {
            pct(l.statutory_rate)
        } else {
            format!("{}，减征 {}", pct(l.statutory_rate), pct(l.relief_rate))
        };
        v.push(f(&format!("  {}（{rate}）", l.name), l.amount.to_string()));
    }
    if !b.surcharge_relief.is_zero() {
        v.push(f("附加税费减征额", b.surcharge_relief.to_string()));
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
            cfg_generation: 0,
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
        st.restore_batch();
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

    /// 单项“默认”：把设置行 `k` 的输入填为内置默认值，进入未生效状态（不弹框、不保存）；
    /// 之后点击 √ 生效并保存，点击 × 放弃。生效值已等于默认时无变化。
    pub fn reset_setting(&mut self, k: &str) -> bool {
        let Some(w) = &mut self.settings else {
            return false;
        };
        let def = w.edit.defaults();
        if let Some(i) = item_index(k) {
            match w.edit.standard_item(i, &def).cloned() {
                Some(it) => {
                    if it.name == labor_fee_core::URBAN_CONSTRUCTION_TAX {
                        w.edit.urban_region = def.urban_region;
                        w.edit.urban_rates = def.urban_rates.clone();
                    }
                    w.edit.items[i] = it;
                }
                None => return false,
            }
        } else {
            let _ = w.edit.copy_row(&def, k);
        }
        w.message = None;
        true
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
        self.batch.pending = None;
        self.persist_batch();
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
        // 批量结果按新参数重新计算（当前方式在批量页显示时后台重算，其他方式切换过去时重算）
        self.cfg_generation += 1;
        let b = &mut self.batch;
        let slots = std::iter::once((&mut b.result, &mut b.message, &mut b.pending)).chain(
            b.others
                .iter_mut()
                .map(|s| (&mut s.result, &mut s.message, &mut s.pending)),
        );
        for (result, message, pending) in slots {
            if let Some(r) = result.take() {
                *pending = Some(BatchInput {
                    path: r.input,
                    bytes: r.bytes,
                    restored: false,
                });
                *message = Some(Status::ok("参数已更改，正在按新参数重新计算…"));
            }
        }
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

    // ---------------- 批量测算 ----------------

    /// 当前测算方式在当前参数下可导出的全部列（默认顺序）。
    pub fn batch_available_columns(&self) -> Vec<ExportColumn> {
        labor_fee_batch::available_columns(&self.cfg, self.batch.kind)
    }

    /// 当前测算方式的导出列列表（实际顺序与是否导出）。
    pub fn batch_column_entries(&self) -> Vec<(ExportColumn, bool)> {
        self.batch.layouts[kind_index(self.batch.kind)].entries(&self.batch_available_columns())
    }

    /// 当前测算方式导出的列（按顺序）。
    pub fn batch_selected_columns(&self) -> Vec<ExportColumn> {
        self.batch.layouts[kind_index(self.batch.kind)].selected(&self.batch_available_columns())
    }

    /// 当前测算方式的导出列是否为默认（全部导出、默认顺序）。
    pub fn batch_columns_default(&self) -> bool {
        ColumnLayout::from_entries(
            &self.batch_column_entries(),
            &self.batch_available_columns(),
        )
        .is_default()
    }

    /// 以界面上的列表更新当前测算方式的导出列设置并保存。
    pub fn set_batch_column_entries(&mut self, entries: &[(ExportColumn, bool)]) {
        let avail = self.batch_available_columns();
        let layout = ColumnLayout::from_entries(entries, &avail);
        let i = kind_index(self.batch.kind);
        if self.batch.layouts[i] != layout {
            self.batch.layouts[i] = layout;
            self.persist_batch();
        }
    }

    /// 勾选或取消第 `index` 列。
    pub fn set_batch_column_enabled(&mut self, index: usize, on: bool) {
        let mut e = self.batch_column_entries();
        if let Some(x) = e.get_mut(index) {
            x.1 = on;
            self.set_batch_column_entries(&e);
        }
    }

    /// 把第 `from` 列移到第 `to` 个位置（拖动排序、上移/下移）。
    pub fn move_batch_column(&mut self, from: usize, to: usize) {
        let mut e = self.batch_column_entries();
        if from >= e.len() || to >= e.len() || from == to {
            return;
        }
        let x = e.remove(from);
        e.insert(to, x);
        self.set_batch_column_entries(&e);
    }

    /// 全部勾选或全部取消（保持顺序）。
    pub fn set_all_batch_columns(&mut self, on: bool) {
        let e: Vec<_> = self
            .batch_column_entries()
            .into_iter()
            .map(|(c, _)| (c, on))
            .collect();
        self.set_batch_column_entries(&e);
    }

    /// 恢复默认列：全部导出、默认顺序。
    pub fn reset_batch_columns(&mut self) {
        self.batch.layouts[kind_index(self.batch.kind)] = ColumnLayout::default();
        self.persist_batch();
    }

    /// 切换测算方式：当前方式的输入与结果暂存，换出目标方式上次的输入与结果。
    /// 目标方式尚未选择文件时沿用当前文件路径。
    pub fn set_batch_kind(&mut self, kind: InputKind) {
        let b = &mut self.batch;
        if b.kind == kind {
            return;
        }
        let old = kind_index(b.kind);
        let new = kind_index(kind);
        b.others[old] = ModeStash {
            path: b.path.clone(),
            column: std::mem::take(&mut b.column),
            result: b.result.take(),
            message: b.message.take(),
            pending: b.pending.take(),
        };
        let s = std::mem::take(&mut b.others[new]);
        if !s.path.trim().is_empty() || s.result.is_some() || s.pending.is_some() {
            b.path = s.path;
        }
        b.column = s.column;
        b.result = s.result;
        b.message = s.message;
        b.pending = s.pending;
        b.kind = kind;
        self.persist_batch();
    }

    fn batch_job(
        &self,
        input: Option<BatchInput>,
    ) -> Result<impl FnOnce() -> Result<BatchResult, String> + Send + 'static, String> {
        let (path, snapshot, restored) = match input {
            Some(i) => (i.path, Some(i.bytes), i.restored),
            None => {
                if self.batch.path.trim().is_empty() {
                    return Err("请先选择文件".into());
                }
                (PathBuf::from(self.batch.path.trim()), None, false)
            }
        };
        let name = path.to_string_lossy().to_string();
        let format = Format::from_file_name(&name).map_err(|e| e.to_string())?;
        let cfg = self.cfg.clone();
        let kind = self.batch.kind;
        let column = Some(self.batch.column.trim().to_string()).filter(|c| !c.is_empty());
        Ok(move || {
            let bytes = match snapshot {
                Some(b) => b,
                None => Arc::new(std::fs::read(&path).map_err(|e| format!("读取文件失败：{e}"))?),
            };
            let opts = labor_fee_batch::Options {
                kind,
                column,
                ..Default::default()
            };
            let computed =
                labor_fee_batch::compute(&cfg, &bytes, format, &opts).map_err(|e| e.to_string())?;
            Ok(BatchResult {
                input: path,
                kind,
                computed,
                bytes,
                restored,
            })
        })
    }

    fn finish_batch(&mut self, kind: InputKind, generation: u64, r: Result<BatchResult, String>) {
        // 计算期间参数已更改：按新参数重算
        let (result, message, pending) = match r {
            Ok(res) if generation != self.cfg_generation => (
                None,
                Some(Status::ok("参数已更改，正在按新参数重新计算…")),
                Some(BatchInput {
                    path: res.input,
                    bytes: res.bytes,
                    restored: res.restored,
                }),
            ),
            Ok(res) => {
                let text = if res.restored {
                    format!(
                        "已恢复上次的结果（按当前参数重新计算）：{}",
                        res.summary_text()
                    )
                } else {
                    res.summary_text()
                };
                (Some(res), Some(Status::ok(text)), None)
            }
            Err(e) => (None, Some(Status::error(e)), None),
        };
        let fresh = result.as_ref().is_some_and(|r| !r.restored);
        if kind == self.batch.kind {
            self.batch.result = result;
            self.batch.message = message;
            self.batch.pending = pending;
        } else {
            let s = &mut self.batch.others[kind_index(kind)];
            s.result = result;
            s.message = message;
            s.pending = pending;
        }
        if fresh {
            self.cache_batch_input(kind);
        }
        self.persist_batch();
    }

    fn mode_result(&self, kind: InputKind) -> Option<&BatchResult> {
        if kind == self.batch.kind {
            self.batch.result.as_ref()
        } else {
            self.batch.others[kind_index(kind)].result.as_ref()
        }
    }

    /// 缓存输入文件副本（下次启动恢复结果用）。
    fn cache_batch_input(&mut self, kind: InputKind) {
        let Some(store) = &self.store else { return };
        let Some(res) = self.mode_result(kind) else {
            return;
        };
        let mut bp = store.load_batch();
        let m = bp.mode_mut(kind);
        match store.save_batch_input(kind, &res.input, &res.bytes) {
            Ok(Some(name)) => {
                m.cached = name;
                m.cached_from = res.input.display().to_string();
            }
            _ => {
                m.cached.clear();
                m.cached_from.clear();
            }
        }
        let _ = store.save_batch(&bp);
    }

    /// 保存批量测算设置（当前方式、各方式的路径、列标题与导出列）。
    fn persist_batch(&self) {
        let Some(store) = &self.store else { return };
        let mut bp = store.load_batch();
        bp.kind = kind_key(self.batch.kind).to_string();
        for k in KINDS {
            let i = kind_index(k);
            let (path, column) = if k == self.batch.kind {
                (&self.batch.path, &self.batch.column)
            } else {
                (&self.batch.others[i].path, &self.batch.others[i].column)
            };
            let m = bp.mode_mut(k);
            m.path = path.trim().to_string();
            m.column = column.trim().to_string();
            m.set_layout(&self.batch.layouts[i]);
        }
        if let Err(e) = store.save_batch(&bp) {
            eprintln!("保存批量测算状态失败：{e}");
        }
    }

    /// 启动时读取批量测算设置，并准备恢复各方式上次的结果（按当前参数重新计算）。
    fn restore_batch(&mut self) {
        let Some(store) = &self.store else { return };
        let bp = store.load_batch();
        let kind = bp.kind();
        for k in KINDS {
            let i = kind_index(k);
            let m = bp.mode(k);
            self.batch.layouts[i] = m.layout();
            let pending = store.load_batch_input(&m.cached).and_then(|bytes| {
                let from = PathBuf::from(&m.cached_from);
                // 副本的格式按原文件名判断
                Format::from_file_name(&m.cached_from).ok()?;
                Some(BatchInput {
                    path: from,
                    bytes: Arc::new(bytes),
                    restored: true,
                })
            });
            let message = pending
                .as_ref()
                .map(|_| Status::ok("正在恢复上次的批量结果…"));
            let s = ModeStash {
                path: m.path.clone(),
                column: m.column.clone(),
                result: None,
                message,
                pending,
            };
            if k == kind {
                self.batch.path = s.path;
                self.batch.column = s.column;
                self.batch.message = s.message;
                self.batch.pending = s.pending;
            } else {
                self.batch.others[i] = s;
            }
        }
        self.batch.kind = kind;
    }

    /// 当前方式是否有待重新计算的结果（参数更改后、启动恢复）。
    pub fn batch_needs_refresh(&self) -> bool {
        self.batch.pending.is_some() && self.batch.running.is_none()
    }

    /// 在后台按当前参数重新计算待更新的结果（批量页显示时调用）。
    pub fn refresh_batch(&mut self, on_done: impl FnOnce() + Send + 'static) {
        if let Some(input) = self.batch.pending.clone() {
            self.spawn_batch(Some(input), on_done);
        }
    }

    /// 同步重新计算待更新的结果（测试用）。
    pub fn refresh_batch_blocking(&mut self) {
        if let Some(input) = self.batch.pending.clone() {
            let r = self.batch_job(Some(input)).and_then(|job| job());
            self.finish_batch(self.batch.kind, self.cfg_generation, r);
        }
    }

    /// 同步执行批量计算（测试与小文件用）。
    pub fn run_batch_blocking(&mut self) {
        self.batch.pending = None;
        let r = self.batch_job(None).and_then(|job| job());
        self.finish_batch(self.batch.kind, self.cfg_generation, r);
    }

    /// 在后台线程执行批量计算（读取输入文件）；`on_done` 在完成时调用（用于请求界面重绘）。
    pub fn start_batch(&mut self, on_done: impl FnOnce() + Send + 'static) {
        self.spawn_batch(None, on_done);
    }

    fn spawn_batch(&mut self, input: Option<BatchInput>, on_done: impl FnOnce() + Send + 'static) {
        if self.batch.running.is_some() {
            return;
        }
        let refresh = input.is_some();
        match self.batch_job(input) {
            Err(e) => {
                self.batch.pending = None;
                self.finish_batch(self.batch.kind, self.cfg_generation, Err(e))
            }
            Ok(job) => {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(job());
                    on_done();
                });
                if !refresh {
                    self.batch.result = None;
                    self.batch.message = None;
                }
                self.batch.pending = None;
                self.batch.running = Some(RunningBatch {
                    kind: self.batch.kind,
                    generation: self.cfg_generation,
                    rx,
                });
            }
        }
    }

    /// 是否有批量计算在进行（当前方式）。
    pub fn batch_running(&self) -> bool {
        self.batch
            .running
            .as_ref()
            .is_some_and(|r| r.kind == self.batch.kind)
    }

    /// 检查后台批量计算是否完成。
    pub fn poll_batch(&mut self) {
        let Some(run) = &self.batch.running else {
            return;
        };
        let (kind, generation) = (run.kind, run.generation);
        let r = match run.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("批量测算意外中止".into()),
        };
        self.batch.running = None;
        self.finish_batch(kind, generation, r);
    }

    /// 按当前导出列设置生成文件并保存；格式按扩展名（`.csv` 为 CSV，其余为 xlsx）。
    pub fn save_batch(&mut self, path: &Path) {
        let Some(res) = &self.batch.result else {
            self.batch.message = Some(Status::error("没有可保存的结果"));
            return;
        };
        let cols = self.batch_selected_columns();
        if cols.is_empty() {
            self.batch.message = Some(Status::error("请至少选择一个导出列"));
            return;
        }
        let n = res.computed.headers(&cols).len();
        let r = res
            .computed
            .render(&cols, export_format(path))
            .map_err(|e| e.to_string())
            .and_then(|bytes| std::fs::write(path, bytes).map_err(|e| format!("保存失败：{e}")));
        self.batch.message = Some(match r {
            Ok(()) => Status::ok(format!("已保存：{}（{n} 列）", path.display())),
            Err(e) => Status::error(e),
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
