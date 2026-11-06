//! 劳务报酬批量计算。
//!
//! 读取 xlsx/xls/ods（第一个工作表）或 csv，按输入类型取首行标题对应的列：
//! - 默认“税后实发金额”（按税后实发金额批量测算；兼容旧列名“实发金额”“到手金额”）；
//! - 可选“不含增值税收入”（批量计算；兼容“不含税收入”）或“企业总支出”（按企业总支出批量测算；
//!   兼容“企业总成本”，无对应收入的行在备注列写明原因）。
//!
//! 对每一行计算全部结果列（[`compute`]），再按导出列的选择与顺序生成新文件（[`Computed::render`]，
//! CSV 或 xlsx）；导出列见 [`columns`]，默认导出原表各列与全部结果列。
//!
//! 计算全部使用十进制；仅在读取 Excel 数值单元格、写出 Excel 数值单元格这两个 I/O 边界上与 f64 互转
//! （Excel 文件格式本身以浮点存储数值），读入时会校验小数位不超过两位。

use labor_fee_core::{
    forward, forward_with_vat_reference, from_total_cost, from_total_cost_with_vat_reference,
    Breakdown, CalcError, Config, Decimal, Reverser,
};
use rust_decimal::prelude::ToPrimitive;
use std::borrow::Cow;
use std::io::Cursor;
use std::str::FromStr;
use thiserror::Error;

pub mod columns;
mod preserve_xlsx;
pub use columns::{available_columns, parse_columns, ColumnLayout, ExportColumn};

/// 默认读取的列标题（按税后实发金额测算）。
pub const DEFAULT_COLUMN: &str = "税后实发金额";
/// 旧版默认列标题，仍可识别。
pub const LEGACY_COLUMN: &str = "到手金额";

/// 结果表“测算状态”列：按收入计算、按企业总支出测算成功时的状态。
pub const STATUS_DONE: &str = "已计算";
/// 结果表“测算状态”列：数据错误。
pub const STATUS_ERROR: &str = "数据错误";
/// 结果表“测算状态”列：企业总支出无对应收入。
pub const STATUS_UNREACHABLE: &str = "总支出不可达";
/// 未达到起征点时的备注。
pub const VAT_EXEMPT_NOTE: &str = "未达到增值税起征点，不征收增值税及附加税费";

/// 输入列的含义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputKind {
    /// 税后实发金额 → 测算不含增值税收入（默认）。
    #[default]
    TakeHome,
    /// 不含增值税收入 → 计算。
    NetIncome,
    /// 企业总支出 → 测算不含增值税收入（不可达金额报错）。
    TotalCost,
}

impl InputKind {
    /// 该输入类型默认读取的列标题。
    pub fn default_column(self) -> &'static str {
        match self {
            InputKind::TakeHome => DEFAULT_COLUMN,
            InputKind::NetIncome => "不含增值税收入",
            InputKind::TotalCost => "企业总支出",
        }
    }

    /// 未指定列名时可识别的列标题（按优先顺序；含旧版列名）。
    pub fn column_aliases(self) -> &'static [&'static str] {
        match self {
            InputKind::TakeHome => &[DEFAULT_COLUMN, "实发金额", LEGACY_COLUMN],
            InputKind::NetIncome => &["不含增值税收入", "不含税收入"],
            InputKind::TotalCost => &["企业总支出", "企业总成本"],
        }
    }

    /// 简短名称（界面与状态说明用）。
    pub fn title(self) -> &'static str {
        match self {
            InputKind::TakeHome => "按税后实发金额测算",
            InputKind::NetIncome => "按不含增值税收入计算",
            InputKind::TotalCost => "按企业总支出测算",
        }
    }

    /// 由字符串解析：take-home / net-income / total-cost（也接受中文列名）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "take-home" | "take_home" | "税后实发金额" | "实发金额" | "到手金额" => {
                Some(InputKind::TakeHome)
            }
            "net-income" | "net_income" | "不含增值税收入" | "不含税收入" => {
                Some(InputKind::NetIncome)
            }
            "total-cost" | "total_cost" | "企业总支出" | "企业总成本" => {
                Some(InputKind::TotalCost)
            }
            _ => None,
        }
    }
}

/// 批量处理错误（文件级）。行级错误写入结果表的“测算状态”“备注”列，不中断处理。
#[derive(Debug, Error)]
pub enum BatchError {
    #[error("不支持的文件类型：{0}（支持 .xlsx/.xlsm/.xls/.ods/.csv）")]
    UnsupportedFormat(String),
    #[error("不支持的输出文件类型：{0}（请使用 .csv 或 .xlsx）")]
    UnsupportedOutputFormat(String),
    #[error("表格内容为空")]
    Empty,
    #[error("首行未找到标题为{0}的列")]
    ColumnNotFound(String),
    #[error("读取文件失败：{0}")]
    Read(String),
    #[error("增值税起征点分组失败：{0}")]
    Grouping(String),
    #[error("未选择任何导出列")]
    NoColumns,
    #[error("写出文件失败：{0}")]
    Write(String),
    #[error("文件大小 {size} 字节超过上限 {max} 字节")]
    FileTooLarge { size: usize, max: usize },
    #[error("数据行数超过上限 {max} 行（不含标题行）")]
    TooManyRows { max: usize },
    #[error("列数 {cols} 超过上限 {max}")]
    TooManyColumns { cols: usize, max: usize },
    #[error("单元格长度 {len} 超过上限 {max} 个字符")]
    CellTooLong { len: usize, max: usize },
    #[error(transparent)]
    Calc(#[from] CalcError),
}

/// 输入/输出文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    /// Excel / ODS 表格（输出统一为 xlsx）。
    Spreadsheet,
}

