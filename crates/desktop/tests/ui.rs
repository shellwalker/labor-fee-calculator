//! 桌面程序状态逻辑与界面测试（egui_kittest，无需显示器/GPU）。
//!
//! kittest 默认在遇到任何字体都画不出的字符时直接 panic，所以这里的界面测试同时验证内嵌中文字体覆盖了界面上出现的文字。

use eframe::egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use labor_fee_batch::InputKind;
use labor_fee_core::CostGapReason;
use labor_fee_desktop::settings::{self, SettingsDraft, SettingsStore, ThemeChoice};
use labor_fee_desktop::state::{AppState, Level, Outcome, Tab};
use labor_fee_desktop::{fonts, rules, ui};
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

const ALL_FIELDS: [&str; 11] = [
    "不含增值税收入",
    "增值税税额",
    "  教育费附加（3%）",
    "增值税附加税费合计",
    "企业总支出",
    "减除费用",
    "预扣预缴应纳税所得额",
    "预扣率",
    "速算扣除数",
    "应预扣预缴税额",
    "税后实发金额",
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
    assert_eq!(value(o, "不含增值税收入"), "10000.00");
    assert_eq!(value(o, "增值税税额"), "100.00");
    assert_eq!(value(o, "增值税附加税费合计"), "3.00");
    assert_eq!(value(o, "企业总支出"), "10103.00");
    assert_eq!(value(o, "减除费用"), "2000.00");
    assert_eq!(value(o, "预扣预缴应纳税所得额"), "8000.00");
    assert_eq!(value(o, "预扣率"), "20%");
    assert_eq!(value(o, "应预扣预缴税额"), "1600.00");
    assert_eq!(value(o, "税后实发金额"), "8400.00");
    let (lv, text) = status(o);
    assert_eq!(lv, Level::Ok);
    assert!(text.starts_with("校验通过"));

    st.forward.input = "999.99".into();
    st.calc_forward();
    assert_eq!(value(&st.forward.outcome, "增值税税额"), "0.00");
    assert!(status(&st.forward.outcome)
        .1
        .contains("不征收增值税及附加税费"));
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
        assert_eq!(value(o, "不含增值税收入"), net, "到手 {th}");
        assert_eq!(value(o, "企业总支出"), cost, "到手 {th}");
        let (lv, text) = status(o);
        assert_eq!(lv, Level::Ok);
        assert!(text.starts_with("精确匹配"), "{text}");
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
    assert!(labor_fee_desktop::state::gap_reason_text(&g).contains("增值税起征点（1000 元）"));
    // 点击上侧有效值：填入并重算
    st.use_cost(g.upper.total_cost);
    assert_eq!(st.from_cost.input, "1010.30");
    assert_eq!(value(&st.from_cost.outcome, "不含增值税收入"), "1000.00");
    assert!(status(&st.from_cost.outcome).1.starts_with("校验通过"));

    st.from_cost.input = "1061.32".into();
    st.calc_from_cost();
    match &st.from_cost.outcome {
        Outcome::Gap(g) => {
            assert_eq!(g.reason, CostGapReason::Rounding);
            assert!(labor_fee_desktop::state::gap_reason_text(g).contains("按分舍入"));
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
    assert_eq!(st.notice.as_ref().unwrap().level, Level::Ok);
    assert!(st.source.starts_with("文件"));
    assert!(!st.is_default_config());
    assert_eq!(settings::modified_items(st.config()), 1);
    // 已有结果按新口径重算
    assert_eq!(value(&st.forward.outcome, "增值税税额"), "300.00");

    let bad = dir.join("bad.toml");
    std::fs::write(&bad, "[vat]\nrate = \"x\"").unwrap();
    st.load_config_file(&bad);
    assert_eq!(st.notice.as_ref().unwrap().level, Level::Error);
    assert_eq!(st.config().vat.rate, dec!(0.03), "加载失败时保留原参数");

    st.reset_config();
    assert_eq!(st.source, "内置默认参数");
    assert_eq!(value(&st.forward.outcome, "增值税税额"), "100.00");
}

// ---------- 设置：编辑、校验、持久化 ----------

#[test]
fn settings_surcharge_items_prefilled() {
    let cfg = labor_fee_core::Config::default();
    let mut d = SettingsDraft::from_config(&cfg, ThemeChoice::System);
    // 默认：城市维护建设税、地方教育附加不计征，但费率已预填 7%、2%
    assert_eq!((d.items[0].enabled, d.items[0].rate.as_str()), (false, "7"));
    assert_eq!((d.items[2].enabled, d.items[2].rate.as_str()), (false, "2"));
    assert_eq!(d.modified_count(), 0);
    // 只需勾选即可生效
    d.items[0].enabled = true;
    d.items[2].enabled = true;
    let c = d.build().unwrap();
    let b = labor_fee_core::forward(&c, dec!(10000)).unwrap();
    let amts: Vec<_> = b.surcharges.iter().map(|l| l.amount).collect();
    assert_eq!(amts, vec![dec!(7.00), dec!(3.00), dec!(2.00)]);
    assert_eq!(b.surcharge_total, dec!(12.00));
    assert_eq!(b.total_cost, dec!(10112.00));
    assert_eq!(d.modified_count(), 2);
    assert!(!settings::item_same(&d.items[0], &d.defaults().items[0]));
    // 规则说明动态列出计征的项目
    let text: String = rules::rule_groups(&c)
        .into_iter()
        .flat_map(|g| g.lines)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("城市维护建设税 = 实际缴纳的增值税税额 × 7%"),
        "{text}"
    );
    assert!(text.contains("增值税附加税费 = 城市维护建设税 + 教育费附加 + 地方教育附加"));
    assert!(!text.contains("减征"), "{text}");
    // 取消勾选、费率写法不同（7.0）均视为恢复默认
    d.items[0].enabled = false;
    d.items[2].enabled = false;
    d.items[0].rate = "7.0".into();
    assert_eq!(d.modified_count(), 0);
    // 改为 5%（县城、镇）但不勾选：记为已修改，但不参与计算
    d.items[0].rate = "5".into();
    assert_eq!(d.modified_count(), 1);
    let c = d.build().unwrap();
    assert_eq!(
        labor_fee_core::forward(&c, dec!(10000)).unwrap().total_cost,
        dec!(10103.00)
    );
    // 默认规则说明列出未计征项目及预填费率
    let text: String = rules::rule_groups(&labor_fee_core::Config::default())
        .into_iter()
        .flat_map(|g| g.lines)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("未计征：城市维护建设税（7%）、地方教育附加（2%）"),
        "{text}"
    );
}

