use calamine::{Data, Reader};
use labor_fee_batch::{
    available_columns, compute, parse_columns, process, BatchError, ColumnLayout, ExportColumn,
    Format, InputKind, Limits, Options,
};
use labor_fee_core::{forward, Config};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::io::Cursor;
use std::str::FromStr;

fn make_xlsx() -> Vec<u8> {
    let mut wb = rust_xlsxwriter::Workbook::new();
    let ws = wb.add_worksheet();
    ws.write_string(0, 0, "姓名").unwrap();
    ws.write_string(0, 1, "身份证号").unwrap();
    ws.write_string(0, 2, "到手金额").unwrap();
    let data: [(&str, Option<f64>, Option<&str>); 6] = [
        ("张三", Some(8400.0), None),
        ("李四", Some(3360.0), None),
        ("王五", None, Some("12,345.67")),
        ("赵六", None, None),
        ("钱七", None, Some("abc")),
        ("孙八", Some(0.1 + 0.2), None),
    ];
    for (i, (name, num, text)) in data.iter().enumerate() {
        let r = (i + 1) as u32;
        ws.write_string(r, 0, *name).unwrap();
        ws.write_string(r, 1, "110101199001011234").unwrap();
        if let Some(n) = num {
            ws.write_number(r, 2, *n).unwrap();
        }
        if let Some(t) = text {
            ws.write_string(r, 2, *t).unwrap();
        }
    }
    wb.save_to_buffer().unwrap()
}

fn read_back(bytes: &[u8]) -> Vec<Vec<Data>> {
    let mut wb = calamine::open_workbook_auto_from_rs(Cursor::new(bytes.to_vec())).unwrap();
    let range = wb.worksheet_range_at(0).unwrap().unwrap();
    range.rows().map(|r| r.to_vec()).collect()
}

fn money(d: &Data) -> Decimal {
    match d {
        Data::Float(f) => Decimal::from_str(&f.to_string()).unwrap().round_dp(2),
        Data::Int(i) => Decimal::from(*i),
        other => panic!("非数值单元格：{other:?}"),
    }
}

#[test]
fn xlsx_roundtrip() {
    let cfg = Config::default();
    let out = process(&cfg, &make_xlsx(), Format::Spreadsheet, &Options::default()).unwrap();
    assert_eq!(out.summary.rows, 6);
    assert_eq!(out.summary.exact, 4);
    assert_eq!(out.summary.empty, 1);
    assert_eq!(out.summary.errors, 1);
    let rows = read_back(&out.bytes);
    let h: Vec<String> = rows[0].iter().map(|d| d.to_string()).collect();
    assert_eq!(&h[..4], &["姓名", "身份证号", "到手金额", "不含增值税收入"]);
    let col = |name: &str| h.iter().position(|x| x == name).unwrap();
    // 身份证号保持文本
    assert_eq!(rows[1][1], Data::String("110101199001011234".into()));
    for (r, expect) in [
        (1, dec!(8400)),
        (2, dec!(3360)),
        (3, dec!(12345.67)),
        (6, dec!(0.30)),
    ] {
        let net = money(&rows[r][col("不含增值税收入")]);
        assert_eq!(forward(&cfg, net).unwrap().take_home, expect);
        assert_eq!(money(&rows[r][col("税后实发金额")]), expect);
        assert_eq!(rows[r][col("测算状态")], Data::String("精确匹配".into()));
    }
    assert_eq!(money(&rows[1][col("企业总支出")]), dec!(10103.00));
    assert_eq!(money(&rows[1][col("不含增值税收入")]), dec!(10000));
    assert_eq!(rows[5][col("测算状态")].to_string(), "数据错误");
    assert!(rows[5][col("备注")].to_string().contains("无法解析金额"));
    assert_eq!(rows[4][col("企业总支出")], Data::Empty);
}