impl Format {
    /// 按文件扩展名识别。
    pub fn from_file_name(name: &str) -> Result<Self, BatchError> {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "csv" => Ok(Format::Csv),
            "xlsx" | "xlsm" | "xls" | "xlsb" | "ods" => Ok(Format::Spreadsheet),
            _ => Err(BatchError::UnsupportedFormat(name.to_string())),
        }
    }

    /// 按输出文件扩展名决定写出格式。只接受 `.csv` 与 `.xlsx`（大小写不敏感）。
    ///
    /// 输入可以是 `.xls` / `.ods` 等，写出不行：扩展名不支持时返回错误，不按输入格式猜测。
    pub fn from_output_file_name(name: &str) -> Result<Self, BatchError> {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "csv" => Ok(Format::Csv),
            "xlsx" => Ok(Format::Spreadsheet),
            _ => Err(BatchError::UnsupportedOutputFormat(name.to_string())),
        }
    }

    /// 输出文件扩展名。
    pub fn output_extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Spreadsheet => "xlsx",
        }
    }

    /// 输出文件 MIME 类型。
    pub fn content_type(self) -> &'static str {
        match self {
            Format::Csv => "text/csv",
            Format::Spreadsheet => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            }
        }
    }
}

/// 根据输入文件名生成输出文件名：`原名_测算结果.xlsx/csv`。
pub fn output_file_name(input_name: &str) -> Result<String, BatchError> {
    output_file_name_for(input_name, InputKind::TakeHome)
}

/// 按输入类型生成输出文件名：按税后实发金额测算为 `原名_测算结果`，其余为 `原名_计算结果`。
pub fn output_file_name_for(input_name: &str, kind: InputKind) -> Result<String, BatchError> {
    let fmt = Format::from_file_name(input_name)?;
    let stem = input_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(input_name);
    let stem = stem.rsplit(['/', '\\']).next().unwrap_or(stem);
    let suffix = match kind {
        InputKind::TakeHome => "测算结果",
        _ => "计算结果",
    };
    Ok(format!("{stem}_{suffix}.{}", fmt.output_extension()))
}

/// 批量输入的默认文件大小上限（20 MiB）。与 Web `--max-upload-mb` 的默认值一致。
pub const DEFAULT_MAX_FILE_BYTES: usize = 20 * 1024 * 1024;
/// 批量输入的默认数据行数上限（不含标题行）。
pub const DEFAULT_MAX_ROWS: usize = 100_000;
/// 批量输入的默认列数上限（标题与各数据行的最大宽度）。
pub const DEFAULT_MAX_COLS: usize = 256;
/// 批量输入的默认单元格字符数上限，与 Excel 单元格 32,767 个字符的限制一致。
pub const DEFAULT_MAX_CELL_CHARS: usize = 32_767;

/// 批量输入的规模上限。超过任一上限时整份文件报错，不逐行继续。
///
/// 默认值见各 `DEFAULT_MAX_*` 常量，也可在 CLI / Web 启动参数里覆盖。假设见 `docs/ASSUMPTIONS.md`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// 输入文件字节数上限。
    pub max_file_bytes: usize,
    /// 数据行数上限（不含标题行）。
    pub max_rows: usize,
    /// 列数上限。
    pub max_cols: usize,
    /// 单个文本单元格的字符数上限。
    pub max_cell_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_rows: DEFAULT_MAX_ROWS,
            max_cols: DEFAULT_MAX_COLS,
            max_cell_chars: DEFAULT_MAX_CELL_CHARS,
        }
    }
}

/// 批量处理选项。
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// 输入列的含义（默认税后实发金额 → 测算）。
    pub kind: InputKind,
    /// 读取的列标题（首行精确匹配，忽略首尾空白）；为空时用输入类型的默认列名。
    pub column: Option<String>,
    /// 在 [`Output::preview`] 中返回的数据行数（不含标题行），0 表示不生成预览。
    pub preview_rows: usize,
    /// 导出的列及顺序；为空时导出全部列（[`available_columns`] 的默认顺序）。
    pub columns: Option<Vec<ExportColumn>>,
    /// 文件大小、行数、列数、单元格长度上限。
    pub limits: Limits,
    /// 按人员与日期合并判断增值税起征点；默认关闭。
    pub vat_grouping: Option<VatGrouping>,
}

/// 批量增值税起征点分组所使用的标题；日期归一到公历日，拒绝时间部分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VatGrouping {
    pub person_column: String,
    pub date_column: String,
}

impl Options {
    /// 实际使用的列标题。
    pub fn column_name(&self) -> String {
        self.column
            .clone()
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| self.kind.default_column().to_string())
    }
}

/// 处理统计。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// 数据行数（不含标题行）。
    pub rows: usize,
    /// 计算成功（反算精确）的行数。
    pub exact: usize,
    /// 反算不可精确达到的行数。
    pub inexact: usize,
    /// 企业总成本落在空位、没有对应不含税收入的行数。
    pub unreachable_cost: usize,
    /// 金额无法解析或计算出错的行数。
    pub errors: usize,
    /// 到手金额为空的行数。
    pub empty: usize,
}

/// 批量处理输出。
#[derive(Debug, Clone)]
pub struct Output {
    pub bytes: Vec<u8>,
    pub format: Format,
    pub summary: Summary,
    /// 结果表预览（文本形式，与 CSV 输出一致）：第一行为标题，其后为前 `preview_rows` 个数据行。
    /// `preview_rows` 为 0 时为空。
    pub preview: Vec<Vec<String>>,
}

