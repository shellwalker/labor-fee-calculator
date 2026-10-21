//! 劳务报酬批量反算。
//!
//! 读取 xlsx/xls/ods（第一个工作表）或 csv，取首行标题为“到手金额”（可配置）的列，
//! 对每一行反算，并在原表右侧追加各项结果列，输出新文件（表格输入输出 xlsx，csv 输入输出 csv）。
//!
//! 计算全部使用十进制；仅在读取 Excel 数值单元格、写出 Excel 数值单元格这两个 I/O 边界上与 f64 互转
//! （Excel 文件格式本身以浮点存储数值），读入时会校验小数位不超过两位。

use labor_fee_core::{CalcError, Config, Decimal, ReverseOutcome, Reverser};
use rust_decimal::prelude::ToPrimitive;
use std::io::Cursor;
use std::str::FromStr;
use thiserror::Error;

/// 默认读取的列标题。
pub const DEFAULT_COLUMN: &str = "到手金额";

/// 批量处理错误（文件级）。行级错误写入结果表的“校准状态”列，不中断处理。
#[derive(Debug, Error)]
pub enum BatchError {
    #[error("不支持的文件类型：{0}（支持 .xlsx/.xlsm/.xls/.ods/.csv）")]
    UnsupportedFormat(String),
    #[error("表格为空")]
    Empty,
    #[error("首行未找到标题为“{0}”的列")]
    ColumnNotFound(String),
    #[error("读取文件失败：{0}")]
    Read(String),
    #[error("写出文件失败：{0}")]
    Write(String),
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
            Format::Csv => "text/csv; charset=utf-8",
            Format::Spreadsheet => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            }
        }
    }
}

/// 根据输入文件名生成输出文件名：`原名_反算结果.xlsx/csv`。
pub fn output_file_name(input_name: &str) -> Result<String, BatchError> {
    let fmt = Format::from_file_name(input_name)?;
    let stem = input_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(input_name);
    let stem = stem.rsplit(['/', '\\']).next().unwrap_or(stem);
    Ok(format!("{stem}_反算结果.{}", fmt.output_extension()))
}

/// 批量处理选项。
#[derive(Debug, Clone)]
pub struct Options {
    /// 读取的列标题（首行精确匹配，忽略首尾空白）。
    pub column: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            column: DEFAULT_COLUMN.to_string(),
        }
    }
}

/// 处理统计。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// 数据行数（不含标题行）。
    pub rows: usize,
    /// 精确反算行数。
    pub exact: usize,
    /// 不可精确达到行数。
    pub inexact: usize,
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
            let d = Decimal::from_str(&cleaned)
                .or_else(|_| Decimal::from_scientific(&cleaned))
                .map_err(|_| format!("无法解析金额：{s}"))?;
            (d, false)
        }
    };
    let r = d.round_dp(scale);
    if r != d && !(tolerant && (r - d).abs() < Decimal::new(1, 6)) {
        return Err(format!("金额 {d} 超过 {scale} 位小数"));
    }
    Ok(Some(r))
}

fn result_headers(cfg: &Config) -> Vec<String> {
    let mut h: Vec<String> = ["企业总成本", "不含税开票收入", "增值税"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if cfg.surcharge.items.len() > 1 {
        h.extend(cfg.surcharge.items.iter().map(|i| i.name.clone()));
    }
    h.extend(
        [
            "附加税合计",
            "收入额",
            "减除费用",
            "应纳税所得额",
            "预扣率",
            "速算扣除数",
            "代扣个税",
            "正算到手（校验）",
            "校准状态",
            "差额",
            "反算备注",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    h
}

fn result_values(cfg: &Config, r: &ReverseOutcome) -> Vec<Out> {
    let b = &r.breakdown;
    let mut v = vec![
        Out::Money(b.total_cost),
        Out::Money(b.net_revenue),
        Out::Money(b.vat),
    ];
    if cfg.surcharge.items.len() > 1 {
        v.extend(b.surcharges.iter().map(|l| Out::Money(l.amount)));
    }
    v.extend([
        Out::Money(b.surcharge_total),
        Out::Money(b.iit.income),
        Out::Money(b.iit.deduction),
        Out::Money(b.iit.taxable_income),
        Out::Rate(b.iit.rate),
        Out::Money(b.iit.quick_deduction),
        Out::Money(b.iit.tax),
        Out::Money(b.take_home),
        Out::Text(
            if r.exact {
                "精确"
            } else {
                "不可精确达到"
            }
            .into(),
        ),
        Out::Money(r.diff),
    ]);
    let mut notes = Vec::new();
    if b.vat_exempt {
        notes.push("未达起征点，免增值税及附加税".to_string());
    }
    if !r.other_solutions.is_empty() {
        let s: Vec<String> = r.other_solutions.iter().map(|d| d.to_string()).collect();
        notes.push(format!("另有总成本 {} 亦可精确达到", s.join("、")));
    }
    v.push(Out::Text(notes.join("；")));
    v
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
                    other => Cell::Text(other.to_string()),
                })
                .collect()
        })
        .collect())
}

