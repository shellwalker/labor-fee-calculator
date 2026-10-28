//! 界面截图（wgpu 离屏渲染，用于设计自检，默认不运行）：
//!
//! ```bash
//! LFD_SHOTS_DIR=/tmp/shots cargo test -p labor-fee-desktop --test screenshots -- --ignored
//! ```
//!
//! 需要可用的 GPU 或软件渲染（Linux 可安装 mesa-vulkan-drivers 使用 lavapipe）。

use eframe::egui;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use labor_fee_desktop::settings::{Section, ThemeChoice};
use labor_fee_desktop::state::{AppState, Tab};
use labor_fee_desktop::{fonts, ui};

fn out_dir() -> std::path::PathBuf {
    let d = std::env::var("LFD_SHOTS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots")
        });
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn shoot(
    name: &str,
    mut st: AppState,
    theme: ThemeChoice,
    size: (f32, f32),
    prep: impl FnOnce(&mut Harness<'_, AppState>),
) {
    st.prefs.theme = theme;
    if let Some(w) = &mut st.settings {
        w.edit.theme = theme;
        w.applied.theme = theme;
    }
    let mut h = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .with_pixels_per_point(1.0)
        .wgpu()
        .with_context_setup(fonts::install)
        .build_ui_state(|ui, st: &mut AppState| ui::draw(ui, st), st);
    h.run();
    prep(&mut h);
    h.run();
    let img = h.render().expect("渲染失败");
    let suffix = match theme {
        ThemeChoice::Dark => "dark",
        _ => "light",
    };
    let p = out_dir().join(format!("{name}-{suffix}.png"));
    img.save(&p).unwrap();
    eprintln!("已保存 {}", p.display());
}

fn sample_xlsx() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../samples/到手样例.xlsx")
        .display()
        .to_string()
}

