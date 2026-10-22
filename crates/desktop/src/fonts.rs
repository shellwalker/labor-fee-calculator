//! 字体：
//! - 中文：内嵌 Noto Sans CJK SC 的 GB2312 子集（SIL OFL 1.1，已改名 LaborFee Sans SC），保证三平台中文不显示成方块；
//! - 图标：内嵌 Noto Emoji 子集（SIL OFL 1.1，已改名 LaborFee Icons），只含界面用到的图标；
//! - 数字：等宽字体用 egui 自带的 Hack，金额列对位整齐。
//!
//! 许可证见 assets/fonts/OFL.txt。罕见字由 eframe 的 `system_fonts` 按需从系统字体补齐。

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontTweak};
use std::sync::Arc;

/// 内嵌中文字体文件。
pub const EMBEDDED_FONT: &[u8] = include_bytes!("../assets/fonts/LaborFeeSansSC-Regular.otf");
/// 内嵌图标字体文件。
pub const ICON_FONT: &[u8] = include_bytes!("../assets/fonts/LaborFeeIcons-Regular.ttf");

/// 内嵌中文字体在 egui 中的名称。
pub const EMBEDDED_FONT_NAME: &str = "LaborFeeSansSC";
/// 内嵌图标字体在 egui 中的名称。
pub const ICON_FONT_NAME: &str = "LaborFeeIcons";

/// 在 egui 默认字体基础上加入内嵌字体：比例字体中文优先（中英文风格一致），等宽字体中文作回退；图标字体在最后回退。
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        EMBEDDED_FONT_NAME.to_owned(),
        Arc::new(FontData::from_static(EMBEDDED_FONT)),
    );
    fonts.font_data.insert(
        ICON_FONT_NAME.to_owned(),
        Arc::new(FontData::from_static(ICON_FONT).tweak(FontTweak {
            scale: 0.86,
            y_offset_factor: 0.04,
            ..Default::default()
        })),
    );
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, EMBEDDED_FONT_NAME.to_owned());
    prop.push(ICON_FONT_NAME.to_owned());
    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    let pos = mono.len().min(1);
    mono.insert(pos, EMBEDDED_FONT_NAME.to_owned());
    mono.push(ICON_FONT_NAME.to_owned());
    fonts
}

/// 安装字体与界面样式（浅色、深色两套）。
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    crate::theme::install(ctx);
}
