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
    if let Some(d) = &mut st.settings {
        d.theme = theme;
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
        if let Some(d) = &mut st.settings {
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

        // 非默认参数：城市维护建设税 7%、教育费附加 3%、地方教育附加 2% 全部启用
        let mut cfg = labor_fee_core::Config::default();
        cfg.surcharge.items[0].rate = rust_decimal_macros::dec!(0.07);
        cfg.surcharge.items[0].enabled = true;
        cfg.surcharge.items[2].rate = rust_decimal_macros::dec!(0.02);
        cfg.surcharge.items[2].enabled = true;
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
        if let Some(d) = &mut st.settings {
            d.section = Section::Surcharge;
            d.items[0].enabled = true;
            d.items[0].rate = "7".into();
            d.items[2].enabled = true;
            d.items[2].rate = "2".into();
        }
        shoot("5b-settings-surcharge", st, theme, size, |_| {});

        st = AppState::with_default_config();
        st.tab = Tab::Batch;
        st.batch.path = sample_xlsx();
        st.run_batch_blocking();
        shoot("13-batch-min-size", st, theme, (960.0, 640.0), |_| {});
    }
}