#[test]
fn csv_utf8_and_gbk() {
    let cfg = Config::default();
    let text = "序号,到手金额,备注\n1,8400,a\n2,\"1,000.00\",b\n3,,c\n";
    for bytes in [
        text.as_bytes().to_vec(),
        encoding_rs::GBK.encode(text).0.into_owned(),
        [b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat(),
    ] {
        let out = process(&cfg, &bytes, Format::Csv, &Options::default()).unwrap();
        assert_eq!(out.summary.exact, 2);
        assert_eq!(out.summary.empty, 1);
        let s = if std::str::from_utf8(&bytes).is_ok() {
            String::from_utf8(out.bytes).unwrap()
        } else {
            encoding_rs::GB18030.decode(&out.bytes).0.into_owned()
        };
        let s = s.trim_start_matches('\u{feff}');
        let mut rdr = csv::Reader::from_reader(s.as_bytes());
        let h = rdr.headers().unwrap().clone();
        assert_eq!(&h[0], "序号");
        assert_eq!(&h[3], "不含增值税收入");
        let recs: Vec<_> = rdr.records().map(|r| r.unwrap()).collect();
        assert_eq!(&recs[0][3], "10000.00");
        let tc = h.iter().position(|x| x == "企业总支出").unwrap();
        assert_eq!(&recs[0][tc], "10103.00");
        let ci = h.iter().position(|x| x == "预扣率").unwrap();
        assert_eq!(&recs[0][ci], "20%");
        let net = Decimal::from_str(&recs[1][3]).unwrap();
        assert_eq!(forward(&cfg, net).unwrap().take_home, dec!(1000));
    }
}

#[test]
fn custom_column_and_missing_column() {
    let cfg = Config::default();
    let text = "实发,x\n100,1\n";
    assert!(matches!(
        process(&cfg, text.as_bytes(), Format::Csv, &Options::default()),
        Err(BatchError::ColumnNotFound(_))
    ));
    let opts = Options {
        column: Some("实发".into()),
        ..Options::default()
    };
    let out = process(&cfg, text.as_bytes(), Format::Csv, &opts).unwrap();
    assert_eq!(out.summary.exact, 1);
    assert!(matches!(
        process(&cfg, b"", Format::Csv, &opts),
        Err(BatchError::Empty)
    ));
}

#[test]
fn total_cost_gap_notes() {
    // 按企业总成本批量正算：门槛空白区间与逐分舍入空位都在备注列写明原因和两侧最近的有效总成本
    let cfg = Config::default();
    let text = "企业总成本\n999.99\n1000.00\n1010.29\n1010.30\n10103.00\n1061.32\n";
    let opts = Options {
        kind: InputKind::TotalCost,
        column: None,
        preview_rows: 3,
        columns: None,
        ..Options::default()
    };
    let out = process(&cfg, text.as_bytes(), Format::Csv, &opts).unwrap();
    assert_eq!(out.summary.rows, 6);
    assert_eq!(out.summary.unreachable_cost, 3);
    assert_eq!(out.summary.exact, 3);
    // 预览：标题 + 前 3 行，文本与 CSV 输出一致
    assert_eq!(out.preview.len(), 4);
    assert_eq!(out.preview[0][0], "企业总成本");
    assert_eq!(out.preview[0].last().unwrap(), "备注");
    assert!(out.preview[2].last().unwrap().contains("不可达区间"));
    let s = String::from_utf8(out.bytes).unwrap();
    let s = s.trim_start_matches('\u{feff}');
    let mut rdr = csv::Reader::from_reader(s.as_bytes());
    let h = rdr.headers().unwrap().clone();
    let col = |n: &str| h.iter().position(|x| x == n).unwrap();
    let recs: Vec<_> = rdr.records().map(|r| r.unwrap()).collect();
    assert_eq!(&recs[0][col("不含增值税收入")], "999.99");
    assert!(recs[0][col("备注")].contains("未达到增值税起征点"));
    for r in [1, 2] {
        assert_eq!(&recs[r][col("测算状态")], "总支出不可达");
        assert_eq!(&recs[r][col("不含增值税收入")], "");
        let note = &recs[r][col("备注")];
        assert!(
            note.contains("增值税起征点（1000 元）形成的不可达区间"),
            "{note}"
        );
        assert!(
            note.contains("不含增值税收入 999.99 对应企业总支出 999.99，1000.00 对应 1010.30"),
            "{note}"
        );
        assert!(note.contains("两侧最近的有效企业总支出：999.99"), "{note}");
    }
    assert_eq!(&recs[3][col("不含增值税收入")], "1000.00");
    assert_eq!(&recs[4][col("不含增值税收入")], "10000.00");
    // 1050.49 → 1061.31，1050.50 → 1061.33（增值税 10.50 → 10.51，三项减半后附加税费均为 0.32），中间 1061.32 不可达
    assert_eq!(&recs[5][col("测算状态")], "总支出不可达");
    assert!(
        recs[5][col("备注")].contains("按分舍入"),
        "{}",
        &recs[5][col("备注")]
    );
}

#[test]
fn net_income_kind() {
    let cfg = Config::default();
    let opts = Options {
        kind: InputKind::NetIncome,
        column: None,
        ..Options::default()
    };
    let out = process(&cfg, "不含税收入\n10000\n".as_bytes(), Format::Csv, &opts).unwrap();
    let s = String::from_utf8(out.bytes).unwrap();
    assert!(s.contains("10103.00") && s.contains("8400.00"));
}

#[test]
fn new_and_legacy_column_names() {
    // 新正式列名与旧列名均可识别；新列名优先
    let cfg = Config::default();
    for (text, kind) in [
        ("税后实发金额\n8400\n", InputKind::TakeHome),
        ("实发金额\n8400\n", InputKind::TakeHome),
        ("到手金额\n8400\n", InputKind::TakeHome),
        ("不含增值税收入\n10000\n", InputKind::NetIncome),
        ("不含税收入\n10000\n", InputKind::NetIncome),
        ("企业总支出\n10103.00\n", InputKind::TotalCost),
        ("企业总成本\n10103.00\n", InputKind::TotalCost),
    ] {
        let opts = Options {
            kind,
            ..Default::default()
        };
        let out = process(&cfg, text.as_bytes(), Format::Csv, &opts).unwrap();
        let s = String::from_utf8(out.bytes).unwrap();
        assert!(s.contains("10000.00"), "{text}: {s}");
    }
    let err = process(
        &cfg,
        "金额\n1\n".as_bytes(),
        Format::Csv,
        &Options::default(),
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "首行未找到标题为“税后实发金额”或“实发金额”或“到手金额”的列"
    );
}

#[test]
fn three_items_columns() {
    // 三项附加税费逐项列示（减半征收后的实际金额）；城市维护建设税改为市区 7%
    let mut cfg = Config::default();
    cfg.surcharge.items[0].rate = dec!(0.07);
    let opts = Options {
        kind: InputKind::NetIncome,
        ..Default::default()
    };
    let out = process(
        &cfg,
        "不含增值税收入\n10000\n".as_bytes(),
        Format::Csv,
        &opts,
    )
    .unwrap();
    let s = String::from_utf8(out.bytes).unwrap();
    let mut lines = s.lines();
    let h = lines.next().unwrap().trim_start_matches('\u{feff}');
    assert!(
        h.contains(
            "增值税税额,城市维护建设税,教育费附加,地方教育附加,附加税费减征额,增值税附加税费合计,企业总支出"
        ),
        "{h}"
    );
    assert!(lines
        .next()
        .unwrap()
        .contains("100.00,3.50,1.50,1.00,6.00,6.00,10106.00"));
    // 默认（城市维护建设税 1%）：0.50 + 1.50 + 1.00 = 3.00
    let out = process(
        &Config::default(),
        "不含增值税收入\n10000\n".as_bytes(),
        Format::Csv,
        &opts,
    )
    .unwrap();
    let s = String::from_utf8(out.bytes).unwrap();
    assert!(
        s.contains("100.00,0.50,1.50,1.00,3.00,3.00,10103.00"),
        "{s}"
    );
}

#[test]
fn formula_injection_prefixes_text_but_not_amounts() {
    let cfg = Config::default();
    let mut csv_buf = Vec::new();
    {
        let mut w = csv::Writer::from_writer(&mut csv_buf);
        w.write_record(["姓名", "不含增值税收入"]).unwrap();
        w.write_record(["=1+1", "10000"]).unwrap();
        w.write_record(["+cmd", "100"]).unwrap();
        w.write_record(["-2+3", "50"]).unwrap();
        w.write_record(["@sum", "10"]).unwrap();
        w.write_record(["\tcmd", "1"]).unwrap();
        w.write_record(["\rcmd", "2"]).unwrap();
        w.write_record(["张三", "1000"]).unwrap();
        w.flush().unwrap();
    }
    let opts = Options {
        kind: InputKind::NetIncome,
        ..Default::default()
    };
    let out = process(&cfg, &csv_buf, Format::Csv, &opts).unwrap();
    let s = csv_text(out.bytes);
    let mut rdr = csv::Reader::from_reader(s.as_bytes());
    let h = rdr.headers().unwrap().clone();
    let col = |n: &str| h.iter().position(|x| x == n).unwrap();
    let recs: Vec<_> = rdr.records().map(|r| r.unwrap()).collect();
    assert_eq!(&recs[0][0], "'=1+1");
    assert_eq!(&recs[1][0], "'+cmd");
    assert_eq!(&recs[2][0], "'-2+3");
    assert_eq!(&recs[3][0], "'@sum");
    assert_eq!(&recs[4][0], "'\tcmd");
    assert_eq!(&recs[5][0], "'\rcmd");
    assert_eq!(&recs[6][0], "张三");
    assert_eq!(&recs[0][col("企业总支出")], "10103.00");
    let net = h
        .iter()
        .enumerate()
        .filter(|(_, x)| *x == "不含增值税收入")
        .map(|(i, _)| i)
        .last()
        .unwrap();
    assert_eq!(&recs[0][net], "10000.00");
    assert!(!recs[0][net].starts_with('\''));
    assert_eq!(&recs[6][col("增值税税额")], "10.00");
    assert!(!recs[0][col("企业总支出")].starts_with('\''));

    let mut wb = rust_xlsxwriter::Workbook::new();
    let ws = wb.add_worksheet();
    ws.write_string(0, 0, "=姓名").unwrap();
    ws.write_string(0, 1, "不含增值税收入").unwrap();
    ws.write_string(1, 0, "=1+1").unwrap();
    ws.write_number(1, 1, 10000.0).unwrap();
    ws.write_number(2, 0, -12.5).unwrap();
    ws.write_number(2, 1, 100.0).unwrap();
    let bytes = wb.save_to_buffer().unwrap();
    let out = process(&cfg, &bytes, Format::Spreadsheet, &opts).unwrap();
    let rows = read_back(&out.bytes);
    // Original XLSX strings are already typed text cells; preserve them exactly.
    assert_eq!(rows[0][0], Data::String("=姓名".into()));
    assert_eq!(rows[1][0], Data::String("=1+1".into()));
    assert_eq!(money(&rows[1][col("不含增值税收入")]), dec!(10000));
    match &rows[2][0] {
        Data::Float(n) => assert!((n - -12.5).abs() < 1e-9, "{n}"),
        other => panic!("负数原表数值不应被转义成文本：{other:?}"),
    }
    // 100 元未达起征点，企业总支出等于不含增值税收入，且仍是数值单元格
    assert_eq!(money(&rows[2][col("企业总支出")]), dec!(100.00));
}

#[test]
fn input_limits_reject_oversized_batches() {
    let cfg = Config::default();
    let csv = "不含增值税收入\n1\n2\n";
    let base = Limits {
        max_file_bytes: 1024,
        max_rows: 10,
        max_cols: 8,
        max_cell_chars: 100,
    };
    let run = |limits: Limits, text: &str| {
        process(
            &cfg,
            text.as_bytes(),
            Format::Csv,
            &Options {
                kind: InputKind::NetIncome,
                limits,
                ..Options::default()
            },
        )
    };
    assert!(run(base.clone(), csv).is_ok());
    assert!(matches!(
        run(
            Limits {
                max_rows: 1,
                ..base.clone()
            },
            csv
        ),
        Err(BatchError::TooManyRows { max: 1 })
    ));
    assert!(matches!(
        run(
            Limits {
                max_cols: 1,
                ..base.clone()
            },
            "甲,不含增值税收入\n张三,1\n"
        ),
        Err(BatchError::TooManyColumns { cols: 2, max: 1 })
    ));
    // “不含税收入”是 5 个字符，数据单元格 8 个字符，应报单元格而不是列名
    let long = format!("不含税收入\n{}\n", "甲".repeat(8));
    assert!(matches!(
        run(
            Limits {
                max_cell_chars: 6,
                ..base.clone()
            },
            &long
        ),
        Err(BatchError::CellTooLong { len: 8, max: 6 })
    ));
    assert!(matches!(
        run(
            Limits {
                max_file_bytes: 4,
                ..base
            },
            csv
        ),
        Err(BatchError::FileTooLarge { .. })
    ));
}

fn csv_text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap()
        .trim_start_matches('\u{feff}')
        .to_string()
}

