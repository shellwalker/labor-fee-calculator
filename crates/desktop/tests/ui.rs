//! 桌面程序状态逻辑与界面测试（egui_kittest，无需显示器/GPU）。
//!
//! kittest 默认在遇到任何字体都画不出的字符时直接 panic，所以这里的界面测试同时验证内嵌中文字体覆盖了界面上出现的文字。

use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use labor_fee_batch::InputKind;
use labor_fee_core::CostGapReason;
use labor_fee_desktop::state::{AppState, Level, Outcome, Tab};
use labor_fee_desktop::{fonts, ui};
use rust_decimal_macros::dec;

fn value(o: &Outcome, label: &str) -> String {
    match o {
        Outcome::Done { fields, .. } => fields
            .iter()
            .find(|f| f.label == label)
            .unwrap_or_else(|| panic!("缺少字段 {label}"))
            .value
            .clone(),
        other => panic!("期望结果，实际 {other:?}"),
    }
}

fn status(o: &Outcome) -> (Level, String) {
    match o {
        Outcome::Done { status, .. } => (status.level, status.text.clone()),
        other => panic!("期望结果，实际 {other:?}"),
    }
}

const ALL_FIELDS: [&str; 10] = [
    "不含税收入",
    "增值税",
    "附加税",
    "企业总成本",
    "减除费用",
    "应纳税所得额",
    "预扣率",
    "速算扣除数",
    "个税",
    "实际到手",
];

// ---------- 状态逻辑 ----------

#[test]
fn forward_fields() {
    let mut st = AppState::with_default_config();
    st.forward.input = " 10,000 ".into();
    st.calc_forward();
    let o = &st.forward.outcome;
    for f in ALL_FIELDS {
        value(o, f);
    }
    assert_eq!(value(o, "不含税收入"), "10000.00");
    assert_eq!(value(o, "增值税"), "100.00");
    assert_eq!(value(o, "附加税"), "3.00");
    assert_eq!(value(o, "企业总成本"), "10103.00");
    assert_eq!(value(o, "减除费用"), "2000.00");
    assert_eq!(value(o, "应纳税所得额"), "8000.00");
    assert_eq!(value(o, "预扣率"), "20%");
    assert_eq!(value(o, "个税"), "1600.00");
    assert_eq!(value(o, "实际到手"), "8400.00");
    let (lv, text) = status(o);
    assert_eq!(lv, Level::Ok);
    assert!(text.starts_with("校验通过"));

    st.forward.input = "999.99".into();
    st.calc_forward();
    assert_eq!(value(&st.forward.outcome, "增值税"), "0.00");
    assert!(status(&st.forward.outcome).1.contains("不征增值税及附加税"));
}

#[test]
fn input_errors() {
    let mut st = AppState::with_default_config();
    for (input, msg) in [
        ("", "请输入金额"),
        ("abc", "无法解析"),
        ("-1", "负数"),
        ("1.001", "小数位"),
    ] {
        st.forward.input = input.into();
        st.calc_forward();
        match &st.forward.outcome {
            Outcome::Error(e) => assert!(e.contains(msg), "{input}: {e}"),
            o => panic!("{input}: {o:?}"),
        }
    }
}

#[test]
fn reverse_examples() {
    let mut st = AppState::with_default_config();
    for (th, net, cost) in [
        ("959.99", "999.99", "999.99"),
        ("960", "1000.00", "1010.30"),
        ("3360", "4000.00", "4041.20"),
        ("20000", "23809.52", "24054.76"),
        ("50000", "63235.29", "63886.61"),
    ] {
        st.reverse.input = th.into();
        st.calc_reverse();
        let o = &st.reverse.outcome;
        assert_eq!(value(o, "不含税收入"), net, "到手 {th}");
        assert_eq!(value(o, "企业总成本"), cost, "到手 {th}");
        let (lv, text) = status(o);
        assert_eq!(lv, Level::Ok);
        assert!(text.starts_with("精确"), "{text}");
    }
    // 50000 有两个不含税收入，状态里说明已取最小
    assert!(status(&st.reverse.outcome).1.contains("63235.29～63235.30"));
}

