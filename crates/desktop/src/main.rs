// Windows 发布构建不弹出控制台窗口
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! labor-fee-desktop：劳务报酬计算桌面程序。
//!
//! 用法：`labor-fee-desktop [-c|--config 口径.toml]`
//! 主题与口径在程序“设置”中修改，自动保存到平台标准配置目录；`--config` 只对本次运行生效。

use eframe::egui;
use labor_fee_desktop::settings::SettingsStore;
use labor_fee_desktop::{fonts, AppState, DesktopApp};

fn initial_state() -> AppState {
    let mut args = std::env::args().skip(1);
    let mut config_path = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-c" | "--config" => config_path = args.next(),
            "-h" | "--help" => {
                println!("用法：labor-fee-desktop [-c|--config 口径.toml]");
                println!("主题与口径请在程序“设置”中修改（自动保存）；--config 只对本次运行生效。");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!(
                    "labor-fee-desktop {}（{}，{}）",
                    labor_fee_desktop::VERSION,
                    labor_fee_desktop::GIT_HASH,
                    labor_fee_desktop::BUILD_DATE
                );
                std::process::exit(0);
            }
            _ => {}
        }
    }
    let mut st = match SettingsStore::platform() {
        Some(store) => AppState::with_store(store),
        None => AppState::with_default_config(),
    };
    if let Some(p) = config_path {
        st.load_config_file(std::path::Path::new(&p));
    }
    st
}

fn main() -> eframe::Result {
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut state = initial_state();
    let icon =
        eframe::icon_data::from_png_bytes(labor_fee_desktop::ICON_PNG).expect("内置图标有效");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(labor_fee_desktop::window_title())
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 640.0])
            .with_icon(std::sync::Arc::new(icon))
            .with_app_id("labor-fee-desktop"),
        ..Default::default()
    };
    eframe::run_native(
        labor_fee_desktop::APP_NAME,
        options,
        Box::new(move |cc| {
            fonts::install(&cc.egui_ctx);
            #[cfg(target_os = "macos")]
            let menu = match labor_fee_desktop::menu_macos::NativeMenu::install(&cc.egui_ctx) {
                Ok(m) => {
                    state.native_menu = true;
                    Some(m)
                }
                Err(e) => {
                    eprintln!("原生菜单初始化失败，改用窗口内菜单：{e}");
                    None
                }
            };
            Ok(Box::new(DesktopApp {
                state,
                #[cfg(target_os = "macos")]
                menu,
            }))
        }),
    )
}