#[test]
fn export_columns_selection_and_order() {
    // 导出列可选择、可排序；CSV 与 xlsx 均按所选列与顺序输出
    let cfg = Config::default();
    let text = "姓名,税后实发金额\n张三,8400\n李四,\n王五,abc\n";
    let cols = parse_columns(
        "税后实发金额,source,net_income,应预扣预缴税额,status,notes",
        &cfg,
        InputKind::TakeHome,
    )
    .unwrap();
    let opts = Options {
        columns: Some(cols.clone()),
        preview_rows: 5,
        ..Options::default()
    };
    let out = process(&cfg, text.as_bytes(), Format::Csv, &opts).unwrap();
    let s = csv_text(out.bytes);
    let mut lines = s.lines();
    assert_eq!(
        lines.next().unwrap(),
        "姓名,税后实发金额,税后实发金额,不含增值税收入,应预扣预缴税额,测算状态,备注"
    );
    assert_eq!(
        lines.next().unwrap(),
        "张三,8400,8400.00,10000.00,1600.00,精确匹配,"
    );
    assert_eq!(lines.next().unwrap(), "李四,,,,,,");
    let err = lines.next().unwrap();
    assert!(err.starts_with("王五,abc,,,,数据错误,"), "{err}");
    // 预览与导出一致
    assert_eq!(out.preview[0][0], "姓名");
    assert_eq!(out.preview[0].len(), 7);

    // 同一计算结果可直接按不同导出列生成 xlsx，无需重新计算
    let computed = compute(&cfg, text.as_bytes(), Format::Csv, &opts).unwrap();
    let x = computed
        .render(
            &[ExportColumn::Status, ExportColumn::NetIncome],
            Format::Spreadsheet,
        )
        .unwrap();
    let rows = read_back(&x);
    assert_eq!(rows[0][2].to_string(), "测算状态");
    assert_eq!(rows[0][3].to_string(), "不含增值税收入");
    assert_eq!(rows[0].len(), 4);
    assert_eq!(money(&rows[1][3]), dec!(10000));
    assert!(matches!(
        computed.render(&[], Format::Csv),
        Err(BatchError::NoColumns)
    ));
    // 不适用于本方式的列被忽略
    let only = computed
        .render(&[ExportColumn::Diff, ExportColumn::Vat], Format::Csv)
        .unwrap();
    assert!(csv_text(only).starts_with("姓名,税后实发金额,差额,增值税税额\n"));
}

