//! 中文字体：内嵌 Noto Sans CJK SC 的 GB2312 子集（SIL OFL 1.1，已改名 LaborFee Sans SC，见 assets/fonts/OFL.txt），
//! 保证 Windows、macOS、Linux 上中文都不显示成方块。罕见字由 eframe 的 `system_fonts` 按需从系统字体补齐。

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

/// 内嵌字体文件。
pub const EMBEDDED_FONT: &[u8] = include_bytes!("../assets/fonts/LaborFeeSansSC-Regular.otf");

/// 内嵌字体在 egui 中的名称。
pub const EMBEDDED_FONT_NAME: &str = "LaborFeeSansSC";

/// 在 egui 默认字体基础上加入内嵌中文字体：比例字体中优先使用（中英文风格一致），等宽字体中作为回退。
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        EMBEDDED_FONT_NAME.to_owned(),
        Arc::new(FontData::from_static(EMBEDDED_FONT)),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, EMBEDDED_FONT_NAME.to_owned());
    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    let pos = mono.len().min(1);
    mono.insert(pos, EMBEDDED_FONT_NAME.to_owned());
    fonts
}

/// 安装字体与界面样式。
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    ctx.all_styles_mut(|style| {
        use egui::{FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(22.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(14.0)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
    });
}