/// 原表单元格。
#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Empty,
    Text(String),
    Number(f64),
}

impl Cell {
    fn text(&self) -> String {
        match self {
            Cell::Empty => String::new(),
            Cell::Text(s) => s.clone(),
            Cell::Number(n) => n.to_string(),
        }
    }
}

/// 文本若以 `=` `+` `-` `@`、制表符或回车开头，加前缀 `'`，避免被表格软件当成公式。
/// 数值单元格和金额结果不经过这里，负数金额仍按数值写出。
fn escape_formula(s: &str) -> Cow<'_, str> {
    match s.as_bytes().first() {
        Some(b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r') => Cow::Owned(format!("'{s}")),
        _ => Cow::Borrowed(s),
    }
}

/// 导出时的原表单元格文本：数值（含负数）原样，文本做公式转义。
fn cell_export_text(cell: &Cell) -> String {
    match cell {
        Cell::Empty => String::new(),
        Cell::Number(n) => n.to_string(),
        Cell::Text(s) => escape_formula(s).into_owned(),
    }
}

/// 结果单元格。
#[derive(Debug, Clone, PartialEq)]
enum Out {
    Empty,
    Money(Decimal),
    Rate(Decimal),
    Text(String),
}

/// 解析金额：文本允许千分位逗号、¥、元、空白；数值单元格允许浮点噪声（<1e-6）后舍入到分。
fn parse_amount(cell: &Cell, scale: u32) -> Result<Option<Decimal>, String> {
    let (d, tolerant) = match cell {
        Cell::Empty => return Ok(None),
        Cell::Number(n) => {
            if !n.is_finite() {
                return Err("金额不是有效数字".into());
            }
            (
                Decimal::from_str(&n.to_string()).map_err(|_| format!("金额超出范围：{n}"))?,
                true,
            )
        }
        Cell::Text(s) => {
            let cleaned: String = s
                .chars()
                .filter(|c| !c.is_whitespace() && !matches!(c, ',' | '，' | '¥' | '￥' | '元'))
                .collect();
            if cleaned.is_empty() {
                return Ok(None);
            }
            let d = labor_fee_core::parse_amount(s)?;
            (d, false)
        }
    };
    let r = d.round_dp(scale);
    if r != d && !(tolerant && (r - d).abs() < Decimal::new(1, 6)) {
        return Err(format!("金额 {d} 的小数位数超过 {scale} 位"));
    }
    Ok(Some(r))
}

/// 一行的计算结果。
struct RowResult {
    breakdown: Breakdown,
    status: String,
    diff: Option<Decimal>,
    notes: Vec<String>,
}

/// 按可导出列计算一行的结果值（与 `cols` 一一对应，原表各列位置为空）。
fn result_values(cols: &[ExportColumn], r: &RowResult) -> Vec<Out> {
    use ExportColumn as C;
    let b = &r.breakdown;
    cols.iter()
        .map(|c| match c {
            C::Source => Out::Empty,
            C::NetIncome => Out::Money(b.net_income),
            C::Vat => Out::Money(b.vat),
            C::Surcharge(name) => b
                .surcharges
                .iter()
                .find(|l| &l.name == name)
                .map_or(Out::Empty, |l| Out::Money(l.amount)),
            C::SurchargeRelief => Out::Money(b.surcharge_relief),
            C::SurchargeTotal => Out::Money(b.surcharge_total),
            C::TotalCost => Out::Money(b.total_cost),
            C::Income => Out::Money(b.iit.income),
            C::Deduction => Out::Money(b.iit.deduction),
            C::TaxableIncome => Out::Money(b.iit.taxable_income),
            C::Rate => Out::Rate(b.iit.rate),
            C::QuickDeduction => Out::Money(b.iit.quick_deduction),
            C::IitTax => Out::Money(b.iit.tax),
            C::TakeHome => Out::Money(b.take_home),
            C::Status => Out::Text(r.status.clone()),
            C::Diff => r.diff.map_or(Out::Empty, Out::Money),
            C::Notes => {
                let mut notes = Vec::new();
                if b.vat_exempt {
                    notes.push(VAT_EXEMPT_NOTE.to_string());
                }
                notes.extend(r.notes.iter().cloned());
                Out::Text(notes.join("；"))
            }
        })
        .collect()
}

/// 出错行：测算状态写简短状态，备注写完整原因，其余为空。
fn error_values(cols: &[ExportColumn], status: &str, msg: &str) -> Vec<Out> {
    cols.iter()
        .map(|c| match c {
            ExportColumn::Status => Out::Text(status.to_string()),
            ExportColumn::Notes => Out::Text(msg.to_string()),
            _ => Out::Empty,
        })
        .collect()
}

fn money_format(scale: u32) -> String {
    if scale == 0 {
        "0".into()
    } else {
        format!("0.{}", "0".repeat(scale as usize))
    }
}

fn decimal_number(value: Decimal) -> Result<f64, BatchError> {
    value
        .to_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| BatchError::Write(format!("金额无法写入 Excel 数值：{value}")))
}

