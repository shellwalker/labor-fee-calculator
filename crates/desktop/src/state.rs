//! 桌面程序的状态与操作逻辑（与界面绘制分离，便于测试）。所有计算都调用 labor-fee-core / labor-fee-batch。

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
    Config,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Forward,
        Tab::Reverse,
        Tab::FromCost,
        Tab::Batch,
        Tab::Config,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Forward => "正算",
            Tab::Reverse => "反算",
            Tab::FromCost => "按总成本",
            Tab::Batch => "批量",
            Tab::Config => "口径",
        }
    }
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
    fn ok(text: impl Into<String>) -> Self {
        Status {
            level: Level::Ok,
            text: text.into(),
        }
    }
    fn warn(text: impl Into<String>) -> Self {
        Status {
            level: Level::Warn,
            text: text.into(),
        }
    }
    fn error(text: impl Into<String>) -> Self {
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
    Done { fields: Vec<Field>, status: Status },
    /// 企业总成本落在空位：原因及两侧最近的有效值。
    Gap(Box<CostGap>),
    /// 输入或计算错误。
    Error(String),
}

/// 单笔计算页签（正算 / 反算 / 按总成本）。
#[derive(Debug, Clone)]
pub struct SingleTab {
    pub input: String,
    pub outcome: Outcome,
}

impl Default for SingleTab {
    fn default() -> Self {
        SingleTab {
            input: String::new(),
            outcome: Outcome::None,
        }
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
            t += &format!("，不可精确达到 {}", s.inexact);
        }
        if self.kind == InputKind::TotalCost {
            t += &format!("，总成本不可达 {}", s.unreachable_cost);
        }
        t += &format!("，错误 {}，空白 {}", s.errors, s.empty);
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

/// 口径查看区。
#[derive(Debug, Clone)]
pub struct ConfigView {
    /// 当前口径来源说明。
    pub source: String,
    /// 当前生效配置（TOML）。
    pub toml: String,
    /// 自定义 TOML 路径输入框。
    pub path: String,
    pub message: Option<Status>,
}

/// 整个程序的状态。
pub struct AppState {
    pub tab: Tab,
    cfg: Config,
    reverser: Reverser,
    pub forward: SingleTab,
    pub reverse: SingleTab,
    pub from_cost: SingleTab,
    pub batch: BatchTab,
    pub config: ConfigView,
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
        f("不含税收入", b.net_income.to_string()),
        f("增值税", b.vat.to_string()),
    ];
    if b.surcharges.len() > 1 {
        for l in &b.surcharges {
            v.push(f(
                &format!("  {}（{}）", l.name, pct(l.rate)),
                l.amount.to_string(),
            ));
        }
        v.push(f("附加税合计", b.surcharge_total.to_string()));
    } else {
        v.push(f("附加税", b.surcharge_total.to_string()));
    }
    v.extend([
        f("企业总成本", b.total_cost.to_string()),
        f("减除费用", b.iit.deduction.to_string()),
        f("应纳税所得额", b.iit.taxable_income.to_string()),
        f("预扣率", pct(b.iit.rate)),
        f("速算扣除数", b.iit.quick_deduction.to_string()),
        f("个税", b.iit.tax.to_string()),
        f("实际到手", b.take_home.to_string()),
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
        "；未达征收门槛，不征增值税及附加税"
    } else {
        ""
    }
}

/// 空位原因的简短说明。
pub fn gap_reason_text(g: &CostGap) -> String {
    match g.reason {
        CostGapReason::Threshold => format!(
            "原因：{} 元增值税征收门槛形成的空白区间",
            g.threshold
                .map(|t| t.normalize().to_string())
                .unwrap_or_default()
        ),
        CostGapReason::Rounding => "原因：增值税、附加税逐分舍入".to_string(),
    }
}

impl AppState {
    /// 以给定口径创建；`source` 为口径来源说明。
    pub fn new(cfg: Config, source: impl Into<String>) -> Result<Self, CalcError> {
        let reverser = Reverser::new(&cfg)?;
        let toml = cfg.to_toml_string();
        Ok(AppState {
            tab: Tab::Forward,
            cfg,
            reverser,
            forward: SingleTab::default(),
            reverse: SingleTab::default(),
            from_cost: SingleTab::default(),
            batch: BatchTab::default(),
            config: ConfigView {
                source: source.into(),
                toml,
                path: String::new(),
                message: None,
            },
        })
    }

