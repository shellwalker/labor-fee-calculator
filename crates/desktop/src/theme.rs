//! 设计令牌（配色、字号、间距、圆角、阴影）与 egui 样式。设计说明见 docs/UI_DESIGN.md。

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme,
    Visuals,
};

/// 一套主题的语义色。
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// 窗口底色。
    pub bg: Color32,
    /// 卡片底色。
    pub surface: Color32,
    /// 卡片内次级底色（表头、斑马纹、输入框）。
    pub surface_alt: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub primary: Color32,
    pub primary_hover: Color32,
    pub primary_soft: Color32,
    pub on_primary: Color32,
    pub accent: Color32,
    pub success: Color32,
    pub success_soft: Color32,
    pub warning: Color32,
    pub warning_soft: Color32,
    pub error: Color32,
    pub error_soft: Color32,
    pub info_soft: Color32,
    pub shadow: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const LIGHT: Palette = Palette {
    bg: hex(0xF3F5F9),
    surface: hex(0xFFFFFF),
    surface_alt: hex(0xF6F8FB),
    border: hex(0xE2E7EF),
    text: hex(0x1B2433),
    text_weak: hex(0x667085),
    primary: hex(0x1D4ED8),
    primary_hover: hex(0x1E40AF),
    primary_soft: hex(0xE8EEFC),
    on_primary: hex(0xFFFFFF),
    accent: hex(0x0D9488),
    success: hex(0x15803D),
    success_soft: hex(0xE6F4EA),
    warning: hex(0xC2410C),
    warning_soft: hex(0xFDF0E6),
    error: hex(0xC81E1E),
    error_soft: hex(0xFCEBEB),
    info_soft: hex(0xEEF4FF),
    shadow: Color32::from_rgba_premultiplied(16, 24, 40, 18),
};

pub const DARK: Palette = Palette {
    bg: hex(0x0E131C),
    surface: hex(0x161D29),
    surface_alt: hex(0x1C2433),
    border: hex(0x273043),
    text: hex(0xE6EAF2),
    text_weak: hex(0x98A2B3),
    primary: hex(0x5B8DEF),
    primary_hover: hex(0x7AA3F5),
    primary_soft: hex(0x1B2946),
    on_primary: hex(0x0B1220),
    accent: hex(0x2DD4BF),
    success: hex(0x4ADE80),
    success_soft: hex(0x12291D),
    warning: hex(0xFB923C),
    warning_soft: hex(0x33220F),
    error: hex(0xF87171),
    error_soft: hex(0x3A1818),
    info_soft: hex(0x16223A),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 60),
};

/// 当前界面使用的配色。
pub fn palette(ui: &egui::Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

// ---- 间距（8px 栅格）、圆角、字号 ----
pub const SP1: f32 = 4.0;
pub const SP2: f32 = 8.0;
pub const SP3: f32 = 12.0;
pub const SP4: f32 = 16.0;
pub const SP5: f32 = 24.0;
pub const SP6: f32 = 32.0;

pub const RADIUS_CARD: u8 = 10;
pub const RADIUS_CTRL: u8 = 6;
pub const RADIUS_PILL: u8 = 12;

pub const FS_DISPLAY: f32 = 26.0;
pub const FS_TITLE: f32 = 20.0;
pub const FS_SECTION: f32 = 15.5;
pub const FS_BODY: f32 = 14.0;
pub const FS_SMALL: f32 = 12.0;
pub const FS_MONEY: f32 = 15.0;

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn prop(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(RADIUS_CARD + 2);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: p.shadow.gamma_multiply(2.0),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: p.shadow,
    };
    v.menu_corner_radius = CornerRadius::same(RADIUS_CTRL + 2);
    v.extreme_bg_color = p.surface;
    v.text_edit_bg_color = Some(p.surface);
    v.faint_bg_color = p.surface_alt;
    v.code_bg_color = p.surface_alt;
    v.hyperlink_color = p.primary;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.error;
    v.override_text_color = None;
    v.selection.bg_fill = if dark {
        p.primary.gamma_multiply(0.45)
    } else {
        p.primary.gamma_multiply(0.22)
    };
    v.selection.stroke = Stroke::new(1.0, p.primary);
    let r = CornerRadius::same(RADIUS_CTRL);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = p.surface_alt;
    w.inactive.weak_bg_fill = p.surface_alt;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = p.primary_soft;
    w.hovered.weak_bg_fill = p.primary_soft;
    w.hovered.bg_stroke = Stroke::new(1.0, p.primary);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.primary_soft;
    w.active.weak_bg_fill = p.primary_soft;
    w.active.bg_stroke = Stroke::new(1.5, p.primary);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open = w.active;
    v.striped = true;
    v
}

/// 安装两套样式（字号、间距、配色）。主题选择由 [`crate::settings::ThemeChoice`] 决定。
pub fn install(ctx: &egui::Context) {
    for (theme, p) in [(Theme::Light, &LIGHT), (Theme::Dark, &DARK)] {
        ctx.style_mut_of(theme, |style| {
            style.visuals = visuals(p, theme == Theme::Dark);
            style.text_styles = [
                (TextStyle::Heading, prop(FS_TITLE)),
                (TextStyle::Body, prop(FS_BODY)),
                (TextStyle::Button, prop(FS_BODY)),
                (TextStyle::Small, prop(FS_SMALL)),
                (TextStyle::Monospace, mono(FS_BODY)),
            ]
            .into();
            let s = &mut style.spacing;
            s.item_spacing = egui::vec2(SP2, SP2);
            s.button_padding = egui::vec2(SP3, 5.0);
            s.interact_size.y = 28.0;
            s.window_margin = Margin::same(SP4 as i8);
            s.menu_margin = Margin::same(6);
            s.combo_height = 300.0;
        });
    }
}