#[derive(Debug, Clone, Copy)]
enum CsvEncoding {
    Utf8,
    Utf8Bom,
    Gb18030,
}
impl CsvEncoding {
    fn detect(input: &[u8], format: Format) -> Self {
        if format != Format::Csv || input.starts_with(b"\xef\xbb\xbf") {
            Self::Utf8Bom
        } else if std::str::from_utf8(input).is_ok() {
            Self::Utf8
        } else {
            Self::Gb18030
        }
    }
    fn encode(self, bytes: Vec<u8>) -> Result<Vec<u8>, BatchError> {
        match self {
            Self::Utf8 => Ok(bytes),
            Self::Utf8Bom => Ok([b"\xef\xbb\xbf".as_slice(), &bytes].concat()),
            Self::Gb18030 => {
                let text =
                    std::str::from_utf8(&bytes).map_err(|e| BatchError::Write(e.to_string()))?;
                let (encoded, _, bad) = encoding_rs::GB18030.encode(text);
                if bad {
                    return Err(BatchError::Write("结果包含 GB18030 无法表示的字符".into()));
                }
                Ok(encoded.into_owned())
            }
        }
    }
}

/// 读取 CSV：自动识别 UTF-8（含 BOM）与 GBK/GB18030 编码。
fn read_csv(input: &[u8]) -> Result<Vec<Vec<Cell>>, BatchError> {
    let text: String = match std::str::from_utf8(input) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(_) => {
            let (s, _, bad) = encoding_rs::GB18030.decode(input);
            if bad {
                return Err(BatchError::Read(
                    "CSV 编码无法识别（支持 UTF-8 / GBK）".into(),
                ));
            }
            s.into_owned()
        }
    };
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut rows = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| BatchError::Read(e.to_string()))?;
        rows.push(
            rec.iter()
                .map(|f| {
                    if f.is_empty() {
                        Cell::Empty
                    } else {
                        Cell::Text(f.to_string())
                    }
                })
                .collect(),
        );
    }
    Ok(rows)
}

/// 读取表格第一个工作表。
fn read_spreadsheet(input: &[u8]) -> Result<Vec<Vec<Cell>>, BatchError> {
    use calamine::{Data, Reader};
    let mut wb = calamine::open_workbook_auto_from_rs(Cursor::new(input.to_vec()))
        .map_err(|e| BatchError::Read(e.to_string()))?;
    let range = wb
        .worksheet_range_at(0)
        .ok_or(BatchError::Empty)?
        .map_err(|e| BatchError::Read(e.to_string()))?;
    Ok(range
        .rows()
        .map(|row| {
            row.iter()
                .map(|d| match d {
                    Data::Empty => Cell::Empty,
                    Data::Int(i) => Cell::Number(*i as f64),
                    Data::Float(f) => Cell::Number(*f),
                    Data::String(s) => Cell::Text(s.clone()),
                    Data::DateTime(d) if d.is_datetime() => {
                        let (y, m, day, h, min, sec, milli) = d.to_ymd_hms_milli();
                        Cell::Text(if (h, min, sec, milli) == (0, 0, 0, 0) {
                            format!("{y:04}-{m:02}-{day:02}")
                        } else {
                            format!("{y:04}-{m:02}-{day:02}T{h:02}:{min:02}:{sec:02}.{milli:03}")
                        })
                    }
                    other => Cell::Text(other.to_string()),
                })
                .collect()
        })
        .collect())
}

impl Out {
    fn text(&self) -> String {
        match self {
            Out::Empty => String::new(),
            Out::Money(d) => d.to_string(),
            Out::Rate(d) => format!("{}%", (*d * Decimal::ONE_HUNDRED).normalize()),
            Out::Text(s) => s.clone(),
        }
    }

    /// CSV / 预览用文本。金额和预扣率保持数值写法（含负数），其余文本做公式转义。
    fn export_text(&self) -> String {
        match self {
            Out::Text(s) => escape_formula(s).into_owned(),
            other => other.text(),
        }
    }
}

/// 输出表的一列：原表第 i 列，或第 i 个结果列。
#[derive(Debug, Clone, Copy)]
enum Slot {
    Source(usize),
    Result(usize),
}

/// 已完成计算的批量结果：保存原表与全部结果列，可按任意导出列选择与顺序生成文件和预览，无需重新计算。
#[derive(Debug, Clone)]
pub struct Computed {
    /// 输入列的含义。
    pub kind: InputKind,
    /// 输入文件格式（默认导出格式）。
    pub format: Format,
    pub summary: Summary,
    header: Vec<Cell>,
    original_xlsx: Option<Vec<u8>>,
    csv_encoding: CsvEncoding,
    money_scale: u32,
    columns: Vec<ExportColumn>,
    rows: Vec<(Vec<Cell>, Vec<Out>)>,
}

impl Computed {
    /// 可导出的全部列（默认顺序）。
    pub fn available_columns(&self) -> &[ExportColumn] {
        &self.columns
    }

    /// 数据行数。
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// 原表的列标题（原表各列展开后的标题）。
    pub fn source_headers(&self) -> Vec<String> {
        self.header.iter().map(Cell::text).collect()
    }

    fn plan(&self, columns: &[ExportColumn]) -> Vec<Slot> {
        if columns.is_empty() {
            return Vec::new();
        }
        let mut plan: Vec<_> = (0..self.header.len()).map(Slot::Source).collect();
        for c in columns {
            match c {
                ExportColumn::Source => {}
                c => {
                    if let Some(i) = self.columns.iter().position(|x| x == c) {
                        plan.push(Slot::Result(i));
                    }
                }
            }
        }
        plan
    }

    fn header_text(&self, s: Slot) -> String {
        match s {
            Slot::Source(i) => cell_export_text(&self.header[i]),
            Slot::Result(i) => self.columns[i].title().to_string(),
        }
    }

    fn row_texts(&self, plan: &[Slot], orig: &[Cell], out: &[Out]) -> Vec<String> {
        plan.iter()
            .map(|s| match *s {
                Slot::Source(i) => cell_export_text(&orig[i]),
                Slot::Result(i) => out[i].export_text(),
            })
            .collect()
    }

