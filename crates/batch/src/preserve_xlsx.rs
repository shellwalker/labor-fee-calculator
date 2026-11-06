//! Append cells to the original OOXML package. Untouched entries are copied compressed;
//! original worksheet XML is edited by byte offsets so existing cell/style/width markup survives.
use super::{decimal_number, money_format, BatchError, Computed, Out, Slot};
use quick_xml::{events::Event, Reader};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

fn error(e: impl std::fmt::Display) -> BatchError {
    BatchError::Write(e.to_string())
}
fn read(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String, BatchError> {
    let mut s = String::new();
    archive
        .by_name(name)
        .map_err(error)?
        .read_to_string(&mut s)
        .map_err(error)?;
    Ok(s)
}
pub(super) fn is_xlsx(bytes: &[u8]) -> bool {
    ZipArchive::new(Cursor::new(bytes)).is_ok_and(|mut z| z.by_name("xl/workbook.xml").is_ok())
}
pub(super) fn check_size(bytes: &[u8], max: usize) -> Result<(), BatchError> {
    if !bytes.starts_with(b"PK") {
        return Ok(());
    }
    let mut z = ZipArchive::new(Cursor::new(bytes)).map_err(|e| BatchError::Read(e.to_string()))?;
    let limit = (max as u64).saturating_mul(20);
    let mut total = 0u64;
    for i in 0..z.len() {
        total = total
            .checked_add(z.by_index(i).map_err(error)?.size())
            .ok_or_else(|| BatchError::Read("表格解压大小溢出".into()))?;
        if total > limit {
            return Err(BatchError::Read(format!(
                "表格解压大小超过上限 {limit} 字节"
            )));
        }
    }
    Ok(())
}
fn attr(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
}
fn ensure_unprefixed(xml: &str) -> Result<(), BatchError> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event().map_err(error)? {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref().contains(&b':') => {
                return Err(error(
                    "暂不支持带命名空间前缀的工作表或样式元素；无法保真追加",
                ))
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}
fn styles_path(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Result<String, BatchError> {
    let rels = read(archive, "xl/_rels/workbook.xml.rels")?;
    let mut reader = Reader::from_str(&rels);
    loop {
        match reader.read_event().map_err(error)? {
            Event::Start(e) | Event::Empty(e)
                if e.local_name().as_ref() == b"Relationship"
                    && attr(&e, b"Type").is_some_and(|t| t.ends_with("/styles")) =>
            {
                let target = attr(&e, b"Target").ok_or_else(|| error("样式关系缺少路径"))?;
                if target.contains("..") || attr(&e, b"TargetMode").as_deref() == Some("External") {
                    return Err(error("不支持的样式路径"));
                }
                return Ok(if target.starts_with('/') {
                    target.trim_start_matches('/').into()
                } else {
                    format!("xl/{target}")
                });
            }
            Event::Eof => return Err(error("工作簿缺少样式关系；无法保真追加")),
            _ => {}
        }
    }
}
fn first_sheet(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Result<String, BatchError> {
    let workbook = read(archive, "xl/workbook.xml")?;
    let mut r = Reader::from_str(&workbook);
    let id = loop {
        match r.read_event().map_err(error)? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"sheet" => {
                break e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.local_name().as_ref() == b"id")
                    .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                    .ok_or_else(|| error("工作表缺少关系标识"))?
            }
            Event::Eof => return Err(error("工作簿没有工作表")),
            _ => {}
        }
    };
    let rels = read(archive, "xl/_rels/workbook.xml.rels")?;
    let mut r = Reader::from_str(&rels);
    loop {
        match r.read_event().map_err(error)? {
            Event::Start(e) | Event::Empty(e)
                if e.local_name().as_ref() == b"Relationship"
                    && attr(&e, b"Id").as_deref() == Some(&id) =>
            {
                let target = attr(&e, b"Target").ok_or_else(|| error("工作表缺少路径"))?;
                if attr(&e, b"TargetMode").as_deref() == Some("External") || target.contains("..") {
                    return Err(error("不支持的工作表路径"));
                }
                return Ok(if target.starts_with('/') {
                    target.trim_start_matches('/').to_owned()
                } else {
                    format!("xl/{target}")
                });
            }
            Event::Eof => return Err(error("找不到工作表关系")),
            _ => {}
        }
    }
}
fn extent(sheet: &str) -> Result<(usize, usize, usize, usize), BatchError> {
    let mut r = Reader::from_str(sheet);
    let mut edge = 0;
    let mut bottom = 0;
    let mut left = usize::MAX;
    let mut top = usize::MAX;
    loop {
        match r.read_event().map_err(error)? {
            Event::Start(e) | Event::Empty(e) => {
                let local = e.local_name();
                if local.as_ref() == b"row" {
                    if let Some(row) = attr(&e, b"r").and_then(|s| s.parse::<usize>().ok()) {
                        bottom = bottom.max(row);
                    }
                }
                let key = match local.as_ref() {
                    b"dimension" | b"mergeCell" => Some(b"ref".as_slice()),
                    b"c" => Some(b"r".as_slice()),
                    _ => None,
                };
                if let Some(value) = key.and_then(|key| attr(&e, key)) {
                    for coordinate in value.split(':') {
                        let col = coordinate.bytes().take_while(u8::is_ascii_alphabetic).fold(
                            0usize,
                            |n, c| {
                                n.saturating_mul(26)
                                    .saturating_add((c.to_ascii_uppercase() - b'A' + 1) as usize)
                            },
                        );
                        let row = coordinate
                            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
                            .parse::<usize>()
                            .map_err(|_| error("无效的工作表单元格坐标"))?;
                        if col == 0 || row == 0 {
                            return Err(error("无效的工作表单元格坐标"));
                        }
                        edge = edge.max(col);
                        bottom = bottom.max(row);
                        left = left.min(col);
                        top = top.min(row);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((edge, bottom, left, top))
}
fn escape(s: &str) -> String {
    quick_xml::escape::escape(s).into_owned()
}
fn col_name(mut col: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII")
}
fn text_cell(reference: &str, text: &str) -> String {
    let mut safe = String::new();
    for (index, c) in text.char_indices() {
        let rest = &text.as_bytes()[index..];
        if c == '_'
            && rest.len() >= 7
            && rest[1] == b'x'
            && rest[2..6].iter().all(u8::is_ascii_hexdigit)
            && rest[6] == b'_'
        {
            safe.push_str("_x005F_");
        } else if matches!(c,'\u{0}'..='\u{8}'|'\u{b}'|'\u{c}'|'\u{e}'..='\u{1f}'|'\u{fffe}'|'\u{ffff}')
        {
            safe.push_str(&format!("_x{:04X}_", c as u32));
        } else {
            safe.push(c);
        }
    }
    format!(
        "<c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
        escape(&safe)
    )
}
// OOXML numFmtId and collection counts use unsignedInt (32 bits), even on
// hosts where usize is wider. Validate before doing index/count arithmetic.
fn style_uint(value: &str) -> Result<usize, BatchError> {
    value
        .parse::<u32>()
        .map(|n| n as usize)
        .map_err(|_| error("样式编号或数量超出 OOXML unsignedInt 范围"))
}
fn increment_style_uint(value: usize, increment: usize) -> Result<usize, BatchError> {
    value
        .checked_add(increment)
        .filter(|n| *n <= u32::MAX as usize)
        .ok_or_else(|| error("追加样式后编号或数量超出 OOXML unsignedInt 范围"))
}
fn add_styles(styles: &str, scale: u32) -> Result<(String, usize), BatchError> {
    ensure_unprefixed(styles)?;
    let mut r = Reader::from_str(styles);
    let mut max_id = 163;
    let mut xf_count = None;
    let mut num_formats = None;
    let mut empty_num_formats = None;
    let mut xfs_end = None;
    let mut num_end = None;
    loop {
        let start = r.buffer_position() as usize;
        match r.read_event().map_err(error)? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"numFmt" => {
                let id = attr(&e, b"numFmtId").ok_or_else(|| error("数字格式缺少编号"))?;
                max_id = max_id.max(style_uint(&id)?);
            }
            Event::Empty(e) if e.local_name().as_ref() == b"numFmts" => {
                if let Some(count) = attr(&e, b"count") {
                    style_uint(&count)?;
                }
                empty_num_formats = Some((start, r.buffer_position() as usize))
            }
            Event::Start(e) if e.local_name().as_ref() == b"numFmts" => {
                num_formats = Some((
                    start,
                    r.buffer_position() as usize,
                    attr(&e, b"count")
                        .map(|s| style_uint(&s))
                        .transpose()?
                        .unwrap_or(0),
                ))
            }
            Event::Start(e) if e.local_name().as_ref() == b"cellXfs" => {
                xf_count = Some((
                    start,
                    r.buffer_position() as usize,
                    style_uint(&attr(&e, b"count").ok_or_else(|| error("样式缺少数量"))?)?,
                ))
            }
            Event::End(e) if e.local_name().as_ref() == b"numFmts" => num_end = Some(start),
            Event::End(e) if e.local_name().as_ref() == b"cellXfs" => xfs_end = Some(start),
            Event::Eof => break,
            _ => {}
        }
    }
    let (xf_start, xf_tag_end, count) =
        xf_count.ok_or_else(|| error("不支持缺少 cellXfs 的样式表"))?;
    let num_id = increment_style_uint(max_id, 1)?;
    let new_xf_count = increment_style_uint(count, 2)?;
    let fmt = format!(
        "<numFmt numFmtId=\"{num_id}\" formatCode=\"{}\"/>",
        money_format(scale)
    );
    let mut edits = vec![(xf_start, xf_tag_end, format!("<cellXfs count=\"{new_xf_count}\">")), (xfs_end.ok_or_else(||error("样式表不完整"))?,xfs_end.unwrap(),format!("<xf numFmtId=\"{num_id}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/><xf numFmtId=\"9\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>"))];
    if let Some((start, end, count)) = num_formats {
        edits.push((
            start,
            end,
            format!("<numFmts count=\"{}\">", increment_style_uint(count, 1)?),
        ));
        let at = num_end.ok_or_else(|| error("数字格式表不完整"))?;
        edits.push((at, at, fmt));
    } else if let Some((start, end)) = empty_num_formats {
        edits.push((start, end, format!("<numFmts count=\"1\">{fmt}</numFmts>")));
    } else {
        // numFmts precedes fonts in the stylesheet schema.
        let at = styles
            .find("<fonts")
            .ok_or_else(|| error("样式表缺少字体"))?;
        edits.push((at, at, format!("<numFmts count=\"1\">{fmt}</numFmts>")));
    }
    Ok((apply(styles, edits), count))
}
fn apply(source: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by_key(|e| e.0);
    let mut out = source.to_owned();
    for (start, end, text) in edits.into_iter().rev() {
        out.replace_range(start..end, &text);
    }
    out
}
fn flush_before(cells: &mut BTreeMap<usize, String>, number: usize) -> String {
    let missing: Vec<_> = cells.range(..number).map(|(&n, _)| n).collect();
    missing
        .into_iter()
        .map(|n| format!("<row r=\"{n}\">{}</row>", cells.remove(&n).unwrap()))
        .collect()
}

pub(super) fn append(
    computed: &Computed,
    input: &[u8],
    plan: &[Slot],
) -> Result<Vec<u8>, BatchError> {
    if !plan.iter().any(|s| matches!(s, Slot::Result(_))) {
        return Ok(input.to_vec());
    }
    let mut archive = ZipArchive::new(Cursor::new(input)).map_err(error)?;
    let sheet_path = first_sheet(&mut archive)?;
    let sheet = read(&mut archive, &sheet_path)?;
    ensure_unprefixed(&sheet)?;
    let style_path = styles_path(&mut archive)?;
    let styles = read(&mut archive, &style_path)?;
    let (styles, money_style) = add_styles(&styles, computed.money_scale)?;
    let results: Vec<_> = plan
        .iter()
        .filter_map(|s| {
            if let Slot::Result(i) = s {
                Some(*i)
            } else {
                None
            }
        })
        .collect();
    let width = computed.header.len();
    if width + results.len() > 16384 || computed.rows.len() >= 1048576 {
        return Err(error("结果超出 Excel 行列上限"));
    }
    // Calamine's range begins at the first used row/column. Keep the absolute origin.
    use calamine::Reader as _;
    let mut wb =
        calamine::open_workbook_auto_from_rs(Cursor::new(input.to_vec())).map_err(error)?;
    let range = wb
        .worksheet_range_at(0)
        .ok_or_else(|| error("工作表为空"))?
        .map_err(error)?;
    let (row_offset, col_offset) = range.start().unwrap_or((0, 0));
    let first_row = row_offset as usize + 1;
    let (right, bottom, left, top) = extent(&sheet)?;
    let first_col = (col_offset as usize + width).max(right);
    if first_col + results.len() > 16384 || first_row + computed.rows.len() > 1048576 {
        return Err(error("结果超出 Excel 行列上限"));
    }
    let mut cells = BTreeMap::new();
    for row in 0..=computed.rows.len() {
        let row_number = first_row + row;
        let mut xml = String::new();
        for (j, &index) in results.iter().enumerate() {
            let reference = format!("{}{row_number}", col_name(first_col + j));
            if row == 0 {
                xml.push_str(&text_cell(&reference, computed.columns[index].title()));
                continue;
            }
            match &computed.rows[row - 1].1[index] {
                Out::Empty => {}
                Out::Text(text) => xml.push_str(&text_cell(&reference, text)),
                Out::Money(value) => {
                    // Validate the numeric boundary even though XML retains the exact decimal text.
                    decimal_number(*value)?;
                    xml.push_str(&format!(
                        "<c r=\"{reference}\" s=\"{money_style}\"><v>{value}</v></c>"
                    ));
                }
                Out::Rate(value) => {
                    decimal_number(*value)?;
                    xml.push_str(&format!(
                        "<c r=\"{reference}\" s=\"{}\"><v>{value}</v></c>",
                        money_style + 1
                    ));
                }
            }
        }
        cells.insert(row_number, xml);
    }
    let mut r = Reader::from_str(&sheet);
    let mut edits = Vec::new();
    let mut current_row = None;
    loop {
        let start = r.buffer_position() as usize;
        match r.read_event().map_err(error)? {
            Event::Start(e) if e.local_name().as_ref() == b"row" => {
                current_row = attr(&e, b"r").and_then(|s| s.parse::<usize>().ok());
                if let Some(number) = current_row {
                    let xml = flush_before(&mut cells, number);
                    if !xml.is_empty() {
                        edits.push((start, start, xml));
                    }
                }
            }
            Event::Empty(e) if e.local_name().as_ref() == b"row" => {
                if let Some(number) = attr(&e, b"r").and_then(|s| s.parse::<usize>().ok()) {
                    let xml = flush_before(&mut cells, number);
                    if !xml.is_empty() {
                        edits.push((start, start, xml));
                    }
                    if let Some(xml) = cells.remove(&number) {
                        let tag = &sheet[start..r.buffer_position() as usize];
                        edits.push((
                            start,
                            r.buffer_position() as usize,
                            format!("{}>{xml}</row>", tag.trim_end_matches("/>")),
                        ));
                    }
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"row" => {
                if let Some(number) = current_row.take() {
                    if let Some(xml) = cells.remove(&number) {
                        edits.push((start, start, xml));
                    }
                }
            }
            Event::Empty(e) if e.local_name().as_ref() == b"dimension" => {
                if !results.is_empty() {
                    let end = format!(
                        "{}{}",
                        col_name(first_col + results.len() - 1),
                        (first_row + computed.rows.len()).max(bottom)
                    );
                    let origin = format!(
                        "{}{}",
                        col_name((col_offset as usize).min(left.saturating_sub(1))),
                        first_row.min(top)
                    );
                    edits.push((
                        start,
                        r.buffer_position() as usize,
                        format!("<dimension ref=\"{origin}:{end}\"/>"),
                    ));
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"sheetData" => {
                let xml: String = std::mem::take(&mut cells)
                    .into_iter()
                    .map(|(n, xml)| format!("<row r=\"{n}\">{xml}</row>"))
                    .collect();
                if !xml.is_empty() {
                    edits.push((start, start, xml));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let sheet = apply(&sheet, edits);
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(error)?;
        if entry.name() == sheet_path || entry.name() == style_path {
            let replacement = if entry.name() == sheet_path {
                &sheet
            } else {
                &styles
            };
            let options = SimpleFileOptions::default().compression_method(entry.compression());
            output.start_file(entry.name(), options).map_err(error)?;
            output.write_all(replacement.as_bytes()).map_err(error)?;
        } else {
            output.raw_copy_file(entry).map_err(error)?;
        }
    }
    Ok(output.finish().map_err(error)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_rejects_malformed_xlsx_style_id_without_panicking() {
        let mut wb = rust_xlsxwriter::Workbook::new();
        wb.add_worksheet().write_string(0, 0, "到手金额").unwrap();
        let original = wb.save_to_buffer().unwrap();
        let mut archive = ZipArchive::new(Cursor::new(original.as_slice())).unwrap();
        let mut output = ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..archive.len() {
            let entry = archive.by_index(i).unwrap();
            if entry.name() == "xl/styles.xml" {
                output
                    .start_file(entry.name(), SimpleFileOptions::default())
                    .unwrap();
                output.write_all(format!("<styleSheet><numFmts count=\"1\"><numFmt numFmtId=\"{}\" formatCode=\"0.0\"/></numFmts><fonts/><cellXfs count=\"1\"><xf/></cellXfs></styleSheet>",usize::MAX).as_bytes()).unwrap();
            } else {
                output.raw_copy_file(entry).unwrap();
            }
        }
        let mut computed = super::super::compute(
            &super::super::Config::default(),
            "到手金额\n600\n".as_bytes(),
            super::super::Format::Csv,
            &super::super::Options::default(),
        )
        .unwrap();
        computed.original_xlsx = Some(output.finish().unwrap().into_inner());
        let result = std::panic::catch_unwind(|| {
            computed.render(
                &[super::super::ExportColumn::NetIncome],
                super::super::Format::Spreadsheet,
            )
        });
        assert!(matches!(result, Ok(Err(BatchError::Write(_)))));
    }

    #[test]
    fn malformed_style_integer_limits_return_errors_without_panicking() {
        let template = "<styleSheet><numFmts count=\"NUM_COUNT\"><numFmt numFmtId=\"NUM_ID\" formatCode=\"0.0\"/></numFmts><fonts count=\"1\"><font/></fonts><cellXfs count=\"XF_COUNT\"><xf/></cellXfs></styleSheet>";
        for (id, formats, xfs) in [
            (usize::MAX.to_string(), "1".into(), "1".into()),
            ("164".into(), usize::MAX.to_string(), "1".into()),
            ("164".into(), "1".into(), usize::MAX.to_string()),
            (u32::MAX.to_string(), "1".into(), "1".into()),
            ("164".into(), u32::MAX.to_string(), "1".into()),
            ("164".into(), "1".into(), (u32::MAX - 1).to_string()),
        ] {
            let xml = template
                .replace("NUM_COUNT", &formats)
                .replace("NUM_ID", &id)
                .replace("XF_COUNT", &xfs);
            let result = std::panic::catch_unwind(|| add_styles(&xml, 2));
            assert!(result.is_ok(), "malformed style must never panic: {xml}");
            assert!(
                matches!(result.unwrap(), Err(BatchError::Write(_))),
                "malformed style must be rejected: {xml}"
            );
        }
    }

    #[test]
    fn appended_text_escapes_xml_controls_and_ooxml_escape_sequences() {
        let cell = text_cell("A1", "备注\0_x000D_<>&");
        assert!(cell.contains("备注_x0000__x005F_x000D_&lt;&gt;&amp;"));
        assert!(!cell.contains('\0'));
    }
}