#[test]
fn export_columns_per_kind_and_defaults() {
    let cfg = Config::default();
    // 默认导出全部列：与可导出列一致；仅按税后实发金额测算有“差额”
    for (kind, text) in [
        (InputKind::TakeHome, "税后实发金额\n8400\n"),
        (InputKind::NetIncome, "不含增值税收入\n10000\n"),
        (InputKind::TotalCost, "企业总支出\n10103.00\n"),
    ] {
        let opts = Options {
            kind,
            ..Options::default()
        };
        let c = compute(&cfg, text.as_bytes(), Format::Csv, &opts).unwrap();
        let avail = available_columns(&cfg, kind);
        assert_eq!(c.available_columns(), avail.as_slice());
        let h = c.headers(&ColumnLayout::default().selected(&avail));
        assert_eq!(h.len(), avail.len(), "{kind:?}");
        assert_eq!(h.contains(&"差额".to_string()), kind == InputKind::TakeHome);
        assert_eq!(h.last().unwrap(), "备注");
    }
}

#[test]
fn csv_preserves_encoding_and_always_appends_to_source() {
    let cfg = Config::default();
    for (input, encoding) in [
        ("姓名,到手金额\n张三,8400\n".as_bytes().to_vec(), 0),
        ("\u{feff}姓名,到手金额\n张三,8400\n".as_bytes().to_vec(), 1),
        (
            encoding_rs::GB18030
                .encode("姓名,到手金额\n张三,8400\n")
                .0
                .into_owned(),
            2,
        ),
    ] {
        let computed = compute(&cfg, &input, Format::Csv, &Options::default()).unwrap();
        let output = computed
            .render(
                &[ExportColumn::NetIncome, ExportColumn::Source],
                Format::Csv,
            )
            .unwrap();
        let decoded = if encoding == 2 {
            encoding_rs::GB18030.decode(&output).0.into_owned()
        } else {
            assert_eq!(output.starts_with(b"\xef\xbb\xbf"), encoding == 1);
            String::from_utf8(output).unwrap()
        };
        let mut reader =
            csv::Reader::from_reader(decoded.trim_start_matches('\u{feff}').as_bytes());
        assert_eq!(
            reader.headers().unwrap().iter().collect::<Vec<_>>(),
            ["姓名", "到手金额", "不含增值税收入"]
        );
    }
}