    /// 导出文件的标题行（按导出列展开）。
    pub fn headers(&self, columns: &[ExportColumn]) -> Vec<String> {
        self.plan(columns)
            .into_iter()
            .map(|s| self.header_text(s))
            .collect()
    }

    /// 文本预览（与 CSV 输出一致）：标题行 + 前 `rows` 个数据行。
    pub fn preview(&self, columns: &[ExportColumn], rows: usize) -> Vec<Vec<String>> {
        let plan = self.plan(columns);
        std::iter::once(plan.iter().map(|s| self.header_text(*s)).collect())
            .chain(
                self.rows
                    .iter()
                    .take(rows)
                    .map(|(orig, out)| self.row_texts(&plan, orig, out)),
            )
            .collect()
    }

    /// 按导出列（选择与顺序）生成文件；`format` 为输出格式（CSV 或 xlsx）。
    pub fn render(&self, columns: &[ExportColumn], format: Format) -> Result<Vec<u8>, BatchError> {
        let plan = self.plan(columns);
        if plan.is_empty() {
            return Err(BatchError::NoColumns);
        }
        match format {
            Format::Csv => self.write_csv(&plan),
            Format::Spreadsheet => self.write_xlsx(&plan),
        }
    }

    fn write_csv(&self, plan: &[Slot]) -> Result<Vec<u8>, BatchError> {
        let mut buf = Vec::new();
        {
            let mut w = csv::WriterBuilder::new()
                .flexible(true)
                .from_writer(&mut buf);
            let h: Vec<String> = plan.iter().map(|s| self.header_text(*s)).collect();
            w.write_record(&h)
                .map_err(|e| BatchError::Write(e.to_string()))?;
            for (orig, out) in &self.rows {
                w.write_record(self.row_texts(plan, orig, out))
                    .map_err(|e| BatchError::Write(e.to_string()))?;
            }
            w.flush().map_err(|e| BatchError::Write(e.to_string()))?;
        }
        self.csv_encoding.encode(buf)
    }

    fn write_xlsx(&self, plan: &[Slot]) -> Result<Vec<u8>, BatchError> {
        if let Some(original) = &self.original_xlsx {
            return preserve_xlsx::append(self, original, plan);
        }
        use rust_xlsxwriter::{Format as XFormat, Workbook};
        let werr = |e: rust_xlsxwriter::XlsxError| BatchError::Write(e.to_string());
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet();
        ws.set_name("计算结果").map_err(werr)?;
        let bold = XFormat::new().set_bold();
        let money = XFormat::new().set_num_format(money_format(self.money_scale));
        let pct = XFormat::new().set_num_format("0%");
        for (c, s) in plan.iter().enumerate() {
            let c = c as u16;
            match *s {
                Slot::Source(i) => match &self.header[i] {
                    Cell::Empty => {}
                    Cell::Number(n) => {
                        ws.write_number_with_format(0, c, *n, &bold).map_err(werr)?;
                    }
                    Cell::Text(t) => {
                        ws.write_string_with_format(0, c, escape_formula(t).as_ref(), &bold)
                            .map_err(werr)?;
                    }
                },
                Slot::Result(i) => {
                    ws.write_string_with_format(0, c, self.columns[i].title(), &bold)
                        .map_err(werr)?;
                    ws.set_column_width(c, 14).map_err(werr)?;
                }
            }
        }
        for (ri, (orig, out)) in self.rows.iter().enumerate() {
            let r = (ri + 1) as u32;
            for (c, s) in plan.iter().enumerate() {
                let c = c as u16;
                match *s {
                    Slot::Source(i) => match &orig[i] {
                        Cell::Empty => {}
                        Cell::Text(t) => {
                            ws.write_string(r, c, escape_formula(t).as_ref())
                                .map_err(werr)?;
                        }
                        Cell::Number(n) => {
                            ws.write_number(r, c, *n).map_err(werr)?;
                        }
                    },
                    Slot::Result(i) => match &out[i] {
                        Out::Empty => {}
                        Out::Money(d) => {
                            ws.write_number_with_format(r, c, decimal_number(*d)?, &money)
                                .map_err(werr)?;
                        }
                        Out::Rate(d) => {
                            ws.write_number_with_format(r, c, decimal_number(*d)?, &pct)
                                .map_err(werr)?;
                        }
                        Out::Text(t) => {
                            ws.write_string(r, c, escape_formula(t).as_ref())
                                .map_err(werr)?;
                        }
                    },
                }
            }
        }
        wb.save_to_buffer().map_err(werr)
    }
}

/// 批量计算：读取输入文件字节，逐行计算全部结果列（不生成文件）。
/// 之后可用 [`Computed::render`] 按任意导出列生成文件。
fn enforce_table_limits(table: &[Vec<Cell>], limits: &Limits) -> Result<(), BatchError> {
    let data_rows = table.len().saturating_sub(1);
    if data_rows > limits.max_rows {
        return Err(BatchError::TooManyRows {
            max: limits.max_rows,
        });
    }
    let cols = table.iter().map(Vec::len).max().unwrap_or(0);
    if cols > limits.max_cols {
        return Err(BatchError::TooManyColumns {
            cols,
            max: limits.max_cols,
        });
    }
    for row in table {
        for cell in row {
            if let Cell::Text(s) = cell {
                let len = if s.len() <= limits.max_cell_chars {
                    s.len()
                } else {
                    s.chars().count()
                };
                if len > limits.max_cell_chars {
                    return Err(BatchError::CellTooLong {
                        len,
                        max: limits.max_cell_chars,
                    });
                }
            }
        }
    }
    Ok(())
}

