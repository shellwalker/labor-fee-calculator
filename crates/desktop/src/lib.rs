//! 劳务报酬计算桌面程序（eframe/egui）。
//!
//! 计算全部复用 `labor-fee-core` 与 `labor-fee-batch`，本 crate 只负责状态管理与界面。

pub mod batch_store;
pub mod fonts;
pub mod icons;
#[cfg(target_os = "macos")]
pub mod menu_macos;
pub mod rules;
pub mod settings;
pub mod state;
pub mod theme;
pub mod ui;

pub use state::AppState;

/// 应用名称。
pub const APP_NAME: &str = "劳务报酬计算工具";
/// 版本号。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// 构建时的 git 短哈希。
pub const GIT_HASH: &str = env!("LFD_GIT_HASH");
/// 构建日期（UTC）。
pub const BUILD_DATE: &str = env!("LFD_BUILD_DATE");
/// 程序图标（256×256 PNG）。
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon/icon-256.png");

/// 窗口标题（含版本）。
pub fn window_title() -> String {
    format!("{APP_NAME} v{VERSION}")
}

/// eframe 应用。
pub struct DesktopApp {
    pub state: AppState,
    #[cfg(target_os = "macos")]
    pub menu: Option<menu_macos::NativeMenu>,
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        if let Some(m) = &self.menu {
            self.state.actions.extend(m.poll());
        }
        ui::draw(ui, &mut self.state);
    }
}