#[test]
fn legacy_rules_file_loads_as_default() {
    // 旧版设置文件（单一“附加税”项目、已不再支持的 halve 减征字段）读入后与内置默认一致
    let dir = tmp_dir("legacy");
    let store = SettingsStore::at(dir.clone());
    let legacy = labor_fee_core::DEFAULT_CONFIG_TOML.replace(
        &labor_fee_core::DEFAULT_CONFIG_TOML[labor_fee_core::DEFAULT_CONFIG_TOML.find("[surcharge]").unwrap()
            ..labor_fee_core::DEFAULT_CONFIG_TOML.find("[iit]").unwrap()],
        "[surcharge]\nbase = \"vat\"\nhalve = true\n[[surcharge.items]]\nname = \"附加税\"\nrate = \"0.03\"\n\n",
    );
    std::fs::write(dir.join("rules.toml"), legacy).unwrap();
    let st = AppState::with_store(store);
    assert!(st.is_default_config(), "{}", st.source);
}

#[test]
fn settings_draft_roundtrip_and_validation() {
    let cfg = labor_fee_core::Config::default();
    let d = SettingsDraft::from_config(&cfg, ThemeChoice::System);
    assert_eq!(d.vat_rate, "1");
    assert_eq!(d.threshold, "1000");
    assert_eq!(d.items.len(), 3);
    assert_eq!(d.items[1].name, "教育费附加");
    assert_eq!(d.items[1].rate, "3");
    assert!(d.items[1].enabled && !d.items[0].enabled && !d.items[2].enabled);
    assert_eq!(d.pct_deduction, "20");
    assert_eq!(d.brackets.len(), 3);
    assert_eq!(d.build().unwrap(), cfg, "未修改时生成的配置与默认一致");
    assert_eq!(d.modified_count(), 0);

    let mut e = d.clone();
    e.vat_rate = "abc".into();
    e.brackets[1].upper = "100".into(); // 不递增
    e.scale = "9".into();
    let errs = e.build().unwrap_err();
    assert!(errs.contains_key(settings::key::VAT_RATE));
    assert!(errs.contains_key("iit.brackets.1"));
    assert!(errs.contains_key(settings::key::SCALE));

    let mut m = d.clone();
    m.vat_rate = "3".into();
    m.threshold_enabled = false;
    let c = m.build().unwrap();
    assert_eq!(c.vat.rate, dec!(0.03));
    assert!(!c.vat.threshold_enabled);
    assert_eq!(m.modified_count(), 2);
    m.reset_all();
    assert_eq!(m.modified_count(), 0);
    // 数值等价视为未修改
    let mut z = d.clone();
    z.vat_rate = "1.0".into();
    assert_eq!(z.modified_count(), 0);
}

