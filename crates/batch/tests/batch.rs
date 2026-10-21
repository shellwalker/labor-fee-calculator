use calamine::{Data, Reader};
use labor_fee_batch::{process, BatchError, Format, Options};
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
    assert_eq!(&h[..4], &["姓名", "身份证号", "到手金额", "企业总成本"]);
    let col = |name: &str| h.iter().position(|x| x == name).unwrap();
    // 身份证号保持文本
    assert_eq!(rows[1][1], Data::String("110101199001011234".into()));
    for (r, expect) in [
        (1, dec!(8400)),
        (2, dec!(3360)),
        (3, dec!(12345.67)),
        (6, dec!(0.30)),
    ] {
        let cost = money(&rows[r][col("企业总成本")]);
        assert_eq!(forward(&cfg, cost).unwrap().take_home, expect);
        assert_eq!(money(&rows[r][col("正算到手（校验）")]), expect);
        assert_eq!(rows[r][col("校准状态")], Data::String("精确".into()));
    }
    assert_eq!(money(&rows[1][col("企业总成本")]), dec!(10103));
    assert!(rows[5][col("校准状态")].to_string().starts_with("错误"));
    assert_eq!(rows[4][col("企业总成本")], Data::Empty);
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
        assert_eq!(&h[3], "企业总成本");
        let recs: Vec<_> = rdr.records().map(|r| r.unwrap()).collect();
        assert_eq!(&recs[0][3], "10103.00");
        let ci = h.iter().position(|x| x == "预扣率").unwrap();
        assert_eq!(&recs[0][ci], "20%");
        let cost = Decimal::from_str(&recs[1][3]).unwrap();
        assert_eq!(forward(&cfg, cost).unwrap().take_home, dec!(1000));
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
        column: "实发".into(),
    };
    let out = process(&cfg, text.as_bytes(), Format::Csv, &opts).unwrap();
    assert_eq!(out.summary.exact, 1);
    assert!(matches!(
        process(&cfg, b"", Format::Csv, &opts),
        Err(BatchError::Empty)
    ));
}