fn normalize_date(text: &str) -> Result<String, String> {
    let error = || format!("日期 {text} 无效；请使用 YYYY-MM-DD 或 YYYY/M/D 的纯日期（不含时间）");
    let separator = if text.contains('/') { '/' } else { '-' };
    let parts: Vec<_> = text.split(separator).collect();
    if parts.len() != 3
        || parts[0].len() != 4
        || parts[1].is_empty()
        || parts[1].len() > 2
        || parts[2].is_empty()
        || parts[2].len() > 2
        || parts.iter().any(|s| !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(error());
    }
    let year = parts[0].parse::<u32>().map_err(|_| error())?;
    let month = parts[1].parse::<u32>().map_err(|_| error())?;
    let day = parts[2].parse::<u32>().map_err(|_| error())?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || day == 0 || day > max {
        return Err(error());
    }
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

// Grouping validates every nonempty amount before making a group-wide decision. A bad row
// must never silently reduce the reference aggregate and exempt other rows in its group.
fn grouping_references(
    cfg: &Config,
    opts: &Options,
    header: &[Cell],
    table: &[Vec<Cell>],
    amount_col: usize,
    reverser: Option<&Reverser>,
) -> Result<Option<Vec<Option<Decimal>>>, BatchError> {
    let Some(grouping) = &opts.vat_grouping else {
        return Ok(None);
    };
    let find = |name: &str| {
        if name.trim().is_empty() {
            return Err(BatchError::Grouping("人员与日期列名不能为空".into()));
        }
        let found: Vec<_> = header
            .iter()
            .enumerate()
            .filter(|(_, c)| c.text().trim() == name.trim())
            .map(|(i, _)| i)
            .collect();
        if found.len() != 1 {
            return Err(BatchError::Grouping(format!(
                "人员或日期标题必须唯一：{name}"
            )));
        }
        Ok(found[0])
    };
    let person_col = find(&grouping.person_column)?;
    let date_col = find(&grouping.date_column)?;
    if person_col == date_col || person_col == amount_col || date_col == amount_col {
        return Err(BatchError::Grouping(
            "人员、日期与金额必须使用不同的列".into(),
        ));
    }
    let fail = |row: usize, msg: String| BatchError::Grouping(format!("第 {} 行：{msg}", row + 2));
    let mut groups: std::collections::BTreeMap<(String, String), Vec<(usize, Decimal)>> =
        std::collections::BTreeMap::new();
    for (i, row) in table.iter().enumerate() {
        let amount = parse_amount(
            row.get(amount_col).unwrap_or(&Cell::Empty),
            cfg.rounding.scale,
        )
        .map_err(|e| fail(i, e))?;
        let Some(amount) = amount else {
            continue;
        };
        let person = row
            .get(person_col)
            .map(Cell::text)
            .unwrap_or_default()
            .trim()
            .to_owned();
        let raw_date = row.get(date_col).map(Cell::text).unwrap_or_default();
        let date = normalize_date(raw_date.trim()).map_err(|e| fail(i, e))?;
        if person.is_empty() || date.is_empty() {
            return Err(fail(i, "非空金额行必须有人员与日期".into()));
        }
        let net = match opts.kind {
            InputKind::NetIncome => forward(cfg, amount).map(|b| b.net_income),
            InputKind::TakeHome => reverser
                .expect("take-home reverser initialized")
                .reverse(amount)
                .map(|r| r.breakdown.net_income),
            InputKind::TotalCost => forward(cfg, amount).map(|_| amount),
        }
        .map_err(|e| fail(i, e.to_string()))?;
        groups.entry((person, date)).or_default().push((i, net));
    }
    let mut references = vec![None; table.len()];
    let sum = |values: &[Decimal]| -> Result<Decimal, BatchError> {
        values.iter().try_fold(Decimal::ZERO, |acc, v| {
            acc.checked_add(*v)
                .ok_or_else(|| BatchError::Grouping("分组金额合计溢出".into()))
        })
    };
    for ((person, date), members) in groups {
        let amounts: Vec<_> = members.iter().map(|(_, amount)| *amount).collect();
        let aggregate = sum(&amounts)?;
        let reference = if opts.kind != InputKind::TotalCost || !cfg.levied(aggregate) {
            aggregate
        } else {
            // Try the taxable regime consistently for every entry, then check the group's
            // recovered aggregate. A threshold gap has no self-consistent solution.
            let taxable_reference = if cfg.vat.threshold_enabled {
                cfg.vat
                    .threshold
                    .checked_add(cfg.unit())
                    .ok_or_else(|| BatchError::Grouping("起征点超出可计算范围".into()))?
            } else {
                Decimal::ZERO
            };
            let candidates: Result<Vec<_>, _> = members
                .iter()
                .map(|(_, cost)| {
                    from_total_cost_with_vat_reference(cfg, *cost, taxable_reference)
                        .map(|b| b.net_income)
                })
                .collect();
            match candidates {
                Ok(candidates) => {
                    let aggregate = sum(&candidates)?;
                    if !cfg.levied(aggregate) {
                        return Err(BatchError::Grouping(format!(
                            "人员 {person} 日期 {date} 的企业总支出无一致的起征点解"
                        )));
                    }
                    aggregate
                }
                Err(e) => {
                    return Err(BatchError::Grouping(format!(
                        "人员 {person} 日期 {date}：{e}"
                    )))
                }
            }
        };
        for (index, _) in members {
            references[index] = Some(reference);
        }
    }
    Ok(Some(references))
}

pub fn compute(
    cfg: &Config,
    input: &[u8],
    format: Format,
    opts: &Options,
) -> Result<Computed, BatchError> {
    if input.len() > opts.limits.max_file_bytes {
        return Err(BatchError::FileTooLarge {
            size: input.len(),
            max: opts.limits.max_file_bytes,
        });
    }
    if format == Format::Spreadsheet {
        preserve_xlsx::check_size(input, opts.limits.max_file_bytes)?;
    }
    let mut table = match format {
        Format::Csv => read_csv(input)?,
        Format::Spreadsheet => read_spreadsheet(input)?,
    };
    // 去掉末尾完全空白的行
    while table
        .last()
        .is_some_and(|r| r.iter().all(|c| matches!(c, Cell::Empty)))
    {
        table.pop();
    }
    if table.is_empty() {
        return Err(BatchError::Empty);
    }
    enforce_table_limits(&table, &opts.limits)?;
    let header = table.remove(0);
    let find = |name: &str| header.iter().position(|c| c.text().trim() == name.trim());
    let col = match &opts.column {
        Some(c) if !c.trim().is_empty() => {
            find(c).ok_or_else(|| BatchError::ColumnNotFound(format!("“{}”", c.trim())))?
        }
        _ => {
            let names = opts.kind.column_aliases();
            names.iter().find_map(|n| find(n)).ok_or_else(|| {
                let quoted: Vec<String> = names.iter().map(|n| format!("“{n}”")).collect();
                BatchError::ColumnNotFound(quoted.join("或"))
            })?
        }
    };
    // 原表列数：取标题与各数据行的最大宽度
    let width = table
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(header.len()))
        .max()
        .unwrap_or(0);
    let mut header = header;
    header.resize(width, Cell::Empty);

    let rev = match opts.kind {
        InputKind::TakeHome => Some(Reverser::new(cfg)?),
        _ => {
            cfg.validate()?;
            None
        }
    };
    let group_references = grouping_references(cfg, opts, &header, &table, col, rev.as_ref())?;
    let columns = available_columns(cfg, opts.kind);
    let n = columns.len();
    let mut summary = Summary::default();
    let mut rows = Vec::with_capacity(table.len());
    for (row_index, mut orig) in table.into_iter().enumerate() {
        orig.resize(width, Cell::Empty);
        summary.rows += 1;
        let amount = match parse_amount(&orig[col], cfg.rounding.scale) {
            Ok(None) => {
                summary.empty += 1;
                rows.push((orig, vec![Out::Empty; n]));
                continue;
            }
            Err(e) => {
                summary.errors += 1;
                rows.push((orig, error_values(&columns, STATUS_ERROR, &e)));
                continue;
            }
            Ok(Some(a)) => a,
        };
        let reference = group_references.as_ref().and_then(|refs| refs[row_index]);
        let res: Result<RowResult, CalcError> = match (opts.kind, &rev) {
            (InputKind::TakeHome, Some(rev)) => rev.reverse(amount).and_then(|mut r| {
                if let Some(reference) = reference {
                    r.breakdown =
                        forward_with_vat_reference(cfg, r.breakdown.net_income, reference)?;
                }
                let mut notes = Vec::new();
                if !r.other_solutions.is_empty() {
                    let s: Vec<String> = r.other_solutions.iter().map(|d| d.to_string()).collect();
                    notes.push(format!("另有不含增值税收入 {} 亦可精确匹配", s.join("、")));
                }
                Ok(RowResult {
                    status: r.status_text(),
                    diff: Some(r.diff),
                    breakdown: r.breakdown,
                    notes,
                })
            }),
            (InputKind::NetIncome, _) => reference
                .map_or_else(
                    || forward(cfg, amount),
                    |reference| forward_with_vat_reference(cfg, amount, reference),
                )
                .map(|b| RowResult {
                    breakdown: b,
                    status: STATUS_DONE.into(),
                    diff: None,
                    notes: Vec::new(),
                }),
            _ => reference
                .map_or_else(
                    || from_total_cost(cfg, amount),
                    |reference| from_total_cost_with_vat_reference(cfg, amount, reference),
                )
                .map(|b| RowResult {
                    breakdown: b,
                    status: STATUS_DONE.into(),
                    diff: None,
                    notes: Vec::new(),
                }),
        };
        let out = match res {
            Ok(r) => {
                if r.diff.is_some_and(|d| !d.is_zero()) {
                    summary.inexact += 1;
                } else {
                    summary.exact += 1;
                }
                result_values(&columns, &r)
            }
            Err(e @ CalcError::CostUnreachable(_)) => {
                summary.unreachable_cost += 1;
                error_values(&columns, STATUS_UNREACHABLE, &e.to_string())
            }
            Err(e) => {
                summary.errors += 1;
                error_values(&columns, STATUS_ERROR, &e.to_string())
            }
        };
        rows.push((orig, out));
    }
    Ok(Computed {
        kind: opts.kind,
        format,
        summary,
        header,
        original_xlsx: (format == Format::Spreadsheet && preserve_xlsx::is_xlsx(input))
            .then(|| input.to_vec()),
        csv_encoding: CsvEncoding::detect(input, format),
        money_scale: cfg.rounding.scale,
        columns,
        rows,
    })
}

