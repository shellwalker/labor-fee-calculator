use calamine::{Data, Reader};
use labor_fee_batch::{
    available_columns, compute, parse_columns, process, BatchError, ColumnLayout, ExportColumn,
    Format, InputKind, Options,
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
        let s = String::from_utf8(out.bytes).unwrap();
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
        "税后实发金额,姓名,税后实发金额,不含增值税收入,应预扣预缴税额,测算状态,备注"
    );
    assert_eq!(
        lines.next().unwrap(),
        "8400.00,张三,8400,10000.00,1600.00,精确匹配,"
    );
    assert_eq!(lines.next().unwrap(), ",李四,,,,,");
    let err = lines.next().unwrap();
    assert!(err.starts_with(",王五,abc,,,数据错误,"), "{err}");
    // 预览与导出一致
    assert_eq!(out.preview[0][0], "税后实发金额");
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
    assert_eq!(rows[0][0].to_string(), "测算状态");
    assert_eq!(rows[0][1].to_string(), "不含增值税收入");
    assert_eq!(rows[0].len(), 2);
    assert_eq!(money(&rows[1][1]), dec!(10000));
    assert!(matches!(
        computed.render(&[], Format::Csv),
        Err(BatchError::NoColumns)
    ));
    // 不适用于本方式的列被忽略
    let only = computed
        .render(&[ExportColumn::Diff, ExportColumn::Vat], Format::Csv)
        .unwrap();
    assert!(csv_text(only).starts_with("差额,增值税税额\n"));
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