#[test]
#[ignore = "需要 GPU / 软件渲染，手动运行"]
fn screenshots() {
    let size = (1280.0, 820.0);
    for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
        let mut st = AppState::with_default_config();
        shoot("0-empty", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.forward.input = "10000".into();
        st.calc_forward();
        for v in ["1000", "4000", "50000"] {
            st.forward.input = v.into();
            st.calc_forward();
        }
        st.forward.input = "10000".into();
        st.calc_forward();
        shoot("1-forward", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.tab = Tab::Reverse;
        st.reverse.input = "20000".into();
        st.calc_reverse();
        shoot("2-reverse", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.tab = Tab::FromCost;
        st.from_cost.input = "1005".into();
        st.calc_from_cost();
        shoot("3-from-cost-gap", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.tab = Tab::Batch;
        st.batch.path = sample_xlsx();
        st.run_batch_blocking();
        shoot("4-batch", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.tab = Tab::Batch;
        shoot("4-batch-empty", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.open_settings();
        shoot("5-settings-vat", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.open_settings();
        if let Some(d) = st.settings.as_mut().map(|w| &mut w.edit) {
            d.section = Section::Iit;
            d.brackets[1].rate = "abc".into();
            d.fixed_deduction = "1000".into();
        }
        shoot("6-settings-iit-error", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.show_about = true;
        shoot("7-about", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.forward.input = "10000".into();
        st.calc_forward();
        shoot("8-forward-min-size", st, theme, (960.0, 640.0), |_| {});

        st = AppState::with_default_config();
        shoot("9-menu", st, theme, size, |h| {
            h.get_by_label("文件").click();
        });

        // 非默认参数：城市维护建设税改选市区 7%，与教育费附加 3%、地方教育附加 2% 三项征收
        let mut cfg = labor_fee_core::Config::default();
        cfg.surcharge.urban_construction.region = Some(labor_fee_core::UrbanRegion::Urban);
        cfg.surcharge.items[0].rate = cfg.surcharge.urban_construction.urban;
        st = AppState::new(cfg, "已保存的设置").unwrap();
        st.forward.input = "10000".into();
        st.calc_forward();
        shoot("10-forward-three-items", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.tab = Tab::FromCost;
        st.from_cost.input = "1005".into();
        st.calc_from_cost();
        shoot("11-from-cost-min-size", st, theme, (960.0, 640.0), |_| {});

        st = AppState::with_default_config();
        st.open_settings();
        shoot("12-settings-min-size", st, theme, (960.0, 640.0), |_| {});

        st = AppState::with_default_config();
        st.open_settings();
        if let Some(d) = st.settings.as_mut().map(|w| &mut w.edit) {
            d.section = Section::Surcharge;
        }
        shoot("5a-settings-surcharge-default", st, theme, size, |_| {});

        // 城建、地方教育附加选“否”（未生效）
        st = AppState::with_default_config();
        st.open_settings();
        if let Some(d) = st.settings.as_mut().map(|w| &mut w.edit) {
            d.section = Section::Surcharge;
            d.items[0].enabled = false;
            d.items[2].enabled = false;
        }
        shoot("5b-settings-surcharge", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.tab = Tab::Batch;
        st.batch.path = sample_xlsx();
        st.run_batch_blocking();
        shoot("13-batch-min-size", st, theme, (960.0, 640.0), |_| {});
    }
}

/// 设置窗口逐项确认、分组导航、全部恢复默认等（第五轮设计复核）。
#[test]
#[ignore = "需要 GPU / 软件渲染，手动运行"]
fn screenshots_settings() {
    use labor_fee_desktop::settings::{key, Dialog, ItemDraft};
    let size = (1280.0, 820.0);
    let at = |sec: Section| {
        let mut st = AppState::with_default_config();
        st.open_settings();
        st.settings.as_mut().unwrap().edit.section = sec;
        st
    };
    // 已生效：征收率 3%、城市维护建设税不征收
    let modified = |sec: Section| {
        let mut st = at(sec);
        {
            let w = st.settings.as_mut().unwrap();
            w.edit.vat_rate = "3".into();
            w.edit.items[0].enabled = false;
        }
        st.confirm_setting(key::VAT_RATE);
        st.confirm_setting("surcharge.items.0");
        st
    };
    for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
        // 1. 编辑中未确认：√ × 与“未生效”
        let mut st = at(Section::Vat);
        st.settings.as_mut().unwrap().edit.vat_rate = "3".into();
        shoot("s1-vat-editing", st, theme, size, |_| {});
        if theme == ThemeChoice::Dark {
            continue;
        }
        // 2. 生效后与默认不同：常驻提示、已修改、导航红点
        let st = modified(Section::Vat);
        shoot("s2-vat-applied-differs", st, theme, size, |_| {});
        // 3. 附加税费：已生效修改 + 新增项目待确认
        let mut st = modified(Section::Surcharge);
        st.settings.as_mut().unwrap().edit.items.push(ItemDraft {
            name: "其他附加".into(),
            rate: "0.5".into(),
            enabled: true,
        });
        shoot("s3-surcharge-new-item", st, theme, size, |_| {});
        // 4. 非法值：√ 禁用并说明原因
        let mut st = at(Section::Iit);
        st.settings.as_mut().unwrap().edit.brackets[1].rate = "abc".into();
        shoot("s4-iit-invalid", st, theme, size, |_| {});
        // 5. 计算规则分组：金额舍入、测算求解
        let mut st = at(Section::Rounding);
        st.settings.as_mut().unwrap().edit.mode = labor_fee_core::RoundingMode::HalfEven;
        shoot("s5-rounding", st, theme, size, |_| {});
        let st = at(Section::Solving);
        shoot("s6-solving", st, theme, size, |_| {});
        // 7. 全部恢复默认确认框
        let mut st = modified(Section::Vat);
        st.settings.as_mut().unwrap().dialog = Some(Dialog::ResetAll);
        shoot("s7-reset-all-confirm", st, theme, size, |_| {});
        // 8. 恢复后
        let mut st = modified(Section::Vat);
        st.reset_all_settings();
        shoot("s8-after-reset", st, theme, size, |_| {});
        // 9. 关闭时有未生效的修改
        let mut st = modified(Section::Vat);
        st.settings.as_mut().unwrap().edit.threshold = "2000".into();
        st.settings.as_mut().unwrap().dialog = Some(Dialog::DiscardOnClose);
        shoot("s9-close-pending-confirm", st, theme, size, |_| {});
        // 10. 外观、设置存储
        let mut st = at(Section::Appearance);
        st.settings.as_mut().unwrap().applied.theme = ThemeChoice::System;
        shoot("s10-appearance", st, theme, size, |_| {});
        let st = at(Section::Storage);
        shoot("s11-storage", st, theme, size, |_| {});
        // 12. 最小尺寸
        let mut st = modified(Section::Vat);
        st.settings.as_mut().unwrap().edit.threshold = "2000".into();
        shoot("s12-min-size", st, theme, (960.0, 640.0), |_| {});
        // 13. 文件菜单：关闭窗口 Ctrl+W
        let st = AppState::with_default_config();
        shoot("s13-file-menu", st, theme, size, |h| {
            h.get_by_label("文件").click();
        });
    }
}

/// 附加税费页：减征优惠、是否征收单选、合计比例（第六轮设计复核）。
#[test]
#[ignore = "需要 GPU / 软件渲染，手动运行"]
fn screenshots_surcharge() {
    use labor_fee_desktop::settings::{key, Dialog};
    let size = (1280.0, 820.0);
    let at = || {
        let mut st = AppState::with_default_config();
        st.open_settings();
        st.settings.as_mut().unwrap().edit.section = Section::Surcharge;
        st
    };
    // 已生效：三项征收，城建改选市区 7% + 减半征收
    let three = || {
        let mut st = at();
        {
            let w = st.settings.as_mut().unwrap();
            w.edit.urban_region = Some(labor_fee_core::UrbanRegion::Urban);
            w.edit.items[0].rate = "7".into();
        }
        st.confirm_setting("surcharge.items.0");
        st
    };
    for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
        // 1. 默认：减征“是”50%，三项征收（城建 1%），合计 6% → 3%
        shoot("v1-surcharge-default", at(), theme, size, |_| {});
        // 2. 城建市区 7% + 减征：合计 12% → 6%
        shoot("v2-surcharge-three-relief", three(), theme, size, |_| {});
        if theme == ThemeChoice::Dark {
            continue;
        }
        shoot(
            "v2b-surcharge-three-relief-items",
            three(),
            theme,
            size,
            |h| {
                h.get_by_label("城市维护建设税是否征收：是").scroll_to_me();
            },
        );
        // 3. 编辑中：城建选“否”（未生效，隐藏纳税人所在地，合计比例显示预览）
        let mut st = at();
        st.settings.as_mut().unwrap().edit.items[0].enabled = false;
        shoot("v3-surcharge-editing", st, theme, size, |_| {});
        let mut st = at();
        st.settings.as_mut().unwrap().edit.urban_region =
            Some(labor_fee_core::UrbanRegion::CountyTown);
        shoot("v3b-surcharge-editing-region", st, theme, size, |h| {
            h.get_by_label("纳税人所在地（城市维护建设税法第四条）")
                .scroll_to_me();
        });
        // 3c. 单项“默认”：减征已改为“否”并生效，点“默认”后填入“是，50%”待确认
        let mut st = at();
        st.settings.as_mut().unwrap().edit.relief_enabled = false;
        st.confirm_setting(key::RELIEF);
        st.reset_setting(key::RELIEF);
        shoot("v3c-surcharge-default-pending", st, theme, size, |_| {});
        // 4. 全部恢复默认确认框
        let mut st = three();
        st.settings.as_mut().unwrap().dialog = Some(Dialog::ResetAll);
        shoot("v4-reset-all-confirm", st, theme, size, |_| {});
    }
    // 5. 收入计算页：逐项列示减征前后金额
    for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
        let mut cfg = labor_fee_core::Config::default();
        cfg.surcharge.urban_construction.region = Some(labor_fee_core::UrbanRegion::Urban);
        cfg.surcharge.items[0].rate = cfg.surcharge.urban_construction.urban;
        let mut st = AppState::new(cfg, "已保存的设置").unwrap();
        st.forward.input = "10000".into();
        st.calc_forward();
        shoot("v5-forward-three-relief", st, theme, size, |_| {});
        if theme == ThemeChoice::Light {
            let mut st = AppState::with_default_config();
            st.forward.input = "10000".into();
            st.calc_forward();
            shoot("v5b-forward-default", st, theme, size, |_| {});
        }
    }
}

/// 第六轮：三项默认征收、城建档位税率可改、设置各页无口径徽章（v7 截图）。
#[test]
#[ignore = "需要 GPU / 软件渲染，手动运行"]
fn screenshots_v7() {
    let size = (1280.0, 820.0);
    let at = |sec: Section| {
        let mut st = AppState::with_default_config();
        st.open_settings();
        st.settings.as_mut().unwrap().edit.section = sec;
        st
    };
    for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
        // 1. 附加税费默认态：合计 6% → 3%
        shoot(
            "v7-1-surcharge-default",
            at(Section::Surcharge),
            theme,
            size,
            |_| {},
        );
        // 2. 城建“是”（默认）：纳税人所在地三档，默认选中 1%
        shoot(
            "v7-2-surcharge-urban-default",
            at(Section::Surcharge),
            theme,
            size,
            |h| {
                h.get_by_label("纳税人所在地（城市维护建设税法第四条）")
                    .scroll_to_me();
            },
        );
        // 3. 编辑档位税率（未生效）：县城、镇 5% → 4%，并选中该档
        let mut st = at(Section::Surcharge);
        {
            let d = &mut st.settings.as_mut().unwrap().edit;
            d.urban_region = Some(labor_fee_core::UrbanRegion::CountyTown);
            d.urban_rates[1] = "4".into();
        }
        shoot("v7-3-surcharge-urban-tier-editing", st, theme, size, |h| {
            h.get_by_label("纳税人所在地（城市维护建设税法第四条）")
                .scroll_to_me();
        });
        // 4. 增值税页、个税页（无口径状态徽章）
        shoot("v7-4-vat", at(Section::Vat), theme, size, |_| {});
        shoot("v7-5-iit", at(Section::Iit), theme, size, |_| {});
        // 6. 收入计算页默认结果：四张 KPI（含增值税税额）
        let mut st = AppState::with_default_config();
        st.forward.input = "10000".into();
        st.calc_forward();
        shoot("v7-6-forward-default", st, theme, size, |_| {});
        // 7. 实发测算、支出测算页同样四张 KPI
        let mut st = AppState::with_default_config();
        st.tab = Tab::Reverse;
        st.reverse.input = "8400".into();
        st.calc_reverse();
        shoot("v7-7-reverse-default", st, theme, size, |_| {});
        let mut st = AppState::with_default_config();
        st.tab = Tab::FromCost;
        st.from_cost.input = "10103.00".into();
        st.calc_from_cost();
        shoot("v7-8-from-cost-default", st, theme, size, |_| {});
    }
}

/// 第七轮：批量测算页“导出列”（选择、排序、各测算方式分别配置）（v8 截图）。
#[test]
#[ignore = "需要 GPU / 软件渲染，手动运行"]
fn screenshots_v8() {
    use labor_fee_batch::InputKind;
    let size = (1280.0, 1000.0);
    let dir = std::env::temp_dir().join(format!("lfd-shots-v8-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ni = dir.join("收入样例.csv");
    std::fs::write(
        &ni,
        "姓名,不含增值税收入\n甲,800\n乙,4000\n丙,10000\n丁,23809.52\n",
    )
    .unwrap();
    let tc = dir.join("支出样例.csv");
    std::fs::write(
        &tc,
        "姓名,企业总支出\n甲,1005\n乙,4000\n丙,10103.00\n丁,30000\n",
    )
    .unwrap();
    let batch = || {
        let mut st = AppState::with_default_config();
        st.tab = Tab::Batch;
        st.batch.path = sample_xlsx();
        st.run_batch_blocking();
        st
    };
    let pos = |st: &AppState, key: &str| {
        st.batch_column_entries()
            .iter()
            .position(|(c, _)| c.key() == key)
            .unwrap()
    };
    for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
        // 1. 默认：全部导出、默认顺序
        shoot("v8-1-batch-columns-default", batch(), theme, size, |_| {});
        // 2. 自定义：只导出部分列并调整顺序（税后实发金额、不含增值税收入在前）
        let mut st = batch();
        for k in [
            "vat",
            "surcharge_relief",
            "deduction",
            "taxable_income",
            "rate",
            "quick_deduction",
            "diff",
        ] {
            let i = pos(&st, k);
            st.set_batch_column_enabled(i, false);
        }
        let i = pos(&st, "take_home");
        st.move_batch_column(i, 1);
        shoot("v8-2-batch-columns-custom", st, theme, size, |_| {});
        // 3. 全不选：提示并禁用保存
        let mut st = batch();
        st.set_all_batch_columns(false);
        shoot("v8-3-batch-columns-none", st, theme, size, |_| {});
        // 4. 按不含增值税收入计算（无“差额”列）
        let mut st = AppState::with_default_config();
        st.tab = Tab::Batch;
        st.set_batch_kind(InputKind::NetIncome);
        st.batch.path = ni.display().to_string();
        st.batch.column = "不含增值税收入".into();
        st.run_batch_blocking();
        shoot("v8-4-batch-net-income", st, theme, size, |_| {});
        // 5. 按企业总支出测算
        let mut st = AppState::with_default_config();
        st.tab = Tab::Batch;
        st.set_batch_kind(InputKind::TotalCost);
        st.batch.path = tc.display().to_string();
        st.batch.column = "企业总支出".into();
        st.run_batch_blocking();
        shoot("v8-5-batch-total-cost", st, theme, size, |_| {});
        // 6. 空白状态
        let mut st = AppState::with_default_config();
        st.tab = Tab::Batch;
        shoot("v8-6-batch-empty", st, theme, size, |_| {});
        // 7. 最小窗口：导出列在预览上方
        shoot(
            "v8-7-batch-min-size",
            batch(),
            theme,
            (960.0, 640.0),
            |_| {},
        );
        // 8. 悬停说明
        shoot("v8-8-batch-column-hint", batch(), theme, size, |h| {
            h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "增值税附加税费合计")
                .hover();
        });
    }
}