fn write_csv(
    header: &[Cell],
    rows: &[(Vec<Cell>, Vec<Out>)],
    extra: &[String],
) -> Result<Vec<u8>, BatchError> {
    let mut buf = "\u{feff}".as_bytes().to_vec(); // BOM，便于 Excel 直接打开
    {
        let mut w = csv::WriterBuilder::new()
            .flexible(true)
            .from_writer(&mut buf);
        let h: Vec<String> = header
            .iter()
            .map(Cell::text)
            .chain(extra.iter().cloned())
            .collect();
        w.write_record(&h)
            .map_err(|e| BatchError::Write(e.to_string()))?;
        for (orig, out) in rows {
            let rec: Vec<String> = orig
                .iter()
                .map(Cell::text)
                .chain(out.iter().map(|o| match o {
                    Out::Empty => String::new(),
                    Out::Money(d) => d.to_string(),
                    Out::Rate(d) => format!("{}%", (*d * Decimal::ONE_HUNDRED).normalize()),
                    Out::Text(s) => s.clone(),
                }))
                .collect();
            w.write_record(&rec)
                .map_err(|e| BatchError::Write(e.to_string()))?;
        }
        w.flush().map_err(|e| BatchError::Write(e.to_string()))?;
    }
    Ok(buf)
}

fn write_xlsx(
    header: &[Cell],
    rows: &[(Vec<Cell>, Vec<Out>)],
    extra: &[String],
) -> Result<Vec<u8>, BatchError> {
    use rust_xlsxwriter::{Format as XFormat, Workbook};
    let werr = |e: rust_xlsxwriter::XlsxError| BatchError::Write(e.to_string());
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("反算结果").map_err(werr)?;
    let bold = XFormat::new().set_bold();
    let money = XFormat::new().set_num_format("0.00");
    let pct = XFormat::new().set_num_format("0%");
    let ncols = header.len();
    let write_cell = |ws: &mut rust_xlsxwriter::Worksheet, r: u32, c: u16, cell: &Cell| match cell {
        Cell::Empty => Ok(()),
        Cell::Text(s) => ws.write_string(r, c, s).map(|_| ()),
        Cell::Number(n) => ws.write_number(r, c, *n).map(|_| ()),
    };
    for (c, cell) in header.iter().enumerate() {
        let c = c as u16;
        match cell {
            Cell::Empty => {}
            _ => {
                ws.write_string_with_format(0, c, cell.text(), &bold)
                    .map_err(werr)?;
            }
        }
    }
    for (i, h) in extra.iter().enumerate() {
        let c = (ncols + i) as u16;
        ws.write_string_with_format(0, c, h, &bold).map_err(werr)?;
        ws.set_column_width(c, 14).map_err(werr)?;
    }
    for (ri, (orig, out)) in rows.iter().enumerate() {
        let r = (ri + 1) as u32;
        for (c, cell) in orig.iter().enumerate().take(ncols) {
            write_cell(ws, r, c as u16, cell).map_err(werr)?;
        }
        for (i, o) in out.iter().enumerate() {
            let c = (ncols + i) as u16;
            match o {
                Out::Empty => {}
                Out::Money(d) => {
                    ws.write_number_with_format(r, c, d.to_f64().unwrap_or(0.0), &money)
                        .map_err(werr)?;
                }
                Out::Rate(d) => {
                    ws.write_number_with_format(r, c, d.to_f64().unwrap_or(0.0), &pct)
                        .map_err(werr)?;
                }
                Out::Text(s) => {
                    ws.write_string(r, c, s).map_err(werr)?;
                }
            }
        }
    }
    wb.save_to_buffer().map_err(werr)
}

/// 批量反算：读取输入文件字节，返回追加结果列后的新文件。
pub fn process(
    cfg: &Config,
    input: &[u8],
    format: Format,
    opts: &Options,
) -> Result<Output, BatchError> {
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
    let header = table.remove(0);
    let col = header
        .iter()
        .position(|c| c.text().trim() == opts.column.trim())
        .ok_or_else(|| BatchError::ColumnNotFound(opts.column.clone()))?;
    // 原表列数：取标题与各数据行的最大宽度，结果列统一追加在其右侧
    let width = table
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(header.len()))
        .max()
        .unwrap_or(0);
    let mut header = header;
    header.resize(width, Cell::Empty);

    let rev = Reverser::new(cfg)?;
    let extra = result_headers(cfg);
    let mut summary = Summary::default();
    let mut rows = Vec::with_capacity(table.len());
    for mut orig in table {
        orig.resize(width, Cell::Empty);
        summary.rows += 1;
        let err_row = |msg: String| {
            let mut v = vec![Out::Empty; extra.len()];
            v[extra.len() - 3] = Out::Text(format!("错误：{msg}"));
            v
        };
        let out = match parse_amount(&orig[col], cfg.rounding.scale) {
            Ok(None) => {
                summary.empty += 1;
                vec![Out::Empty; extra.len()]
            }
            Err(e) => {
                summary.errors += 1;
                err_row(e)
            }
            Ok(Some(t)) => match rev.reverse(t) {
                Ok(r) => {
                    if r.exact {
                        summary.exact += 1;
                    } else {
                        summary.inexact += 1;
                    }
                    result_values(cfg, &r)
                }
                Err(e) => {
                    summary.errors += 1;
                    err_row(e.to_string())
                }
            },
        };
        rows.push((orig, out));
    }
    let bytes = match format {
        Format::Csv => write_csv(&header, &rows, &extra)?,
        Format::Spreadsheet => write_xlsx(&header, &rows, &extra)?,
    };
    Ok(Output {
        bytes,
        format,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

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
            "工资_反算结果.xlsx"
        );
        assert_eq!(output_file_name("b.csv").unwrap(), "b_反算结果.csv");
    }
}