#[test]
fn xlsx_keeps_formula_merge_width_and_other_sheet() {
    let mut wb = rust_xlsxwriter::Workbook::new();
    let format = rust_xlsxwriter::Format::new()
        .set_font_name("Arial")
        .set_num_format("0.0000");
    let ws = wb.add_worksheet();
    ws.write_string(0, 0, "到手金额").unwrap();
    ws.write_number_with_format(1, 0, 8400, &format).unwrap();
    ws.write_formula(1, 1, "=A2*2").unwrap();
    ws.merge_range(0, 2, 0, 3, "合并", &format).unwrap();
    ws.set_column_width(0, 23).unwrap();
    wb.add_worksheet().write_string(0, 0, "保留").unwrap();
    let input = wb.save_to_buffer().unwrap();
    let computed = compute(
        &Config::default(),
        &input,
        Format::Spreadsheet,
        &Options::default(),
    )
    .unwrap();
    let output = computed
        .render(&[ExportColumn::NetIncome], Format::Spreadsheet)
        .unwrap();
    let entry = |bytes: &[u8], name: &str| {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut text = String::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    };
    let before = entry(&input, "xl/worksheets/sheet1.xml");
    let after = entry(&output, "xl/worksheets/sheet1.xml");
    let cols = |s: &str| s[s.find("<cols>").unwrap()..s.find("</cols>").unwrap() + 7].to_owned();
    assert_eq!(
        cols(&before),
        cols(&after),
        "original width XML must be preserved exactly"
    );
    assert!(after.contains("<f>A2*2</f>"));
    assert!(after.contains("<mergeCell ref=\"C1:D1\"/>"));
    assert_eq!(
        entry(&input, "xl/worksheets/sheet2.xml"),
        entry(&output, "xl/worksheets/sheet2.xml")
    );
    assert!(entry(&output, "xl/styles.xml").contains("Arial"));
    let rows = read_back(&output);
    assert_eq!(rows[0][4].to_string(), "不含增值税收入");
    assert_eq!(money(&rows[1][4]), dec!(10000));
}