#[test]
fn settings_confirm_persist_and_reload() {
    use settings::key;
    let dir = tmp_dir("store");
    let store = SettingsStore::at(dir.join("labor-fee-desktop"));
    let mut st = AppState::with_store(store.clone());
    assert!(st.is_default_config());
    assert_eq!(st.prefs.theme, ThemeChoice::System, "默认跟随系统");

    st.forward.input = "10000".into();
    st.calc_forward();
    st.open_settings();
    {
        let w = st.settings.as_mut().unwrap();
        w.edit.vat_rate = "3".into();
        w.edit.theme = ThemeChoice::Dark;
        assert_eq!(w.pending_rows(), vec![key::VAT_RATE, key::THEME]);
    }
    assert_eq!(
        st.effective_theme(),
        ThemeChoice::Dark,
        "设置窗口内预览主题"
    );
    // 未确认：不生效、不保存
    assert_eq!(st.config().vat.rate, dec!(0.01));
    assert_eq!(value(&st.forward.outcome, "增值税税额"), "100.00");
    assert!(!store.rules_path().exists());
    // 非法值：不能确认
    st.settings.as_mut().unwrap().edit.threshold = "-1".into();
    assert!(st
        .settings
        .as_ref()
        .unwrap()
        .row_error(key::THRESHOLD)
        .is_some());
    assert!(!st.confirm_setting(key::THRESHOLD));
    st.discard_setting(key::THRESHOLD);
    assert_eq!(st.settings.as_ref().unwrap().edit.threshold, "1000");
    // √：立即生效并保存，其他未确认的行保留
    assert!(st.confirm_setting(key::VAT_RATE));
    assert_eq!(st.source, "已保存的设置");
    assert_eq!(value(&st.forward.outcome, "增值税税额"), "300.00");
    assert!(store.rules_path().exists());
    assert_eq!(
        st.settings.as_ref().unwrap().pending_rows(),
        vec![key::THEME.to_string()]
    );
    assert_eq!(st.prefs.theme, ThemeChoice::System, "主题尚未确认");
    assert!(st.confirm_setting(key::THEME));
    assert!(std::fs::read_to_string(store.prefs_path())
        .unwrap()
        .contains("dark"));
    assert_eq!(st.settings.as_ref().unwrap().pending_count(), 0);
    assert!(st.request_close_settings());

    // 重新启动：读回主题与口径
    let st2 = AppState::with_store(store.clone());
    assert_eq!(st2.prefs.theme, ThemeChoice::Dark);
    assert_eq!(st2.config().vat.rate, dec!(0.03));
    assert_eq!(st2.source, "已保存的设置");

    // 关闭时有未生效的修改：先确认；放弃后不改变已生效的设置
    let mut st3 = AppState::with_store(store.clone());
    st3.open_settings();
    st3.settings.as_mut().unwrap().edit.vat_rate = "5".into();
    assert!(!st3.request_close_settings());
    assert_eq!(
        st3.settings.as_ref().unwrap().dialog,
        Some(settings::Dialog::DiscardOnClose)
    );
    st3.close_settings();
    assert_eq!(st3.config().vat.rate, dec!(0.03));

    // 单项恢复默认：直接生效
    st3.open_settings();
    assert!(st3.reset_setting(key::VAT_RATE));
    assert!(st3.is_default_config());
    assert!(
        !store.rules_path().exists(),
        "与内置默认相同时不保存 rules.toml"
    );

    // 自行添加的项目：新增行须确认，删除已生效的项目立即生效
    {
        let w = st3.settings.as_mut().unwrap();
        w.edit.items.push(settings::ItemDraft {
            name: "其他附加".into(),
            rate: "1".into(),
            enabled: true,
        });
        assert!(w.is_pending("surcharge.items.3"));
    }
    assert!(st3.confirm_setting("surcharge.items.3"));
    assert_eq!(st3.config().surcharge.items.len(), 4);
    assert!(st3.delete_item(3));
    assert_eq!(st3.config().surcharge.items.len(), 3);
    assert_eq!(st3.settings.as_ref().unwrap().pending_count(), 0);

    // 全部恢复默认：rules.toml 删除、preferences.toml 还原
    st3.settings.as_mut().unwrap().edit.scale = "3".into();
    assert!(st3.confirm_setting(settings::key::SCALE));
    assert!(store.rules_path().exists());
    assert!(st3.reset_all_settings());
    assert!(st3.is_default_config());
    assert!(!store.rules_path().exists());
    assert_eq!(st3.prefs.theme, ThemeChoice::System);
    assert_eq!(store.load_prefs().theme, ThemeChoice::System);
    let w = st3.settings.as_ref().unwrap();
    assert_eq!((w.applied.modified_count(), w.pending_count()), (0, 0));

    // 损坏的设置文件：回退默认并提示
    std::fs::write(store.rules_path(), "garbage = [").unwrap();
    let st4 = AppState::with_store(store);
    assert!(st4.is_default_config());
    assert_eq!(st4.notice.as_ref().unwrap().level, Level::Error);
}