#[test]
fn from_cost_gap_and_click() {
    let mut st = AppState::with_default_config();
    st.from_cost.input = "1005".into();
    st.calc_from_cost();
    let g = match &st.from_cost.outcome {
        Outcome::Gap(g) => g.clone(),
        o => panic!("{o:?}"),
    };
    assert_eq!(g.reason, CostGapReason::Threshold);
    assert_eq!(
        (g.lower.total_cost, g.upper.total_cost),
        (dec!(999.99), dec!(1010.30))
    );
    assert!(labor_fee_desktop::state::gap_reason_text(&g).contains("1000 元增值税征收门槛"));
    // 点击上侧有效值：填入并重算
    st.use_cost(g.upper.total_cost);
    assert_eq!(st.from_cost.input, "1010.30");
    assert_eq!(value(&st.from_cost.outcome, "不含税收入"), "1000.00");
    assert!(status(&st.from_cost.outcome).1.starts_with("可达"));

    st.from_cost.input = "1061.32".into();
    st.calc_from_cost();
    match &st.from_cost.outcome {
        Outcome::Gap(g) => {
            assert_eq!(g.reason, CostGapReason::Rounding);
            assert!(labor_fee_desktop::state::gap_reason_text(g).contains("逐分舍入"));
        }
        o => panic!("{o:?}"),
    }
}

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lfd-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn config_load_and_reset() {
    let mut st = AppState::with_default_config();
    st.forward.input = "10000".into();
    st.calc_forward();
    let dir = tmp_dir("cfg");
    let mut cfg = st.config().clone();
    cfg.vat.rate = dec!(0.03);
    let p = dir.join("三个点.toml");
    std::fs::write(&p, cfg.to_toml_string()).unwrap();
    st.load_config_file(&p);
    assert_eq!(st.config.message.as_ref().unwrap().level, Level::Ok);
    assert!(st.config.source.starts_with("文件"));
    assert!(st.config.toml.contains("rate = \"0.03\""));
    // 已有结果按新口径重算
    assert_eq!(value(&st.forward.outcome, "增值税"), "300.00");

    let bad = dir.join("bad.toml");
    std::fs::write(&bad, "[vat]\nrate = \"x\"").unwrap();
    st.load_config_file(&bad);
    assert_eq!(st.config.message.as_ref().unwrap().level, Level::Error);
    assert_eq!(st.config().vat.rate, dec!(0.03), "加载失败时保留原口径");

    st.reset_config();
    assert_eq!(st.config.source, "内置默认口径");
    assert_eq!(value(&st.forward.outcome, "增值税"), "100.00");
}

fn sample_csv(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("样例.csv");
    std::fs::write(&p, "姓名,到手金额\n甲,500\n乙,960\n丙,abc\n丁,8400\n").unwrap();
    p
}

