// Windows 发布构建不弹出控制台窗口
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! labor-fee-desktop：劳务报酬计算桌面程序。
//!
//! 用法：`labor-fee-desktop [-c|--config 口径.toml]`

use eframe::egui;
use labor_fee_desktop::{fonts, AppState, DesktopApp};

fn initial_state() -> AppState {
    let mut args = std::env::args().skip(1);
    let mut config_path = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-c" | "--config" => config_path = args.next(),
            "-h" | "--help" => {
                println!("用法：labor-fee-desktop [-c|--config 口径.toml]");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("labor-fee-desktop {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => {}
        }
    }
    let mut st = AppState::with_default_config();
    if let Some(p) = config_path {
        st.load_config_file(std::path::Path::new(&p));
        if !st.config.source.starts_with("文件") {
            // 加载失败：切到口径页签显示原因
            st.tab = labor_fee_desktop::state::Tab::Config;
        }
    }
    st
}

fn main() -> eframe::Result {
    let state = initial_state();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("劳务报酬计算工具")
            .with_inner_size([1000.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_app_id("labor-fee-desktop"),
        ..Default::default()
    };
    eframe::run_native(
        "劳务报酬计算工具",
        options,
        Box::new(|cc| {
            fonts::install(&cc.egui_ctx);
            Ok(Box::new(DesktopApp { state }))
        }),
    )
}
