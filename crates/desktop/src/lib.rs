//! 劳务报酬计算桌面程序（eframe/egui）。
//!
//! 计算全部复用 `labor-fee-core` 与 `labor-fee-batch`，本 crate 只负责状态管理与界面。

pub mod fonts;
pub mod state;
pub mod ui;

pub use state::AppState;

/// eframe 应用。
pub struct DesktopApp {
    pub state: AppState,
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        ui::draw(ui, &mut self.state);
    }
}