#[test]
fn batch_blocking_preview_and_save() {
    let dir = tmp_dir("batch");
    let mut st = AppState::with_default_config();
    st.run_batch_blocking();
    assert_eq!(st.batch.message.as_ref().unwrap().text, "请先选择文件");

    st.batch.path = sample_csv(&dir).display().to_string();
    st.run_batch_blocking();
    let res = st.batch.result.as_ref().expect("有结果");
    assert_eq!(res.output.summary.rows, 4);
    assert_eq!(res.output.summary.errors, 1);
    assert!(res.summary_text().starts_with("共 4 行：成功 3"));
    let pv = &res.output.preview;
    assert_eq!(pv.len(), 5);
    assert_eq!(&pv[0][..3], &["姓名", "到手金额", "不含税收入"]);
    assert_eq!(pv[4][2], "10000.00");
    assert_eq!(res.default_file_name(), "样例_反算结果.csv");

    let out = res.default_save_path();
    st.save_batch(&out);
    assert_eq!(st.batch.message.as_ref().unwrap().level, Level::Ok);
    let saved = std::fs::read_to_string(&out).unwrap();
    assert!(saved.contains("10103.00"));

    // 按总成本：空位行写原因
    let p = dir.join("cost.csv");
    std::fs::write(&p, "企业总成本\n1005\n10103\n").unwrap();
    st.batch.path = p.display().to_string();
    st.batch.kind = InputKind::TotalCost;
    st.run_batch_blocking();
    let res = st.batch.result.as_ref().unwrap();
    assert_eq!(res.output.summary.unreachable_cost, 1);
    assert!(res.output.preview[1].last().unwrap().contains("空白区间"));
    assert_eq!(res.default_file_name(), "cost_计算结果.csv");

    st.batch.path = dir.join("x.txt").display().to_string();
    st.run_batch_blocking();
    assert_eq!(st.batch.message.as_ref().unwrap().level, Level::Error);
}

#[test]
fn batch_background_thread() {
    let dir = tmp_dir("bg");
    let mut st = AppState::with_default_config();
    st.batch.path = sample_csv(&dir).display().to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    st.start_batch(move || tx.send(()).unwrap());
    assert!(st.batch_running());
    rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
    st.poll_batch();
    assert!(!st.batch_running());
    assert_eq!(st.batch.result.as_ref().unwrap().output.summary.exact, 3);
}

// ---------- 字体 ----------

/// 内嵌字体覆盖界面及核心/批量模块中出现的全部非 ASCII 字符（中文不会显示成方块）。
#[test]
fn embedded_font_covers_ui_text() {
    let face = ttf_parser::Face::parse(fonts::EMBEDDED_FONT, 0).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut missing = std::collections::BTreeSet::new();
    for dir in ["desktop/src", "core/src", "batch/src"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "rs") {
                for ch in std::fs::read_to_string(&p).unwrap().chars() {
                    if !ch.is_ascii() && face.glyph_index(ch).is_none() {
                        missing.insert((ch, p.file_name().unwrap().to_string_lossy().to_string()));
                    }
                }
            }
        }
    }
    assert!(missing.is_empty(), "内嵌字体缺字：{missing:?}");
    // 常见姓名用字、全角符号
    for ch in
        "张王李赵钱孙周吴郑冯陈褚卫蒋沈韩杨朱秦尤许何吕施孔曹严华金魏陶姜（）：；，。、“”～—￥"
            .chars()
    {
        assert!(face.glyph_index(ch).is_some(), "缺字 {ch}");
    }
}

// ---------- 界面（egui_kittest） ----------

fn harness(state: AppState) -> Harness<'static, AppState> {
    Harness::builder()
        .with_size(eframe::egui::vec2(1000.0, 720.0))
        .with_context_setup(fonts::install)
        .build_ui_state(|ui, st: &mut AppState| ui::draw(ui, st), state)
}

#[test]
fn ui_forward_tab() {
    let mut h = harness(AppState::with_default_config());
    for t in ["正算", "反算", "按总成本", "批量", "口径"] {
        h.get_by_label(t);
    }
    h.get_by_label("不含税收入（元）");
    let input = h.get_by_role(Role::TextInput);
    input.click();
    input.type_text("10000");
    h.run();
    h.get_by_label("计算").click();
    h.run();
    assert_eq!(value(&h.state().forward.outcome, "实际到手"), "8400.00");
    for f in ALL_FIELDS {
        assert!(h.query_by_label(f).is_some(), "表格缺少 {f}");
    }
    h.get_by_label("校验状态");
    h.get_by_label("10103.00");
    h.get_by_label_contains("校验通过");
}