#[test]
fn grouped_vat_uses_all_rows_but_individual_iit() {
    use labor_fee_batch::VatGrouping;
    for kind in [InputKind::NetIncome, InputKind::TakeHome] {
        let input = format!("人员,日期,{}\n甲,2026-01-01,600\n乙,2026-01-01,500\n甲,2026-01-01,600\n甲,2026-01-02,600\n",kind.default_column());
        let options = Options {
            kind,
            vat_grouping: Some(VatGrouping {
                person_column: "人员".into(),
                date_column: "日期".into(),
            }),
            ..Options::default()
        };
        let computed =
            compute(&Config::default(), input.as_bytes(), Format::Csv, &options).unwrap();
        let preview = computed.preview(&[ExportColumn::Vat, ExportColumn::IitTax], 10);
        assert_eq!(&preview[1][3..], &["6.00", "0.00"]);
        assert_eq!(&preview[2][3..], &["0.00", "0.00"]);
        assert_eq!(&preview[3][3..], &["6.00", "0.00"]);
        assert_eq!(&preview[4][3..], &["0.00", "0.00"]);
    }
}

#[test]
fn grouping_normalizes_daily_dates_and_rejects_invalid_keys_or_amounts() {
    use labor_fee_batch::VatGrouping;
    let mut options = Options {
        kind: InputKind::NetIncome,
        vat_grouping: Some(VatGrouping {
            person_column: "人员".into(),
            date_column: "日期".into(),
        }),
        ..Options::default()
    };
    let input = "人员,日期,不含增值税收入\n甲,2026-01-02,600\n甲,2026/1/2,600\n";
    let computed = compute(&Config::default(), input.as_bytes(), Format::Csv, &options).unwrap();
    assert_eq!(computed.preview(&[ExportColumn::Vat], 10)[1][3], "6.00");
    for date in ["2026-02-30", "2026-01-02T12:00:00", "45293", "随便"] {
        let input = format!("人员,日期,不含增值税收入\n甲,{date},600\n");
        assert!(matches!(
            compute(&Config::default(), input.as_bytes(), Format::Csv, &options),
            Err(BatchError::Grouping(_))
        ));
    }
    options.kind = InputKind::TotalCost;
    let input = "人员,日期,企业总支出\n甲,2026-01-02,600\n甲,2026-01-02,600\n甲,2026-01-02,-300\n";
    assert!(matches!(
        compute(&Config::default(), input.as_bytes(), Format::Csv, &options),
        Err(BatchError::Grouping(_))
    ));
    let input = "人员,日期,企业总支出\n甲,2026-01-02,500\n甲,2026-01-02,500\n";
    assert!(matches!(
        compute(&Config::default(), input.as_bytes(), Format::Csv, &options),
        Err(BatchError::Grouping(_))
    ));
}

