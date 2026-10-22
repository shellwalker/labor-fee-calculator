//! 界面绘制：菜单栏（Windows/Linux）、品牌栏与导航、内容区、状态栏，以及设置/关于/使用说明窗口。

pub mod about;
pub mod batch;
pub mod calc;
pub mod settings;
pub mod widgets;

use crate::icons;
use crate::settings::modified_items;
use crate::state::{Action, AppState, Tab};
use crate::theme::{self, palette};
use eframe::egui::{
    self, Align, CornerRadius, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText,
    Sense, Stroke, Ui, Vec2,
};
use widgets::{badge, status_banner};

pub const SC_SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
pub const SC_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
pub const SC_QUIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);

/// 快捷键显示文字（“Ctrl+Comma”显示为“Ctrl+,”）。
pub fn shortcut_text(ctx: &egui::Context, sc: &KeyboardShortcut) -> String {
    ctx.format_shortcut(sc).replace("Comma", ",")
}

/// 对话框外框：卡片底色、大圆角、柔和阴影、24px 内边距。
pub fn modal_frame(ctx: &egui::Context) -> Frame {
    let dark = ctx.theme() == egui::Theme::Dark;
    let p = if dark { &theme::DARK } else { &theme::LIGHT };
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD + 4))
        .inner_margin(Margin::same(24))
        .shadow(egui::Shadow {
            offset: [0, 12],
            blur: 36,
            spread: 0,
            color: egui::Color32::from_black_alpha(if dark { 120 } else { 50 }),
        })
}

/// 绘制整个程序。
pub fn draw(ui: &mut Ui, st: &mut AppState) {
    let ctx = ui.ctx().clone();
    st.poll_batch();
    // 主题：设置窗口打开时预览草稿里的选择
    let want: egui::ThemePreference = st.effective_theme().into();
    if ctx.options(|o| o.theme_preference) != want {
        ctx.set_theme(want);
    }
    // 快捷键（macOS 原生菜单自己处理 Cmd+, / Cmd+O / Cmd+Q）
    if !st.native_menu {
        ctx.input_mut(|i| {
            if i.consume_shortcut(&SC_SETTINGS) {
                st.actions.push(Action::Settings);
            }
            if i.consume_shortcut(&SC_OPEN) {
                st.actions.push(Action::OpenBatchFile);
            }
            if i.consume_shortcut(&SC_QUIT) {
                st.actions.push(Action::Quit);
            }
        });
    }
    // 拖放文件：进入批量页
    if let Some(path) = ctx.input(|i| {
        i.raw.dropped_files.iter().find_map(|f| {
            let p = f.path();
            (!p.as_os_str().is_empty()).then(|| p.to_path_buf())
        })
    }) {
        st.open_batch_file(&path);
    }

    if !st.native_menu {
        menu_bar(ui, st);
    }
    brand_bar(ui, st);
    status_bar(ui, st);
    let p = palette(ui);
    egui::CentralPanel::default()
        .frame(
            Frame::new()
                .fill(p.bg)
                .inner_margin(Margin::symmetric(20, 16)),
        )
        .show(ui, |ui| {
            if let Some(n) = st.notice.clone() {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 40.0;
                    ui.allocate_ui(Vec2::new(w, 0.0), |ui| status_banner(ui, n.level, &n.text));
                    if ui
                        .small_button(icons::ERROR)
                        .on_hover_text("关闭提示")
                        .clicked()
                    {
                        st.notice = None;
                    }
                });
                ui.add_space(theme::SP2);
            }
            widgets::page_header(ui, st.tab.icon(), st.tab.title(), st.tab.description());
            match st.tab {
                Tab::Batch => batch::page(ui, st),
                t => calc::page(ui, st, t),
            }
        });

    settings::window(&ctx, st);
    about::about_window(&ctx, st);
    let sc_settings = shortcut_text(&ctx, &SC_SETTINGS);
    let sc_open = shortcut_text(&ctx, &SC_OPEN);
    about::help_window(&ctx, st, &sc_settings, &sc_open);

    for a in std::mem::take(&mut st.actions) {
        match a {
            Action::Settings => st.open_settings(),
            Action::About => st.show_about = true,
            Action::Help => st.show_help = true,
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::OpenBatchFile => {
                if let Some(pth) = batch::pick_batch_file() {
                    st.open_batch_file(&pth);
                }
            }
        }
    }
}

fn menu_bar(ui: &mut Ui, st: &mut AppState) {
    let p = palette(ui);
    egui::Panel::top("menu_bar")
        .frame(
            Frame::new()
                .fill(p.surface)
                .inner_margin(Margin::symmetric(8, 2)),
        )
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                let ctx = ui.ctx().clone();
                ui.menu_button("文件", |ui| {
                    if ui
                        .add(
                            egui::Button::new(format!("{} 打开批量文件…", icons::FOLDER))
                                .shortcut_text(shortcut_text(&ctx, &SC_OPEN)),
                        )
                        .clicked()
                    {
                        st.actions.push(Action::OpenBatchFile);
                    }
                    if ui
                        .add(
                            egui::Button::new(format!("{} 设置…", icons::SETTINGS))
                                .shortcut_text(shortcut_text(&ctx, &SC_SETTINGS)),
                        )
                        .clicked()
                    {
                        st.actions.push(Action::Settings);
                    }
                    ui.separator();
                    if ui
                        .add(egui::Button::new("退出").shortcut_text(shortcut_text(&ctx, &SC_QUIT)))
                        .clicked()
                    {
                        st.actions.push(Action::Quit);
                    }
                });
                ui.menu_button("帮助", |ui| {
                    if ui.button(format!("{} 使用说明", icons::HELP)).clicked() {
                        st.actions.push(Action::Help);
                    }
                    if ui.button(format!("{} 关于", icons::INFO)).clicked() {
                        st.actions.push(Action::About);
                    }
                });
            });
        });
}