/// 批量计算：读取输入文件字节，按导出列（默认全部列）返回新文件（输出格式与输入相同）。
pub fn process(
    cfg: &Config,
    input: &[u8],
    format: Format,
    opts: &Options,
) -> Result<Output, BatchError> {
    let computed = compute(cfg, input, format, opts)?;
    let columns = opts
        .columns
        .clone()
        .unwrap_or_else(|| computed.available_columns().to_vec());
    let bytes = computed.render(&columns, format)?;
    let preview = if opts.preview_rows == 0 {
        Vec::new()
    } else {
        computed.preview(&columns, opts.preview_rows)
    };
    Ok(Output {
        bytes,
        format,
        summary: computed.summary,
        preview,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use calamine::{Data, Reader};
    use rust_decimal_macros::dec;
    use std::io::Cursor;

    #[test]
    fn gb18030_encoding_failure_is_explicit() {
        // WHATWG GB18030 deliberately excludes U+E5E5; never emit a replacement entity.
        let value = "备注\u{e5e5}".as_bytes().to_vec();
        assert!(
            encoding_rs::GB18030
                .encode(std::str::from_utf8(&value).unwrap())
                .2
        );
        assert!(matches!(
            CsvEncoding::Gb18030.encode(value),
            Err(BatchError::Write(_))
        ));
    }

    #[test]
    fn amount_parsing() {
        assert_eq!(
            parse_amount(&Cell::Text(" ¥8,397.00元 ".into()), 2),
            Ok(Some(dec!(8397.00)))
        );
        assert_eq!(
            parse_amount(&Cell::Number(0.1 + 0.2), 2),
            Ok(Some(dec!(0.30)))
        );
        assert_eq!(
            parse_amount(&Cell::Number(1234.56), 2),
            Ok(Some(dec!(1234.56)))
        );
        assert_eq!(parse_amount(&Cell::Empty, 2), Ok(None));
        assert_eq!(parse_amount(&Cell::Text("  ".into()), 2), Ok(None));
        assert!(parse_amount(&Cell::Text("1.234".into()), 2).is_err());
        assert!(parse_amount(&Cell::Number(1.234), 2).is_err());
        assert!(parse_amount(&Cell::Text("abc".into()), 2).is_err());
    }

    #[test]
    fn file_names() {
        assert_eq!(
            Format::from_file_name("a.XLSX").unwrap(),
            Format::Spreadsheet
        );
        assert_eq!(Format::from_file_name("a.csv").unwrap(), Format::Csv);
        assert!(Format::from_file_name("a.txt").is_err());
        assert_eq!(
            output_file_name("dir/工资.xls").unwrap(),
            "工资_测算结果.xlsx"
        );
        assert_eq!(output_file_name("b.csv").unwrap(), "b_测算结果.csv");
        assert_eq!(
            Format::from_output_file_name("out.CSV").unwrap(),
            Format::Csv
        );
        assert_eq!(
            Format::from_output_file_name(r"dir\结果.XLSX").unwrap(),
            Format::Spreadsheet
        );
        assert!(Format::from_output_file_name("结果.xls").is_err());
        assert!(Format::from_output_file_name("结果.ods").is_err());
        assert!(Format::from_output_file_name("结果.txt").is_err());
        assert!(Format::from_output_file_name("无扩展名").is_err());
    }

    #[test]
    fn negative_money_is_not_formula_escaped() {
        let computed = Computed {
            kind: InputKind::NetIncome,
            format: Format::Csv,
            summary: Summary::default(),
            original_xlsx: None,
            csv_encoding: CsvEncoding::Utf8Bom,
            money_scale: 2,
            header: vec![Cell::Text("=抬头".into()), Cell::Number(-12.5)],
            columns: vec![ExportColumn::NetIncome, ExportColumn::Notes],
            rows: vec![(
                vec![Cell::Text("=1+1".into()), Cell::Number(-12.5)],
                vec![Out::Money(dec!(-1.50)), Out::Text("+备注".into())],
            )],
        };
        let cols = [
            ExportColumn::Source,
            ExportColumn::NetIncome,
            ExportColumn::Notes,
        ];
        let csv = String::from_utf8(computed.render(&cols, Format::Csv).unwrap()).unwrap();
        let csv = csv.trim_start_matches('\u{feff}');
        let mut rdr = csv::Reader::from_reader(csv.as_bytes());
        let h = rdr.headers().unwrap().clone();
        assert_eq!(&h[0], "'=抬头");
        assert_eq!(&h[1], "-12.5");
        let rec = rdr.records().next().unwrap().unwrap();
        assert_eq!(&rec[0], "'=1+1");
        assert_eq!(&rec[1], "-12.5");
        assert_eq!(&rec[2], "-1.50");
        assert_eq!(&rec[3], "'+备注");

        let rows = {
            let bytes = computed.render(&cols, Format::Spreadsheet).unwrap();
            let mut wb = calamine::open_workbook_auto_from_rs(Cursor::new(bytes)).unwrap();
            let range = wb.worksheet_range_at(0).unwrap().unwrap();
            range.rows().map(|r| r.to_vec()).collect::<Vec<_>>()
        };
        assert_eq!(rows[0][0], Data::String("'=抬头".into()));
        match &rows[0][1] {
            Data::Float(n) => assert!((n - -12.5).abs() < 1e-9),
            Data::Int(n) => assert_eq!(n, &-12),
            other => panic!("负数表头应是数值：{other:?}"),
        }
        assert_eq!(rows[1][0], Data::String("'=1+1".into()));
        match &rows[1][1] {
            Data::Float(n) => assert!((n - -12.5).abs() < 1e-9),
            other => panic!("负数原值应是数值：{other:?}"),
        }
        match &rows[1][2] {
            Data::Float(n) => assert!((n - -1.5).abs() < 1e-9, "{n}"),
            other => panic!("负数金额应是数值：{other:?}"),
        }
        assert_eq!(rows[1][3], Data::String("'+备注".into()));
    }
}
