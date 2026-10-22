//! 通用界面组件：卡片、徽章、指标卡、金额行等。

use crate::state::Level;
use crate::theme::{self, palette, Palette};
use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, Response, RichText, Sense, Shadow,
    Stroke, Ui, Vec2,
};
use labor_fee_core::Decimal;

/// 千分位分隔：`"10103.00"` → `"10,103.00"`；非数字原样返回。
pub fn group_digits(s: &str) -> String {
    let t = s.trim();
    let (sign, rest) = t.strip_prefix('-').map_or(("", t), |r| ("-", r));
    let (int, frac) = rest
        .split_once('.')
        .map_or((rest, None), |(a, b)| (a, Some(b)));
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
        return s.to_string();
    }
    if frac.is_some_and(|f| !f.bytes().all(|b| b.is_ascii_digit())) {
        return s.to_string();
    }
    let mut out = String::with_capacity(t.len() + int.len() / 3);
    out.push_str(sign);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    out
}

/// 金额显示（千分位，保留口径小数位）。
pub fn money(d: Decimal) -> String {
    group_digits(&d.to_string())
}

pub fn level_colors(p: &Palette, level: Level) -> (Color32, Color32) {
    match level {
        Level::Ok => (p.success, p.success_soft),
        Level::Warn => (p.warning, p.warning_soft),
        Level::Error => (p.error, p.error_soft),
    }
}

pub fn level_icon(level: Level) -> &'static str {
    match level {
        Level::Ok => crate::icons::OK,
        Level::Warn => crate::icons::WARN,
        Level::Error => crate::icons::ERROR,
    }
}

pub fn card_frame(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(Margin::same(theme::SP4 as i8))
        .shadow(Shadow {
            offset: [0, 1],
            blur: 6,
            spread: 0,
            color: p.shadow,
        })
}

/// 卡片：占满可用宽度。
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui);
    card_frame(p)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// 卡片标题行：图标 + 标题，可选右侧内容。
pub fn card_title(ui: &mut Ui, icon: &str, title: &str, right: impl FnOnce(&mut Ui)) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(theme::FS_SECTION).color(p.primary));
        ui.label(
            RichText::new(title)
                .size(theme::FS_SECTION)
                .color(p.text)
                .strong(),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    ui.add_space(theme::SP1);
}

/// 胶囊徽章。
pub fn badge(ui: &mut Ui, text: impl Into<String>, fg: Color32, bg: Color32) -> Response {
    Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(theme::RADIUS_PILL))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(theme::FS_SMALL).color(fg))
        })
        .inner
}

/// 状态徽章（图标 + 文字，可换行）。
pub fn status_banner(ui: &mut Ui, level: Level, text: &str) {
    let p = palette(ui);
    let (fg, bg) = level_colors(p, level);
    Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, fg.gamma_multiply(0.35)))
        .corner_radius(CornerRadius::same(theme::RADIUS_CTRL + 2))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            let w = ui.available_width();
            ui.set_width(w);
            ui.set_max_width(w);
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(level_icon(level)).color(fg));
                ui.add(egui::Label::new(RichText::new(text).color(fg)).wrap());
            });
        });
}

/// 主按钮（品牌色填充）。
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let p = palette(ui);
    let fill = if enabled { p.primary } else { p.border };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(p.on_primary).strong())
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
            .min_size(Vec2::new(0.0, 34.0)),
    )
}

/// 次按钮（描边）。
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(
        egui::Button::new(RichText::new(text).color(p.text))
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
            .min_size(Vec2::new(0.0, 30.0)),
    )
}

/// 胶囊形快捷按钮（示例金额等）。
pub fn chip(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.add(
        egui::Button::new(RichText::new(text).font(theme::mono(13.0)).color(p.primary))
            .fill(p.primary_soft)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(theme::RADIUS_PILL)),
    )
}

/// 关键指标卡：标签、大号金额、单位、底部说明；`accent` 为左侧色条。
pub fn kpi_card(
    ui: &mut Ui,
    label: &str,
    value: Option<&str>,
    accent: Color32,
    note: &str,
    is_input: bool,
) {
    let p = palette(ui);
    let r = card_frame(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(p.text_weak));
            if is_input {
                badge(ui, "输入", p.primary, p.primary_soft);
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let text = value.unwrap_or("—");
            ui.label(
                RichText::new(text)
                    .font(theme::mono(theme::FS_DISPLAY))
                    .color(if value.is_some() { accent } else { p.text_weak }),
            );
            ui.add_space(2.0);
            ui.label(RichText::new("元").color(p.text_weak));
        });
        ui.add(
            egui::Label::new(RichText::new(note).size(theme::FS_SMALL).color(p.text_weak))
                .truncate(),
        );
    });
    // 左侧色条
    let rect = r.response.rect;
    let bar = egui::Rect::from_min_max(
        rect.left_top() + egui::vec2(0.0, 12.0),
        egui::pos2(rect.left() + 4.0, rect.bottom() - 12.0),
    );
    ui.painter().rect_filled(bar, CornerRadius::same(2), accent);
}

/// 明细行：标签居左、金额等宽右对齐；`strong` 为合计行（上方分隔线、加粗配色）。
pub fn amount_row(
    ui: &mut Ui,
    label: &str,
    value: &str,
    unit: &str,
    strong: bool,
    accent: Color32,
) {
    let p = palette(ui);
    if strong {
        ui.add_space(2.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(r, 0.0, p.border);
        ui.add_space(2.0);
    }
    let h = if strong { 30.0 } else { 26.0 };
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), h),
        Layout::left_to_right(Align::Center),
        |ui| {
            let lt = RichText::new(label).color(if strong { p.text } else { p.text_weak });
            ui.label(if strong { lt.strong() } else { lt });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(
                    RichText::new(if unit.is_empty() { " " } else { unit })
                        .size(theme::FS_SMALL)
                        .color(p.text_weak),
                );
                ui.label(
                    RichText::new(value)
                        .font(theme::mono(if strong { 16.5 } else { theme::FS_MONEY }))
                        .color(if strong { accent } else { p.text }),
                );
            });
        },
    );
}

/// 页面主标题 + 说明。
pub fn page_header(ui: &mut Ui, icon: &str, title: &str, desc: &str) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(theme::FS_TITLE).color(p.primary));
        ui.label(
            RichText::new(title)
                .size(theme::FS_TITLE)
                .color(p.text)
                .strong(),
        );
        ui.add_space(theme::SP2);
        ui.label(RichText::new(desc).color(p.text_weak));
    });
    ui.add_space(theme::SP2);
}

/// 弱化文字。
pub fn weak(ui: &mut Ui, text: impl Into<String>) -> Response {
    let p = palette(ui);
    ui.add(egui::Label::new(RichText::new(text.into()).color(p.text_weak)).wrap())
}

/// 小号弱化文字。
pub fn small(ui: &mut Ui, text: impl Into<String>) -> Response {
    let p = palette(ui);
    ui.add(
        egui::Label::new(
            RichText::new(text.into())
                .size(theme::FS_SMALL)
                .color(p.text_weak),
        )
        .wrap(),
    )
}