    /// 内置默认口径。
    pub fn with_default_config() -> Self {
        Self::new(Config::default(), "内置默认口径").expect("默认配置有效")
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
                        "校验通过：总成本 = 不含税收入 + 增值税 + 附加税，到手 = 不含税收入 − 个税{}",
                        exempt_note(&b)
                    ))
                } else {
                    Status::error("校验失败：恒等式不成立")
                };
                Outcome::Done {
                    fields: fields(&b),
                    status,
                }
            }
            Err(e) => Outcome::Error(e),
        };
    }

    /// 反算：实际到手 → 不含税收入 → 全部金额。
    pub fn calc_reverse(&mut self) {
        self.reverse.outcome = match parse_amount(&self.reverse.input)
            .and_then(|a| self.reverser.reverse(a).map_err(|e| e.to_string()))
        {
            Ok(r) => Outcome::Done {
                fields: fields(&r.breakdown),
                status: reverse_status(&self.cfg, &r),
            },
            Err(e) => Outcome::Error(e),
        };
    }

    /// 按企业总成本正算；空位时给出原因和两侧最近的有效总成本。
    pub fn calc_from_cost(&mut self) {
        self.from_cost.outcome = match parse_amount(&self.from_cost.input) {
            Err(e) => Outcome::Error(e),
            Ok(a) => match from_total_cost(&self.cfg, a) {
                Ok(b) => Outcome::Done {
                    status: if identity_ok(&b) && b.total_cost == a {
                        Status::ok(format!(
                            "可达：不含税收入 {} 正算的企业总成本正好等于输入{}",
                            b.net_income,
                            exempt_note(&b)
                        ))
                    } else {
                        Status::error("校验失败")
                    },
                    fields: fields(&b),
                },
                Err(CalcError::CostUnreachable(g)) => Outcome::Gap(g),
                Err(e) => Outcome::Error(e.to_string()),
            },
        };
    }

    /// 点击空位两侧的有效值：填入并重算。
    pub fn use_cost(&mut self, cost: Decimal) {
        self.from_cost.input = cost.to_string();
        self.calc_from_cost();
    }

    /// 更换口径，并重算已有结果。
    pub fn set_config(&mut self, cfg: Config, source: impl Into<String>) -> Result<(), CalcError> {
        let reverser = Reverser::new(&cfg)?;
        self.config.toml = cfg.to_toml_string();
        self.config.source = source.into();
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

    /// 从文件加载口径，结果写入口径区的提示。
    pub fn load_config_file(&mut self, path: &Path) {
        let r = std::fs::read_to_string(path)
            .map_err(|e| format!("读取失败：{e}"))
            .and_then(|t| self.load_config_str(&t, format!("文件 {}", path.display())));
        self.config.path = path.display().to_string();
        self.config.message = Some(match r {
            Ok(()) => Status::ok(format!("已加载：{}", path.display())),
            Err(e) => Status::error(format!("加载失败（仍使用原口径）：{e}")),
        });
    }

    /// 恢复内置默认口径。
    pub fn reset_config(&mut self) {
        self.set_config(Config::default(), "内置默认口径")
            .expect("默认配置有效");
        self.config.message = Some(Status::ok("已恢复内置默认口径"));
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
                self.finish_batch(Err("批量计算意外中止".into()));
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
            "不可精确达到：已取最接近的不含税收入，正算到手 {}，差额 {}（正算到手 − 目标）",
            r.breakdown.take_home, r.diff
        ));
    }
    // 自检：选中的不含税收入再正算一遍，到手与目标一致
    let again = forward(cfg, r.breakdown.net_income).ok();
    if again.as_ref() != Some(&r.breakdown) || r.breakdown.take_home != r.target {
        return Status::error("校验失败：反算结果正算不一致");
    }
    let mut t = format!(
        "精确：不含税收入 {} 正算回去到手 {}，与输入一致",
        r.breakdown.net_income, r.breakdown.take_home
    );
    if let Some((lo, hi)) = r.exact_net_income_range {
        if lo != hi {
            t += &format!("（不含税收入 {lo}～{hi} 都得到同一到手，已取最小）");
        }
    }
    if !r.other_solutions.is_empty() {
        let s: Vec<String> = r.other_solutions.iter().map(|d| d.to_string()).collect();
        t += &format!("；另有不含税收入 {} 也可精确达到", s.join("、"));
    }
    t += exempt_note(&r.breakdown);
    Status::ok(t)
}