#[test]
fn rules_panel_follows_config() {
    let cfg = labor_fee_core::Config::default();
    let text: String = rules::rule_groups(&cfg)
        .iter()
        .flat_map(|g| std::iter::once(g.title.to_string()).chain(g.lines.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    for s in [
        "增值税税额 = 不含增值税收入 × 征收率 1%",
        "不含增值税收入达到 1000 元 的征收",
        "教育费附加 = 实际缴纳的增值税税额 × 3%",
        "增值税附加税费 = 教育费附加（各项分别按分舍入）",
        "未计征：城市维护建设税（7%）、地方教育附加（2%）",
        "企业承担",
        "每次收入不超过 4000 元 的，减除费用 800 元；4000 元 以上的，减除费用按 20% 计算",
        "四舍五入",
        "取最小不含增值税收入",
        "企业总支出 1000.00～1010.29 无对应的不含增值税收入",
    ] {
        assert!(text.contains(s), "规则缺少：{s}\n{text}");
    }
    let rows = rules::bracket_rows(&cfg);
    assert_eq!(rows[0].range, "≤ 20000");
    assert_eq!(rows[1].range, "20000～50000");
    assert_eq!(
        (
            rows[2].range.as_str(),
            rows[2].rate.as_str(),
            rows[2].quick.as_str()
        ),
        ("> 50000", "40%", "7000")
    );

    let mut c2 = cfg.clone();
    c2.vat.threshold_enabled = false;
    assert!(rules::vacuum_gap(&c2).is_none());
    let t2: Vec<String> = rules::rule_groups(&c2)
        .into_iter()
        .flat_map(|g| g.lines)
        .collect();
    assert!(t2.iter().any(|l| l.contains("未启用起征点")));
}

#[test]
fn money_grouping() {
    use labor_fee_desktop::ui::widgets::group_digits;
    assert_eq!(group_digits("10103.00"), "10,103.00");
    assert_eq!(group_digits("999.99"), "999.99");
    assert_eq!(group_digits("1234567"), "1,234,567");
    assert_eq!(group_digits("-1000.5"), "-1,000.5");
    assert_eq!(group_digits("12元5角"), "12元5角");
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
    assert_eq!(&pv[0][..3], &["姓名", "到手金额", "不含增值税收入"]);
    assert_eq!(pv[4][2], "10000.00");
    assert_eq!(res.default_file_name(), "样例_测算结果.csv");

    let out = res.default_save_path();
    st.save_batch(&out);
    assert_eq!(st.batch.message.as_ref().unwrap().level, Level::Ok);
    let saved = std::fs::read_to_string(&out).unwrap();
    assert!(saved.contains("10103.00"));

    // 按总成本：空位行写原因
    let p = dir.join("cost.csv");
    std::fs::write(&p, "企业总支出\n1005\n10103\n").unwrap();
    st.batch.path = p.display().to_string();
    st.batch.kind = InputKind::TotalCost;
    st.run_batch_blocking();
    let res = st.batch.result.as_ref().unwrap();
    assert_eq!(res.output.summary.unreachable_cost, 1);
    assert!(res.output.preview[1].last().unwrap().contains("不可达区间"));
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
    let icons = ttf_parser::Face::parse(fonts::ICON_FONT, 0).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut missing = std::collections::BTreeSet::new();
    for dir in ["desktop/src", "core/src", "batch/src"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "rs") {
                for ch in std::fs::read_to_string(&p).unwrap().chars() {
                    if !ch.is_ascii()
                        && face.glyph_index(ch).is_none()
                        && icons.glyph_index(ch).is_none()
                    {
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
    harness_sized(state, 1280.0, 820.0)
}

fn harness_sized(mut state: AppState, w: f32, h: f32) -> Harness<'static, AppState> {
    state.prefs.theme = ThemeChoice::Light;
    Harness::builder()
        .with_size(eframe::egui::vec2(w, h))
        .with_context_setup(fonts::install)
        .build_ui_state(|ui, st: &mut AppState| ui::draw(ui, st), state)
}

fn has(h: &Harness<'_, AppState>, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

#[test]
fn ui_forward_tab() {
    let mut h = harness(AppState::with_default_config());
    for t in ["收入计算", "实发测算", "支出测算", "批量测算"] {
        h.get_by_role_and_label(Role::Button, t);
    }
    assert!(h.query_by_label("口径").is_none(), "不再有常驻参数页签");
    h.get_by_label("不含增值税收入（元）");
    h.get_by_label("按不含增值税收入计算");
    h.get_by_label_contains("计算规则");
    h.get_by_label_contains("国家税务总局公告2018年第61号");
    h.get_by_label_contains("财政部 税务总局公告2026年第10号");
    let input = h.get_by_role(Role::TextInput);
    input.click();
    input.type_text("10000");
    h.run();
    h.get_by_label("计算").click();
    h.run();
    assert_eq!(value(&h.state().forward.outcome, "税后实发金额"), "8400.00");
    for f in [
        "增值税税额（1%）",
        "　教育费附加（3%）",
        "增值税附加税费合计",
        "减除费用",
        "预扣预缴应纳税所得额",
        "预扣率",
        "速算扣除数",
    ] {
        assert!(has(&h, f), "缺少 {f}");
    }
    for f in ["企业总支出", "应预扣预缴税额", "税后实发金额"] {
        assert!(has(&h, f), "缺少 {f}");
    }
    h.get_by_label("不含增值税收入（输入）");
    h.get_by_label("增值税及附加税费");
    h.get_by_label("个人所得税");
    assert!(has(&h, "10,103.00"), "千分位金额");
    assert!(has(&h, "8,400.00"));
    h.get_by_label("校验状态");
    h.get_by_label_contains("校验通过");
    // 最近计算
    h.get_by_label_contains("10,000  →  实发 8,400.00");
}

#[test]
fn ui_quick_example_chip() {
    let mut h = harness(AppState::with_default_config());
    h.get_by_label("4000").click();
    h.run();
    assert_eq!(h.state().forward.input, "4000");
    assert_eq!(
        value(&h.state().forward.outcome, "应预扣预缴税额"),
        "640.00"
    );
}

#[test]
fn ui_reverse_tab_enter_key() {
    let mut h = harness(AppState::with_default_config());
    h.get_by_label("实发测算").click();
    h.run();
    assert_eq!(h.state().tab, Tab::Reverse);
    let input = h.get_by_role(Role::TextInput);
    input.click();
    input.type_text("3360");
    h.run();
    h.key_press(eframe::egui::Key::Enter);
    h.run();
    assert_eq!(
        value(&h.state().reverse.outcome, "不含增值税收入"),
        "4000.00"
    );
    assert!(has(&h, "4,041.20"));
    h.get_by_label("税后实发金额（输入）");
    h.get_by_label_contains("精确匹配");
    h.get_by_label_contains("根据税后实发金额测算不含增值税收入");
}

#[test]
fn ui_from_cost_gap_click() {
    let mut st = AppState::with_default_config();
    st.tab = Tab::FromCost;
    st.from_cost.input = "1005.00".into();
    let mut h = harness(st);
    h.get_by_label("计算").click();
    h.run();
    h.get_by_label("该企业总支出无对应的不含增值税收入");
    h.get_by_label_contains("原因：增值税起征点（1000 元）形成的不可达区间");
    h.get_by_label_contains("其间的企业总支出均无对应收入");
    h.get_by_label("← 999.99（收入 999.99）");
    h.get_by_label("1010.30（收入 1000.00）→").click();
    h.run();
    assert_eq!(h.state().from_cost.input, "1010.30");
    assert_eq!(value(&h.state().from_cost.outcome, "增值税税额"), "10.00");
    h.get_by_label_contains("对应的企业总支出与输入一致");
}

#[test]
fn ui_rules_panel_collapse() {
    let mut h = harness(AppState::with_default_config());
    h.get_by_label("收起").click();
    h.run();
    assert!(!h.state().show_rules);
    h.get_by_label_contains("显示计算规则").click();
    h.run();
    assert!(h.state().show_rules);
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
    for s in [
        "选择文件",
        "选择输入列",
        "执行测算",
        "导出结果",
        "总行数",
        "成功",
        "数据错误",
        "空白",
    ] {
        assert!(has(&h, s), "缺少 {s}");
    }
    h.get_by_label("选择文件…");
    h.get_by_label("▶ 执行");
    h.get_by_label_contains("另存为…");
    h.get_by_label_contains("保存到输入文件旁");
    h.get_by_label("样例_测算结果.csv");
    h.get_by_label_contains("共 4 行");
    h.get_by_label("甲");
    assert!(has(&h, "10,103.00"));
    h.get_by_label_contains("无法解析金额");
    // 切换输入列类型
    h.get_by_label("输入列").click();
    h.run();
    h.get_by_label("企业总支出（测算收入）").click();
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
    h.get_by_label("▶ 执行").click();
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

/// 打开设置窗口并切到指定页。
fn settings_at(sec: settings::Section) -> AppState {
    let mut st = AppState::with_default_config();
    st.open_settings();
    st.settings.as_mut().unwrap().edit.section = sec;
    st
}

fn edit<'a>(h: &'a mut Harness<'_, AppState>) -> &'a mut SettingsDraft {
    &mut h.state_mut().settings.as_mut().unwrap().edit
}

#[test]
fn ui_settings_dialog() {
    let mut h = harness(AppState::with_default_config());
    h.get_by_label_contains("设置").click();
    h.run();
    assert!(h.state().settings.is_some());
    h.get_by_label_contains("修改后点击 ✔ 生效，点击 ✖ 放弃本项修改");
    // 分组导航：一级分组标题 + 二级页
    for g in ["计税口径", "计算规则", "通用"] {
        assert!(has(&h, g), "缺少分组 {g}");
    }
    for s in [
        "增值税",
        "附加税费",
        "个人所得税",
        "金额舍入",
        "测算求解",
        "外观",
        "设置存储",
    ] {
        h.get_by_role_and_label(Role::Button, s);
    }
    assert!(
        h.query_by_label("状态说明：").is_none(),
        "不再显示状态说明图例"
    );
    assert!(h.query_by_label("保存").is_none() && h.query_by_label("取消").is_none());
    h.get_by_label("增值税征收率");
    assert!(has(&h, "已确认"));
    h.get_by_role_and_label(Role::Button, "个人所得税").click();
    h.run();
    h.get_by_label("定额减除费用");
    h.get_by_label("预扣率表（个人所得税预扣率表二，居民个人劳务报酬所得适用）");
    assert!(has(&h, "已核实"));
    h.get_by_role_and_label(Role::Button, "金额舍入").click();
    h.run();
    assert!(has(&h, "待确认"));
    h.get_by_label("舍入方式");
    h.get_by_role_and_label(Role::Button, "测算求解").click();
    h.run();
    h.get_by_label("多解时取值");
    h.get_by_label("测算搜索上限");
    h.get_by_role_and_label(Role::Button, "设置存储").click();
    h.run();
    h.get_by_label("设置保存位置");
    h.get_by_role_and_label(Role::Button, "外观").click();
    h.run();
    h.get_by_label("🌙  深色").click();
    h.run();
    assert_eq!(h.state().effective_theme(), ThemeChoice::Dark, "预览");
    assert_eq!(h.state().prefs.theme, ThemeChoice::Light, "未确认不保存");
    h.get_by_label("未生效");
    h.get_by_role_and_label(Role::Button, "✔").click();
    h.run();
    assert_eq!(h.state().prefs.theme, ThemeChoice::Dark);
    h.get_by_label("关闭").click();
    h.run();
    assert!(h.state().settings.is_none(), "无未生效修改时直接关闭");
}

#[test]
fn ui_settings_unconfirmed_edit_not_applied() {
    let mut h = harness(settings_at(settings::Section::Vat));
    h.run();
    assert!(
        h.query_by_role_and_label(Role::Button, "✔").is_none(),
        "未修改时不显示 √ ×"
    );
    let input = h.get_by_label("增值税征收率");
    input.click();
    h.run();
    h.get_by_label("增值税征收率").type_text("5");
    h.run();
    assert_eq!(h.state().settings.as_ref().unwrap().edit.vat_rate, "15");
    // 编辑态：出现 √ ×、“未生效”，参数不变
    h.get_by_role_and_label(Role::Button, "✔");
    h.get_by_label("✖");
    h.get_by_label("未生效");
    h.get_by_label("1 项修改未生效");
    assert_eq!(h.state().config().vat.rate, dec!(0.01));
    assert!(h.query_by_label_contains("当前值与默认值不同").is_none());
    // ×：恢复为生效值
    h.get_by_label("✖").click();
    h.run();
    assert_eq!(h.state().settings.as_ref().unwrap().edit.vat_rate, "1");
    assert!(h.query_by_label("未生效").is_none());
}

#[test]
fn ui_settings_confirm_applies_and_persists() {
    let dir = tmp_dir("ui-confirm");
    let store = SettingsStore::at(dir.join("labor-fee-desktop"));
    let mut st = AppState::with_store(store.clone());
    st.forward.input = "10000".into();
    st.calc_forward();
    st.open_settings();
    let mut h = harness(st);
    h.run();
    edit(&mut h).vat_rate = "3".into();
    h.run();
    h.get_by_role_and_label(Role::Button, "✔").click();
    h.run();
    assert_eq!(h.state().config().vat.rate, dec!(0.03));
    assert_eq!(value(&h.state().forward.outcome, "增值税税额"), "300.00");
    assert!(store.rules_path().exists());
    assert_eq!(store.load_rules().unwrap().unwrap().vat.rate, dec!(0.03));
    // 生效值与默认不同：常驻提示、“已修改”标签、单项“默认”、计数
    h.get_by_label("ℹ 当前值与默认值不同（默认：1%）");
    assert_eq!(modified_tags(&h), 1);
    h.get_by_label("与内置默认不同 1 项");
    assert!(h.query_by_label("未生效").is_none());
    // 单项“默认”：直接生效，提示消失
    h.get_by_label("🔃 默认").click();
    h.run();
    assert_eq!(h.state().config().vat.rate, dec!(0.01));
    assert!(h.query_by_label_contains("当前值与默认值不同").is_none());
    assert!(!store.rules_path().exists());
    // 改回与默认数值相同（1.0）：生效后不显示提示
    edit(&mut h).vat_rate = "1.0".into();
    h.run();
    assert!(
        h.query_by_role_and_label(Role::Button, "✔").is_none(),
        "数值相同不算修改"
    );
}

#[test]
fn ui_settings_enter_and_escape() {
    let mut h = harness(settings_at(settings::Section::Vat));
    h.run();
    let input = h.get_by_label("起征点");
    input.click();
    h.run();
    h.get_by_label("起征点").type_text("0");
    h.run();
    assert_eq!(h.state().settings.as_ref().unwrap().edit.threshold, "10000");
    h.key_press(eframe::egui::Key::Enter);
    h.run();
    assert_eq!(
        h.state().config().vat.threshold,
        dec!(10000),
        "Enter 等同 √"
    );
    // Esc：放弃本项修改，不关闭窗口
    h.get_by_label("起征点").click();
    h.run();
    h.get_by_label("起征点").type_text("5");
    h.run();
    assert!(h
        .state()
        .settings
        .as_ref()
        .unwrap()
        .is_pending("vat.threshold"));
    h.key_press(eframe::egui::Key::Escape);
    h.run();
    let w = h.state().settings.as_ref().expect("Esc 不关闭窗口");
    assert_eq!(w.edit.threshold, "10000");
    assert_eq!(w.pending_count(), 0);
    // 无未生效修改时 Esc 关闭窗口
    h.key_press(eframe::egui::Key::Escape);
    h.run();
    assert!(h.state().settings.is_none());
}

#[test]
fn ui_settings_validation_disables_confirm() {
    let mut h = harness(settings_at(settings::Section::Vat));
    edit(&mut h).vat_rate = "abc".into();
    h.run();
    h.get_by_label_contains("请输入 0～100 之间的百分数");
    h.get_by_label("增值税  (1)");
    assert!(
        h.get_by_role_and_label(Role::Button, "✔")
            .accesskit_node()
            .is_disabled(),
        "非法值时 √ 不可用"
    );
    h.key_press(eframe::egui::Key::Enter);
    h.run();
    assert_eq!(h.state().config().vat.rate, dec!(0.01));
    h.get_by_label("✖").click();
    h.run();
    assert_eq!(h.state().settings.as_ref().unwrap().edit.vat_rate, "1");
    assert!(h.query_by_label("增值税  (1)").is_none());
}

#[test]
fn ui_settings_close_with_pending_asks() {
    let mut h = harness(settings_at(settings::Section::Iit));
    edit(&mut h).fixed_deduction = "1000".into();
    h.run();
    h.get_by_label("关闭").click();
    h.run();
    h.get_by_label("有 1 项修改尚未生效");
    h.get_by_label_contains("已生效的设置不受影响");
    // 返回：窗口与输入保留
    h.get_by_label("返回").click();
    h.run();
    let w = h.state().settings.as_ref().unwrap();
    assert_eq!((w.dialog, w.edit.fixed_deduction.as_str()), (None, "1000"));
    // Esc（无焦点）同样询问
    h.key_press(eframe::egui::Key::Escape);
    h.run();
    h.get_by_label("有 1 项修改尚未生效");
    h.get_by_label("放弃未生效的修改").click();
    h.run();
    assert!(h.state().settings.is_none());
    assert_eq!(h.state().config().iit.fixed_deduction, dec!(800));
}

#[test]
fn ui_settings_reset_all() {
    let dir = tmp_dir("ui-reset");
    let store = SettingsStore::at(dir.join("labor-fee-desktop"));
    let mut st = AppState::with_store(store.clone());
    st.open_settings();
    {
        let w = st.settings.as_mut().unwrap();
        w.edit.vat_rate = "3".into();
        w.edit.items[0].enabled = true;
        w.edit.theme = ThemeChoice::Dark;
    }
    for k in ["vat.rate", "surcharge.items.0", "appearance.theme"] {
        assert!(st.confirm_setting(k), "{k}");
    }
    st.settings.as_mut().unwrap().edit.scale = "3".into(); // 未生效
    let mut h = harness(st);
    h.state_mut().prefs.theme = ThemeChoice::Dark;
    h.run();
    h.get_by_label("与内置默认不同 2 项");
    h.get_by_label("🔃 全部恢复默认").click();
    h.run();
    h.get_by_label("全部恢复默认");
    h.get_by_label_contains("当前有 2 项与内置默认不同");
    // 取消：不变
    h.get_by_label("取消").click();
    h.run();
    assert_eq!(h.state().config().vat.rate, dec!(0.03));
    h.get_by_label("🔃 全部恢复默认").click();
    h.run();
    h.get_by_label("恢复默认").click();
    h.run();
    let w = h.state().settings.as_ref().unwrap();
    assert_eq!((w.applied.modified_count(), w.pending_count()), (0, 0));
    assert!(h.state().is_default_config());
    assert_eq!(h.state().prefs.theme, ThemeChoice::System);
    assert!(!store.rules_path().exists());
    assert_eq!(store.load_prefs().theme, ThemeChoice::System);
    assert!(h.query_by_label_contains("与内置默认不同").is_none());
    assert_eq!(modified_tags(&h), 0);
}

#[test]
fn ui_menu_and_about() {
    let mut h = harness(AppState::with_default_config());
    h.get_by_label("文件").click();
    h.run();
    h.get_by_label_contains("打开批量文件…");
    h.get_by_label_contains("设置…");
    h.get_by_label_contains("退出");
    h.key_press(eframe::egui::Key::Escape);
    h.run();
    h.get_by_label("帮助").click();
    h.run();
    h.get_by_label_contains("关于").click();
    h.run();
    assert!(h.state().show_about);
    h.get_by_label(&format!("版本 {}", env!("CARGO_PKG_VERSION")));
    h.get_by_label_contains("SIL Open Font License 1.1");
    assert!(h
        .query_all_by_label_contains("不构成税务意见")
        .next()
        .is_some());
    h.get_by_label("  好的  ").click();
    h.run();
    assert!(!h.state().show_about);
    h.state_mut().show_help = true;
    h.run();
    h.get_by_label("使用说明");
}

#[test]
fn ui_shortcut_opens_settings() {
    let mut h = harness(AppState::with_default_config());
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::Comma);
    h.run();
    assert!(h.state().settings.is_some());
}

#[test]
fn close_window_dispatch() {
    use labor_fee_desktop::state::{Action, CloseTarget};
    use labor_fee_desktop::ui::window_action;
    let minimize = Some(eframe::egui::ViewportCommand::Minimized(true));
    let mut st = AppState::with_default_config();
    // 没有子窗口：最小化主窗口（不退出）
    assert_eq!(st.close_target(), CloseTarget::MainWindow);
    assert_eq!(window_action(&mut st, Action::CloseWindow), minimize);
    assert_eq!(window_action(&mut st, Action::Minimize), minimize);
    // 关于、使用说明：先关闭子窗口
    st.show_about = true;
    st.show_help = true;
    assert_eq!(window_action(&mut st, Action::CloseWindow), None);
    assert!(!st.show_about && st.show_help);
    assert_eq!(window_action(&mut st, Action::CloseWindow), None);
    assert!(!st.show_help);
    // 设置窗口：无未生效修改直接关闭
    st.open_settings();
    assert_eq!(st.close_target(), CloseTarget::Settings);
    assert_eq!(window_action(&mut st, Action::CloseWindow), None);
    assert!(st.settings.is_none());
    // 有未生效修改：弹出放弃确认；再按一次关闭确认框、回到设置窗口
    st.open_settings();
    st.settings.as_mut().unwrap().edit.vat_rate = "3".into();
    assert_eq!(window_action(&mut st, Action::CloseWindow), None);
    assert_eq!(
        st.settings.as_ref().unwrap().dialog,
        Some(settings::Dialog::DiscardOnClose)
    );
    assert_eq!(window_action(&mut st, Action::CloseWindow), None);
    let w = st.settings.as_ref().unwrap();
    assert_eq!((w.dialog, w.edit.vat_rate.as_str()), (None, "3"));
    // 最小化不影响子窗口
    assert_eq!(window_action(&mut st, Action::Minimize), minimize);
    assert!(st.settings.is_some());
}

#[test]
fn ui_ctrl_w_closes_sub_window() {
    let w = eframe::egui::Key::W;
    let cmd = eframe::egui::Modifiers::COMMAND;
    let mut h = harness(AppState::with_default_config());
    h.state_mut().show_about = true;
    h.run();
    h.key_press_modifiers(cmd, w);
    h.run();
    assert!(!h.state().show_about);
    let mut st = settings_at(settings::Section::Vat);
    st.settings.as_mut().unwrap().edit.vat_rate = "3".into();
    let mut h = harness(st);
    h.run();
    h.key_press_modifiers(cmd, w);
    h.run();
    h.get_by_label("有 1 项修改尚未生效");
    h.get_by_label("放弃未生效的修改").click();
    h.run();
    assert!(h.state().settings.is_none());
    // 文件菜单显示“关闭窗口”及快捷键
    h.get_by_label("文件").click();
    h.run();
    h.get_by_label_contains("关闭窗口");
}

#[test]
fn ui_theme_toggle_and_min_size() {
    let mut st = AppState::with_default_config();
    st.forward.input = "10000".into();
    st.calc_forward();
    let mut h = harness_sized(st, 960.0, 640.0);
    h.run();
    assert!(has(&h, "8,400.00"));
    assert_eq!(h.state().prefs.theme, ThemeChoice::Light);
    h.state_mut().set_theme(ThemeChoice::Light.next());
    h.run();
    assert_eq!(h.state().prefs.theme, ThemeChoice::Dark);
    assert_eq!(h.ctx.theme(), eframe::egui::Theme::Dark);
}

#[test]
fn window_title_has_version() {
    assert_eq!(
        labor_fee_desktop::window_title(),
        format!("劳务报酬计算工具 v{}", env!("CARGO_PKG_VERSION"))
    );
    assert!(!labor_fee_desktop::GIT_HASH.is_empty());
    assert_eq!(labor_fee_desktop::BUILD_DATE.len(), 10);
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
    assert_eq!(res.default_file_name(), "到手样例_测算结果.xlsx");
    let pv = &res.output.preview;
    let col = |n: &str| pv[0].iter().position(|x| x == n).unwrap();
    let nets: Vec<&str> = pv[1..]
        .iter()
        .map(|r| r[col("不含增值税收入")].as_str())
        .collect();
    assert_eq!(
        nets,
        ["500.00", "999.99", "1000.00", "4000.00", "23809.52", "63235.29", ""]
    );
    assert_eq!(pv[7][col("测算状态")], "数据错误");
}

/// 当前页中显示“已修改”标签的数量。
fn modified_tags(h: &Harness<'_, AppState>) -> usize {
    h.query_all_by_label("已修改").count()
}

fn window_with(d: SettingsDraft) -> AppState {
    let mut st = AppState::with_default_config();
    st.settings = Some(settings::SettingsWindow::from_draft(d));
    st
}

#[test]
fn modified_badge_count_matches_row_tags() {
    // 三项附加税费全部勾选：城市维护建设税、地方教育附加两行标“已修改”，底部计数为 2
    let mut st = settings_at(settings::Section::Surcharge);
    {
        let w = st.settings.as_mut().unwrap();
        w.edit.items[0].enabled = true;
        w.edit.items[2].enabled = true;
    }
    assert!(st.confirm_setting("surcharge.items.0"));
    assert!(st.confirm_setting("surcharge.items.2"));
    let mut h = harness(st);
    h.run();
    assert_eq!(modified_tags(&h), 2);
    assert_eq!(
        h.state()
            .settings
            .as_ref()
            .unwrap()
            .applied
            .modified_count(),
        2
    );
    h.get_by_label("与内置默认不同 2 项");

    // 各分组都有修改时，所有分组“已修改”标签之和等于底部计数
    let mut d = SettingsDraft::from_config(&labor_fee_core::Config::default(), ThemeChoice::System);
    d.vat_rate = "3".into();
    d.threshold_enabled = false;
    d.threshold = "500".into();
    d.items[0].enabled = true;
    d.items[1].rate = "2".into();
    d.items.push(settings::ItemDraft {
        name: "其他附加".into(),
        rate: "1".into(),
        enabled: true,
    });
    d.fixed_deduction = "1000".into();
    d.brackets[0].rate = "25".into();
    d.mode = labor_fee_core::RoundingMode::HalfEven;
    d.prefer = labor_fee_core::Prefer::MaxCost;
    let expect = d.modified_count();
    assert_eq!(expect, 10, "{:?}", d.modified_rows());
    let mut total = 0;
    for sec in settings::Section::ALL {
        let mut dd = d.clone();
        dd.section = sec;
        let mut h = harness(window_with(dd));
        h.run();
        total += modified_tags(&h);
        h.get_by_label(&format!("与内置默认不同 {expect} 项"));
    }
    assert_eq!(total, expect);

    // 删除默认项目：单独一行标记并计数
    let mut d = SettingsDraft::from_config(&labor_fee_core::Config::default(), ThemeChoice::System);
    d.items.remove(2);
    assert_eq!(
        d.modified_rows(),
        vec![settings::key::ITEMS_REMOVED.to_string()]
    );
    d.section = settings::Section::Surcharge;
    let mut h = harness(window_with(d));
    h.run();
    assert_eq!(modified_tags(&h), 1);
    // 恢复默认项目：直接生效
    h.get_by_label("🔃 默认").click();
    h.run();
    assert_eq!(modified_tags(&h), 0);
    assert_eq!(h.state().settings.as_ref().unwrap().edit.items.len(), 3);
}