fn zip_entry(bytes: &[u8], name: &str) -> String {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}
fn edit_zip(bytes: &[u8], edit: impl Fn(&str, String) -> (String, String)) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let mut text = String::new();
        entry.read_to_string(&mut text).unwrap();
        let (name, text) = edit(entry.name(), text);
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn xlsx_appends_after_styled_empty_formula_and_merged_extent() {
    let base = make_xlsx();
    for (extra, right) in [
        ("<c r=\"J2\" s=\"0\"/>", "K1"),
        ("<c r=\"L2\"><f>A2*2</f></c>", "M1"),
        ("", "P1"),
    ] {
        let input = edit_zip(&base, |name, xml| {
            let xml = if name == "xl/worksheets/sheet1.xml" {
                let at = xml.find("<row r=\"2\"").unwrap();
                let end = at + xml[at..].find("</row>").unwrap();
                let mut xml = xml;
                xml.insert_str(end, extra);
                if extra.is_empty() {
                    xml.replace("</sheetData>","</sheetData><mergeCells count=\"1\"><mergeCell ref=\"H2:O2\"/></mergeCells>")
                } else {
                    xml
                }
            } else {
                xml
            };
            (name.into(), xml)
        });
        let computed = compute(
            &Config::default(),
            &input,
            Format::Spreadsheet,
            &Options::default(),
        )
        .unwrap();
        let out = computed
            .render(&[ExportColumn::NetIncome], Format::Spreadsheet)
            .unwrap();
        let xml = zip_entry(&out, "xl/worksheets/sheet1.xml");
        assert!(
            xml.contains(&format!("<c r=\"{right}\" t=\"inlineStr\">")),
            "{xml}"
        );
        if !extra.is_empty() {
            assert!(xml.contains(extra));
        }
    }
}

#[test]
fn xlsx_empty_number_formats_custom_style_path_and_prefixed_elements() {
    let base = make_xlsx();
    let input = edit_zip(&base, |name, xml| {
        if name == "xl/styles.xml" {
            (
                "xl/custom-styles.xml".into(),
                xml.replace("<fonts", "<numFmts count=\"0\"/><fonts"),
            )
        } else if name == "xl/_rels/workbook.xml.rels" {
            (
                name.into(),
                xml.replace("Target=\"styles.xml\"", "Target=\"custom-styles.xml\""),
            )
        } else {
            (name.into(), xml)
        }
    });
    let computed = compute(
        &Config::default(),
        &input,
        Format::Spreadsheet,
        &Options::default(),
    )
    .unwrap();
    let out = computed
        .render(&[ExportColumn::NetIncome], Format::Spreadsheet)
        .unwrap();
    let styles = zip_entry(&out, "xl/custom-styles.xml");
    assert_eq!(styles.matches("<numFmts ").count(), 1);
    assert!(styles.contains("formatCode=\"0.00\""));
    let prefixed = edit_zip(&base, |name, xml| {
        let xml = if name == "xl/styles.xml" {
            xml.replace("<styleSheet ","<s:styleSheet xmlns:s=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ").replace("</styleSheet>","</s:styleSheet>")
        } else {
            xml
        };
        (name.into(), xml)
    });
    let computed = compute(
        &Config::default(),
        &prefixed,
        Format::Spreadsheet,
        &Options::default(),
    )
    .unwrap();
    assert!(matches!(
        computed.render(&[ExportColumn::NetIncome], Format::Spreadsheet),
        Err(BatchError::Write(_))
    ));
}

#[test]
fn xlsx_money_number_format_tracks_scale() {
    for scale in [0, 3] {
        let mut cfg = Config::default();
        cfg.rounding.scale = scale;
        for input_format in [Format::Csv, Format::Spreadsheet] {
            let input = if input_format == Format::Csv {
                b"net\n600\n".to_vec()
            } else {
                make_xlsx()
            };
            let options = Options {
                kind: InputKind::NetIncome,
                column: Some(
                    if input_format == Format::Csv {
                        "net"
                    } else {
                        "到手金额"
                    }
                    .into(),
                ),
                ..Options::default()
            };
            let computed = compute(&cfg, &input, input_format, &options).unwrap();
            let output = computed
                .render(&[ExportColumn::NetIncome], Format::Spreadsheet)
                .unwrap();
            let styles = zip_entry(&output, "xl/styles.xml");
            assert!(
                styles.contains(if scale == 0 {
                    "formatCode=\"0\""
                } else {
                    "formatCode=\"0.000\""
                }),
                "{styles}"
            );
        }
    }
}