/// 导航项：图标 + 文字，选中时品牌色底。无障碍标签为页名（如“反算”）。
fn nav_item(ui: &mut Ui, tab: Tab, selected: bool) -> egui::Response {
    let p = palette(ui);
    let font = theme::prop(15.0);
    let icon_g = ui
        .painter()
        .layout_no_wrap(tab.icon().to_string(), font.clone(), p.text);
    let text_g = ui
        .painter()
        .layout_no_wrap(tab.title().to_string(), font.clone(), p.text);
    let w = icon_g.size().x + text_g.size().x + 8.0 + 28.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 38.0), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, selected, tab.title())
    });
    let (bg, fg) = if selected {
        (p.primary_soft, p.primary)
    } else if resp.hovered() {
        (p.surface_alt, p.text)
    } else {
        (egui::Color32::TRANSPARENT, p.text_weak)
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS_CTRL + 2), bg);
    let x = rect.left() + 14.0;
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        tab.icon(),
        font.clone(),
        fg,
    );
    ui.painter().text(
        egui::pos2(x + icon_g.size().x + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        tab.title(),
        font,
        fg,
    );
    if selected {
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 12.0, rect.bottom() - 3.0),
            egui::pos2(rect.right() - 12.0, rect.bottom() - 1.0),
        );
        ui.painter()
            .rect_filled(bar, CornerRadius::same(1), p.primary);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn brand_bar(ui: &mut Ui, st: &mut AppState) {
    let p = palette(ui);
    let r = egui::Panel::top("brand_bar")
        .frame(
            Frame::new()
                .fill(p.surface)
                .inner_margin(Margin::symmetric(20, 10)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let tex = about::icon_texture(ui.ctx());
                ui.add(egui::Image::new((tex.id(), Vec2::splat(36.0))));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        RichText::new(crate::APP_NAME)
                            .size(17.0)
                            .strong()
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new("增值税 · 附加税 · 劳务报酬个税预扣")
                            .size(theme::FS_SMALL)
                            .color(p.text_weak),
                    );
                });
                ui.add_space(theme::SP5);
                for tab in Tab::ALL {
                    if nav_item(ui, tab, st.tab == tab).clicked() {
                        st.tab = tab;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::secondary_button(ui, &format!("{} 设置", icons::SETTINGS))
                        .on_hover_text(format!(
                            "打开设置（{}）",
                            shortcut_text(ui.ctx(), &SC_SETTINGS)
                        ))
                        .clicked()
                    {
                        st.open_settings();
                    }
                    let t = st.prefs.theme;
                    if widgets::secondary_button(ui, t.icon())
                        .on_hover_text(format!(
                            "主题：{}（点击切换为{}）",
                            t.title(),
                            t.next().title()
                        ))
                        .clicked()
                    {
                        st.set_theme(t.next());
                    }
                    let n = modified_items(st.config());
                    if n == 0 {
                        badge(
                            ui,
                            format!("{} {}", icons::OK, st.source),
                            p.success,
                            p.success_soft,
                        );
                    } else {
                        badge(
                            ui,
                            format!("{} {} · 已修改 {n} 项", icons::WARN, st.source),
                            p.warning,
                            p.warning_soft,
                        )
                        .on_hover_text("口径与内置默认不同，可在设置中查看或恢复默认");
                    }
                });
            });
        });
    let rect = r.response.rect;
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.bottom()),
        ],
        Stroke::new(1.0, p.border),
    );
}

fn status_bar(ui: &mut Ui, st: &AppState) {
    let p = palette(ui);
    egui::Panel::bottom("status_bar")
        .frame(
            Frame::new()
                .fill(p.surface)
                .inner_margin(Margin::symmetric(16, 4)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let n = modified_items(st.config());
                let (c, text) = if n == 0 {
                    (p.success, format!("口径：{}（与内置默认一致）", st.source))
                } else {
                    (
                        p.warning,
                        format!("口径：{}（与内置默认不同 {n} 项）", st.source),
                    )
                };
                let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, c);
                ui.label(RichText::new(text).size(theme::FS_SMALL).color(p.text_weak));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "v{} · {} · {}",
                            crate::VERSION,
                            crate::GIT_HASH,
                            crate::BUILD_DATE
                        ))
                        .size(theme::FS_SMALL)
                        .color(p.text_weak),
                    );
                    if st.batch_running() {
                        ui.spinner();
                        ui.label(
                            RichText::new("批量计算中…")
                                .size(theme::FS_SMALL)
                                .color(p.text_weak),
                        );
                    }
                });
            });
        });
}