#[test]
fn ui_reverse_tab_enter_key() {
    let mut h = harness(AppState::with_default_config());
    h.get_by_label("反算").click();
    h.run();
    assert_eq!(h.state().tab, Tab::Reverse);
    let input = h.get_by_role(Role::TextInput);
    input.click();
    input.type_text("3360");
    h.run();
    h.key_press(eframe::egui::Key::Enter);
    h.run();
    assert_eq!(value(&h.state().reverse.outcome, "不含税收入"), "4000.00");
    h.get_by_label("4041.20");
    h.get_by_label_contains("精确");
}

#[test]
fn ui_from_cost_gap_click() {
    let mut st = AppState::with_default_config();
    st.tab = Tab::FromCost;
    st.from_cost.input = "1005.00".into();
    let mut h = harness(st);
    h.get_by_label("计算").click();
    h.run();
    h.get_by_label_contains("原因：1000 元增值税征收门槛形成的空白区间");
    h.get_by_label_contains("这一段没有对应的收入");
    h.get_by_label("999.99（不含税收入 999.99）");
    h.get_by_label("1010.30（不含税收入 1000.00）").click();
    h.run();
    assert_eq!(h.state().from_cost.input, "1010.30");
    assert_eq!(value(&h.state().from_cost.outcome, "增值税"), "10.00");
    h.get_by_label_contains("可达");
}

#[test]
fn ui_batch_tab_preview() {
    let dir = tmp_dir("ui-batch");
    let mut st = AppState::with_default_config();
    st.tab = Tab::Batch;
    st.batch.path = sample_csv(&dir).display().to_string();
    st.run_batch_blocking();
    let mut h = harness(st);
    h.run();
    h.get_by_label("选择文件…");
    h.get_by_label("执行");
    h.get_by_label("另存为…");
    h.get_by_label("保存到输入文件旁（样例_反算结果.csv）");
    h.get_by_label_contains("共 4 行");
    h.get_by_label("甲");
    h.get_by_label("10103.00");
    h.get_by_label_contains("无法解析金额");
    // 切换输入列类型
    h.get_by_label("输入列").click();
    h.run();
    h.get_by_label("企业总成本（按总成本）").click();
    h.run();
    assert_eq!(h.state().batch.kind, InputKind::TotalCost);
}

#[test]
fn ui_batch_execute_button() {
    let dir = tmp_dir("ui-exec");
    let mut st = AppState::with_default_config();
    st.tab = Tab::Batch;
    st.batch.path = sample_csv(&dir).display().to_string();
    let mut h = harness(st);
    h.get_by_label("执行").click();
    h.step();
    let start = std::time::Instant::now();
    while h.state().batch_running() && start.elapsed().as_secs() < 30 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.step();
    }
    h.run();
    assert!(h.state().batch.result.is_some());
    h.get_by_label_contains("共 4 行");
}

#[test]
fn ui_config_tab() {
    let mut st = AppState::with_default_config();
    st.tab = Tab::Config;
    let mut h = harness(st);
    h.run();
    h.get_by_label("加载 TOML 文件…");
    h.get_by_label("恢复内置默认");
    h.get_by_label("内置默认口径");
}

#[test]
fn sample_xlsx() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/到手样例.xlsx");
    let mut st = AppState::with_default_config();
    st.batch.path = p.display().to_string();
    st.run_batch_blocking();
    let res = st.batch.result.as_ref().expect("有结果");
    let s = &res.output.summary;
    assert_eq!((s.rows, s.exact, s.errors), (7, 6, 1));
    assert_eq!(res.default_file_name(), "到手样例_反算结果.xlsx");
    let pv = &res.output.preview;
    let col = |n: &str| pv[0].iter().position(|x| x == n).unwrap();
    let nets: Vec<&str> = pv[1..]
        .iter()
        .map(|r| r[col("不含税收入")].as_str())
        .collect();
    assert_eq!(
        nets,
        ["500.00", "999.99", "1000.00", "4000.00", "23809.52", "63235.29", ""]
    );
    assert_eq!(pv[7][col("校准状态")], "错误");
}