#[test]
fn grouped_excel_midnight_dates_join_text_dates_and_reject_time() {
    use labor_fee_batch::VatGrouping;
    for hour in [0, 12] {
        let mut wb = rust_xlsxwriter::Workbook::new();
        let ws = wb.add_worksheet();
        for (col, label) in ["人员", "日期", "不含增值税收入"].iter().enumerate() {
            ws.write_string(0, col as u16, *label).unwrap();
        }
        ws.write_string(1, 0, "甲").unwrap();
        let date = rust_xlsxwriter::ExcelDateTime::from_ymd(2026, 1, 2)
            .unwrap()
            .and_hms(hour, 0, 0)
            .unwrap();
        ws.write_datetime_with_format(
            1,
            1,
            &date,
            &rust_xlsxwriter::Format::new().set_num_format("yyyy-mm-dd hh:mm:ss"),
        )
        .unwrap();
        ws.write_number(1, 2, 600).unwrap();
        ws.write_string(2, 0, "甲").unwrap();
        ws.write_string(2, 1, "2026/1/2").unwrap();
        ws.write_number(2, 2, 600).unwrap();
        let options = Options {
            kind: InputKind::NetIncome,
            vat_grouping: Some(VatGrouping {
                person_column: "人员".into(),
                date_column: "日期".into(),
            }),
            ..Options::default()
        };
        let input = wb.save_to_buffer().unwrap();
        let result = compute(&Config::default(), &input, Format::Spreadsheet, &options);
        if hour == 0 {
            assert_eq!(
                result.unwrap().preview(&[ExportColumn::Vat], 10)[1][3],
                "6.00"
            );
        } else {
            assert!(matches!(result, Err(BatchError::Grouping(_))));
        }
    }
}

#[test]
fn grouped_total_cost_taxable_regime_is_self_consistent() {
    use labor_fee_batch::VatGrouping;
    let options = Options {
        kind: InputKind::TotalCost,
        vat_grouping: Some(VatGrouping {
            person_column: "人员".into(),
            date_column: "日期".into(),
        }),
        ..Options::default()
    };
    let result = compute(
        &Config::default(),
        "人员,日期,企业总支出\n甲,2026-01-02,505.16\n甲,2026/1/2,505.16\n".as_bytes(),
        Format::Csv,
        &options,
    )
    .unwrap();
    let rows = result.preview(
        &[
            ExportColumn::NetIncome,
            ExportColumn::Vat,
            ExportColumn::IitTax,
        ],
        10,
    );
    assert_eq!(&rows[1][3..], &["500.00", "5.00", "0.00"]);
    assert_eq!(&rows[2][3..], &["500.00", "5.00", "0.00"]);
}

#[test]
fn xlsx_inserts_missing_rows_before_self_closed_rows_and_preserves_bottom_extent() {
    let input = edit_zip(&make_xlsx(), |name, xml| {
        let xml = if name == "xl/worksheets/sheet1.xml" {
            let start = xml.find("<sheetData>").unwrap() + "<sheetData>".len();
            let end = xml.find("</sheetData>").unwrap();
            let mut xml = xml;
            xml.replace_range(start..end,"<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>到手金额</t></is></c></row><row r=\"2\"><c r=\"A2\"><v>600</v></c></row><row r=\"4\" s=\"0\" customFormat=\"1\"/><row r=\"5\"><c r=\"A5\"><v>600</v></c></row><row r=\"30\" s=\"0\" customFormat=\"1\"/>");
            xml.replace(
                "</sheetData>",
                "</sheetData><mergeCells count=\"1\"><mergeCell ref=\"C10:E40\"/></mergeCells>",
            )
        } else {
            xml
        };
        (name.into(), xml)
    });
    let computed = compute(
        &Config::default(),
        &input,
        Format::Spreadsheet,
        &Options::default(),
    )
    .unwrap();
    let output = computed
        .render(&[ExportColumn::NetIncome], Format::Spreadsheet)
        .unwrap();
    let xml = zip_entry(&output, "xl/worksheets/sheet1.xml");
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut rows = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e)
                if e.local_name().as_ref() == b"row" =>
            {
                rows.push(
                    e.attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == b"r")
                        .map(|a| {
                            std::str::from_utf8(&a.value)
                                .unwrap()
                                .parse::<u32>()
                                .unwrap()
                        })
                        .unwrap(),
                )
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    assert!(
        rows.windows(2).all(|pair| pair[0] < pair[1]),
        "row order: {rows:?}"
    );
    assert!(xml.contains("<dimension ref=\"A1:F40\"/>"), "{xml}");
    assert!(xml.contains("<mergeCell ref=\"C10:E40\"/>"));
}
