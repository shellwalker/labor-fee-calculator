//! 设置窗口：左侧分组导航，右侧逐项编辑。每项修改后处于“未生效”状态，
//! 点击 √（或 Enter）生效并立即保存，点击 ×（或 Esc）放弃本项修改；
//! 另有单项“默认”（填入默认值、进入未生效状态，仍须点击 √ 生效）与“全部恢复默认”（二次确认后生效）。

use super::widgets::{badge, danger_button, primary_button, secondary_button, small};
use crate::icons;
use crate::settings::{
    key, prefer_text, rounding_mode_text, surcharge_base_text, surcharge_rounding_text,
    BracketDraft, Dialog, Errors, Group, ItemDraft, Section, SettingsDraft, SettingsWindow,
    ThemeChoice,
};
use crate::state::AppState;
use crate::theme::{self, palette};
use eframe::egui::{
    self, Align, Color32, ComboBox, CornerRadius, Frame, Id, Key, Layout, Margin, Modal, Modifiers,
    Response, RichText, ScrollArea, Stroke, TextEdit, Ui, Vec2,
};
use labor_fee_core::{Prefer, RoundingMode, SurchargeBase, SurchargeRounding};

/// “未生效”标签（描边样式，与实心的状态标签区分）。
fn unapplied_badge(ui: &mut Ui) {
    let p = palette(ui);
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.warning))
        .corner_radius(CornerRadius::same(theme::RADIUS_PILL))
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| {
            ui.label(
                RichText::new("未生效")
                    .size(theme::FS_SMALL)
                    .color(p.warning),
            )
        })
        .inner
        .on_hover_text("本项修改尚未生效：点击 ✔ 或按 Enter 生效，点击 ✖ 或按 Esc 放弃");
}

const LABEL_W: f32 = 180.0;
const CTRL_W: f32 = 240.0;
/// 城市维护建设税档位单选的宽度（使右侧税率输入框对齐）。
const URBAN_RADIO_W: f32 = 290.0;
const NAV_W: f32 = 176.0;

/// 设置窗口内的操作（本帧绘制结束后执行）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Act {
    Confirm(String),
    Discard(String),
    Default(String),
    DeleteItem(usize),
}

/// 绘制表单时共用的状态。
struct Form {
    acts: Vec<Act>,
    /// 本帧的 Esc 已用于放弃某项修改（不再关闭窗口）。
    esc_used: bool,
    /// 未生效各行确认后的校验错误。
    errors: Errors,
    pending: Vec<String>,
    /// 生效值与内置默认不同的行。
    modified: Vec<String>,
}

impl Form {
    fn pending(&self, k: &str) -> bool {
        self.pending.iter().any(|x| x == k)
    }
    fn modified(&self, k: &str) -> bool {
        self.modified.iter().any(|x| x == k)
    }
    /// 本行（含子字段）的第一条错误。
    fn row_error(&self, k: &str) -> Option<String> {
        self.errors
            .iter()
            .find(|(ek, _)| {
                ek.as_str() == k || ek.strip_prefix(k).is_some_and(|r| r.starts_with('.'))
            })
            .map(|(_, m)| m.clone())
    }

    /// 键盘：焦点在本行控件上时，Enter 等同 √，Esc 等同 ×。
    fn keys(&mut self, ui: &Ui, k: &str, r: &Response, can_confirm: bool) {
        if !self.pending(k) || !(r.has_focus() || r.lost_focus()) {
            return;
        }
        if can_confirm && ui.input(|i| i.key_pressed(Key::Enter)) {
            self.acts.push(Act::Confirm(k.to_string()));
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.acts.push(Act::Discard(k.to_string()));
            self.esc_used = true;
        }
    }

    /// 行内操作区：未生效时 √ × 与“未生效”，生效值与默认不同时单项“默认”。
    ///
    /// 不再显示口径状态徽章（已确认 / 已核实等，见 docs/ASSUMPTIONS.md）；“已修改”由下方常驻提示
    /// “当前值与默认值不同（默认：…）”与左侧导航红点表达，不另设徽章。
    fn actions(&mut self, ui: &mut Ui, k: &str, can_default: bool) {
        let p = palette(ui);
        if self.pending(k) {
            let err = self.row_error(k);
            let ok = ui
                .add_enabled(
                    err.is_none(),
                    egui::Button::new(RichText::new(icons::OK).color(if err.is_none() {
                        Color32::WHITE
                    } else {
                        p.text_weak
                    }))
                    .fill(if err.is_none() {
                        p.success
                    } else {
                        p.surface_alt
                    })
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
                    .min_size(Vec2::new(28.0, 26.0)),
                )
                .on_hover_text("确认生效（Enter）")
                .on_disabled_hover_text(format!(
                    "无法生效：{}",
                    err.as_deref().unwrap_or_default()
                ));
            if ok.clicked() {
                self.acts.push(Act::Confirm(k.to_string()));
            }
            let cancel = ui
                .add(
                    egui::Button::new(RichText::new(icons::ERROR).color(p.text_weak))
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
                        .min_size(Vec2::new(28.0, 26.0)),
                )
                .on_hover_text("放弃本项修改（Esc）");
            if cancel.clicked() {
                self.acts.push(Act::Discard(k.to_string()));
            }
            unapplied_badge(ui);
        }
        if self.modified(k) && can_default {
            ui.add_space(2.0);
            if ui
                .small_button(format!("{} 默认", icons::RESET))
                .on_hover_text("填入本项的内置默认值；点击 ✔ 生效，点击 ✖ 放弃")
                .clicked()
            {
                self.acts.push(Act::Default(k.to_string()));
            }
        }
    }

    /// 行下方的补充说明区：错误、未生效提示、常驻的“与默认值不同”提示、帮助文字。
    fn notes(&self, ui: &mut Ui, k: &str, help: &str, default_text: Option<&str>) {
        let p = palette(ui);
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + 8.0);
            ui.vertical(|ui| {
                if self.pending(k) {
                    if let Some(e) = self.row_error(k) {
                        ui.label(
                            RichText::new(format!("{} {e}", icons::ERROR))
                                .size(theme::FS_SMALL)
                                .color(p.error),
                        );
                    }
                }
                if self.modified(k) {
                    let text = match default_text {
                        Some(d) => format!("{} 当前值与默认值不同（默认：{d}）", icons::INFO),
                        None => format!("{} 当前值与默认值不同（内置默认无此项目）", icons::INFO),
                    };
                    ui.label(RichText::new(text).size(theme::FS_SMALL).color(p.warning));
                }
                if !help.is_empty() {
                    small(ui, help);
                }
            });
        });
        ui.add_space(2.0);
    }
}

/// 未生效的控件外加描边。
fn mark_pending(ui: &Ui, r: &Response, pending: bool) {
    if pending {
        let p = palette(ui);
        ui.painter().rect_stroke(
            r.rect.expand(2.0),
            CornerRadius::same(theme::RADIUS_CTRL),
            Stroke::new(1.5, p.warning),
            egui::StrokeKind::Outside,
        );
    }
}

/// 一行设置：标签 | 控件 | √ × 未生效 | 单项默认；下方补充说明。
fn row(
    ui: &mut Ui,
    f: &mut Form,
    k: &str,
    label: &str,
    help: &str,
    default_text: &str,
    control: impl FnOnce(&mut Ui, Id) -> Response,
) {
    let p = palette(ui);
    let pending = f.pending(k);
    ui.horizontal(|ui| {
        let label_id = ui
            .allocate_ui_with_layout(
                Vec2::new(LABEL_W, 30.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_width(LABEL_W);
                    ui.label(RichText::new(label).color(p.text)).id
                },
            )
            .inner;
        let r = ui
            .allocate_ui_with_layout(
                Vec2::new(CTRL_W + 60.0, 30.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_width(CTRL_W + 60.0);
                    control(ui, label_id)
                },
            )
            .inner;
        mark_pending(ui, &r, pending);
        let can = f.row_error(k).is_none();
        f.keys(ui, k, &r, can);
        f.actions(ui, k, true);
    });
    f.notes(ui, k, help, Some(default_text));
}

fn field_bg(ui: &Ui, error: bool, pending: bool) -> Color32 {
    let p = palette(ui);
    if error {
        p.error_soft
    } else if pending {
        p.warning_soft
    } else {
        p.surface
    }
}

fn text_field(
    ui: &mut Ui,
    label: Id,
    value: &mut String,
    unit: &str,
    bg: Color32,
    enabled: bool,
) -> Response {
    let p = palette(ui);
    let r = ui
        .add_enabled(
            enabled,
            TextEdit::singleline(value)
                .font(theme::mono(14.0))
                .desired_width(CTRL_W - 30.0)
                .background_color(bg),
        )
        .labelled_by(label);
    ui.label(RichText::new(unit).color(p.text_weak));
    r
}

fn group_title(ui: &mut Ui, title: &str) {
    let p = palette(ui);
    ui.add_space(theme::SP2);
    ui.label(
        RichText::new(title)
            .size(theme::FS_SECTION)
            .strong()
            .color(p.text),
    );
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, p.border);
    ui.add_space(theme::SP1);
}

/// 左侧分组导航：一级为分组标题（不可点击），二级为页；页名右侧小圆点提示“已修改”（红）、“未生效”（橙）。
fn nav(ui: &mut Ui, w: &mut SettingsWindow, f: &Form) {
    let p = palette(ui);
    for (gi, g) in Group::ALL.iter().enumerate() {
        if gi > 0 {
            ui.add_space(theme::SP2);
        }
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(
                RichText::new(g.title())
                    .size(theme::FS_SMALL)
                    .strong()
                    .color(p.text_weak),
            );
        });
        for &s in g.sections() {
            let n_err = f
                .errors
                .keys()
                .filter(|k| Section::of_key(k) == Some(s))
                .count();
            let n_mod = w.section_modified(s);
            let n_pend = w.section_pending(s);
            let text = if n_err > 0 {
                format!("{}  ({n_err})", s.title())
            } else {
                s.title().to_string()
            };
            let selected = w.edit.section == s;
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                // 自绘导航项：文字左对齐，选中时品牌色浅底 + 左侧色条
                let (rect, mut r) =
                    ui.allocate_exact_size(Vec2::new(NAV_W - 12.0, 32.0), egui::Sense::click());
                r.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Button, true, selected, &text)
                });
                let bg = if selected {
                    p.primary_soft
                } else if r.hovered() {
                    p.surface_alt
                } else {
                    Color32::TRANSPARENT
                };
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(theme::RADIUS_CTRL), bg);
                if selected {
                    let bar = egui::Rect::from_min_size(
                        rect.left_top() + Vec2::new(0.0, 7.0),
                        Vec2::new(3.0, rect.height() - 14.0),
                    );
                    ui.painter()
                        .rect_filled(bar, CornerRadius::same(1), p.primary);
                }
                ui.painter().text(
                    egui::pos2(rect.left() + 14.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &text,
                    theme::prop(theme::FS_BODY),
                    if selected {
                        p.primary
                    } else if n_err > 0 {
                        p.error
                    } else {
                        p.text
                    },
                );
                if r.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                // 小圆点：已修改（红）、未生效（橙）
                let mut x = rect.right() - 12.0;
                for (n, c) in [(n_pend, p.warning), (n_mod, p.error)] {
                    if n > 0 {
                        ui.painter()
                            .circle_filled(egui::pos2(x, rect.center().y), 3.5, c);
                        x -= 11.0;
                    }
                }
                let mut tips = Vec::new();
                if n_mod > 0 {
                    tips.push(format!("{n_mod} 项与内置默认不同"));
                }
                if n_pend > 0 {
                    tips.push(format!("{n_pend} 项修改尚未生效"));
                }
                if !tips.is_empty() {
                    r = r.on_hover_text(tips.join("；"));
                }
                if r.clicked() {
                    w.edit.section = s;
                }
            });
        }
    }
}

/// 绘制设置窗口（打开时）。
pub fn window(ctx: &egui::Context, st: &mut AppState) {
    let Some(w) = st.settings.as_ref() else {
        return;
    };
    let store_dir = st.store.as_ref().map(|s| s.dir.display().to_string());
    let dialog = w.dialog;
    let mut form = Form {
        acts: Vec::new(),
        esc_used: false,
        errors: w.row_errors(),
        pending: w.pending_rows(),
        modified: w.applied.modified_rows(),
    };
    let mut close = false;
    let mut open_reset = false;
    let resp = Modal::new(Id::unique("settings_modal"))
        .frame(super::modal_frame(ctx))
        .show(ctx, |ui| {
            let p = palette(ui);
            let Some(w) = st.settings.as_mut() else {
                return;
            };
            let size = ctx.content_rect().size();
            let width = (size.x - 80.0).clamp(660.0, 920.0);
            let h = (size.y - 220.0).clamp(300.0, 560.0);
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::SETTINGS)
                        .size(theme::FS_TITLE)
                        .color(p.primary),
                );
                ui.label(
                    RichText::new("设置")
                        .size(theme::FS_TITLE)
                        .strong()
                        .color(p.text),
                );
                ui.add_space(theme::SP2);
                small(
                    ui,
                    "修改后点击 ✔ 生效，点击 ✖ 放弃本项修改；生效的设置立即保存。",
                );
            });
            ui.add_space(theme::SP2);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(NAV_W, h),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(NAV_W);
                        nav(ui, w, &form);
                    },
                );
                let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, h), egui::Sense::hover());
                ui.painter().rect_filled(sep, 0.0, p.border);
                ui.allocate_ui_with_layout(
                    Vec2::new(width - NAV_W - 20.0, h),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_max_height(h);
                        ScrollArea::vertical()
                            .id_salt("settings_form")
                            .max_height(h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| match w.edit.section {
                                Section::Vat => vat_section(ui, &mut w.edit, &mut form),
                                Section::Surcharge => {
                                    surcharge_section(ui, &mut w.edit, &w.applied, &mut form)
                                }
                                Section::Iit => iit_section(ui, &mut w.edit, &mut form),
                                Section::Rounding => rounding_section(ui, &mut w.edit, &mut form),
                                Section::Solving => solving_section(ui, &mut w.edit, &mut form),
                                Section::Appearance => {
                                    appearance_section(ui, &mut w.edit, &mut form)
                                }
                                Section::Storage => storage_section(ui, store_dir.as_deref()),
                            });
                    },
                );
            });
            ui.add_space(theme::SP2);
            if let Some(m) = &w.message {
                super::widgets::status_banner(ui, crate::state::Level::Error, m);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let reset = ui
                    .add(
                        egui::Button::new(
                            RichText::new(format!("{} 全部恢复默认", icons::RESET)).color(p.error),
                        )
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.error.gamma_multiply(0.6)))
                        .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
                        .min_size(Vec2::new(0.0, 30.0)),
                    )
                    .on_hover_text("将全部计算参数和外观偏好恢复为内置默认并立即生效（需确认）");
                if reset.clicked() {
                    open_reset = true;
                }
                let n = form.modified.len();
                if n > 0 {
                    badge(ui, format!("与内置默认不同 {n} 项"), p.error, p.error_soft);
                }
                let np = form.pending.len();
                if np > 0 {
                    badge(ui, format!("{np} 项修改未生效"), p.warning, p.warning_soft);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.scope(|ui| {
                        ui.spacing_mut().button_padding.x = 20.0;
                        if primary_button(ui, "关闭", true).clicked() {
                            close = true;
                        }
                    });
                });
            });
        });
    // Esc：先用于放弃焦点所在行的修改；否则（且没有打开的下拉框、确认框）请求关闭窗口
    if dialog.is_none()
        && !form.esc_used
        && resp.is_top_modal
        && !resp.any_popup_open
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
        close = true;
    }
    if dialog.is_none() && resp.backdrop_response.clicked() {
        close = true;
    }
    for a in form.acts {
        match a {
            Act::Confirm(k) => {
                st.confirm_setting(&k);
            }
            Act::Discard(k) => st.discard_setting(&k),
            Act::Default(k) => {
                st.reset_setting(&k);
            }
            Act::DeleteItem(i) => {
                st.delete_item(i);
            }
        }
    }
    if open_reset {
        if let Some(w) = &mut st.settings {
            w.dialog = Some(Dialog::ResetAll);
        }
    } else if close {
        st.request_close_settings();
    }
    if let Some(d) = dialog {
        confirm_dialog(ctx, st, d);
    }
}

/// 二次确认对话框（危险操作样式）。
fn confirm_dialog(ctx: &egui::Context, st: &mut AppState, d: Dialog) {
    let Some(w) = st.settings.as_ref() else {
        return;
    };
    let (title, body, ok_text, cancel_text) = match d {
        Dialog::ResetAll => (
            "全部恢复默认".to_string(),
            {
                let n = w.applied.modified_count();
                let mut t = String::from(
                    "将把全部计算参数（增值税、附加税费、个人所得税、金额舍入、测算求解）和外观偏好恢复为内置默认值，并立即生效；已保存的设置文件（rules.toml、preferences.toml）同步还原，尚未生效的修改一并放弃。",
                );
                if n > 0 {
                    t += &format!("当前有 {n} 项与内置默认不同。");
                }
                t += "此操作不可撤销。";
                t
            },
            "恢复默认",
            "取消",
        ),
        Dialog::DiscardOnClose => (
            format!("有 {} 项修改尚未生效", w.pending_count()),
            "关闭设置窗口将放弃这些修改，已生效的设置不受影响。".to_string(),
            "放弃未生效的修改",
            "返回",
        ),
    };
    let mut ok = false;
    let mut back = false;
    let resp = Modal::new(Id::unique("settings_confirm"))
        .frame(super::modal_frame(ctx))
        .show(ctx, |ui| {
            let p = palette(ui);
            ui.set_width(440.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::WARN)
                        .size(theme::FS_TITLE)
                        .color(p.error),
                );
                ui.label(
                    RichText::new(&title)
                        .size(theme::FS_SECTION + 1.0)
                        .strong()
                        .color(p.text),
                );
            });
            ui.add_space(theme::SP2);
            ui.add(egui::Label::new(RichText::new(&body).color(p.text)).wrap());
            ui.add_space(theme::SP4);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if danger_button(ui, ok_text).clicked() {
                    ok = true;
                }
                let b = secondary_button(ui, cancel_text);
                if b.clicked() {
                    back = true;
                }
            });
        });
    if resp.should_close() {
        back = true;
    }
    if ok {
        match d {
            Dialog::ResetAll => {
                st.reset_all_settings();
            }
            Dialog::DiscardOnClose => st.close_settings(),
        }
    } else if back {
        if let Some(w) = &mut st.settings {
            w.dialog = None;
        }
    }
}

fn on_off(b: bool, on: &str, off: &str) -> String {
    if b { on } else { off }.to_string()
}

fn vat_section(ui: &mut Ui, d: &mut SettingsDraft, f: &mut Form) {
    let def = d.defaults();
    group_title(ui, "征收率");
    let bg = field_bg(
        ui,
        f.row_error(key::VAT_RATE).is_some(),
        f.pending(key::VAT_RATE),
    );
    row(
        ui,
        f,
        key::VAT_RATE,
        "增值税征收率",
        "当前按 1% 优惠征收率（小规模纳税人 3% 征收率减按 1%，财政部 税务总局公告2026年第10号）；可改为 3% 等",
        &format!("{}%", def.vat_rate),
        |ui, l| text_field(ui, l, &mut d.vat_rate, "%", bg, true),
    );
    group_title(ui, "增值税起征点");
    row(
        ui,
        f,
        key::THRESHOLD_ENABLED,
        "启用增值税起征点",
        "未达到起征点时，不征收增值税及附加税费；个人所得税不受影响",
        &on_off(def.threshold_enabled, "启用", "不启用"),
        |ui, _| ui.checkbox(&mut d.threshold_enabled, "启用"),
    );
    let en = d.threshold_enabled;
    let bg = field_bg(
        ui,
        f.row_error(key::THRESHOLD).is_some(),
        f.pending(key::THRESHOLD),
    );
    row(
        ui,
        f,
        key::THRESHOLD,
        "起征点",
        "按不含增值税收入判断。按次纳税的起征点为每次（日）销售额 1000 元（财政部 税务总局公告2026年第10号）",
        &format!("{} 元", def.threshold),
        |ui, l| text_field(ui, l, &mut d.threshold, "元", bg, en),
    );
    row(
        ui,
        f,
        key::LEVY_AT,
        "恰好等于起征点时",
        "勾选：达到起征点即征收（含本数）；不勾选：超过起征点才征收",
        &on_off(def.levy_at_threshold, "征收", "不征收"),
        |ui, _| ui.add_enabled(en, egui::Checkbox::new(&mut d.levy_at_threshold, "征收")),
    );
}

/// “是否征收”等是非选择：否在前、是在后（单选按钮）。
///
/// 无障碍名称为“{name}：否”“{name}：是”，便于读屏与测试区分各行。
fn yes_no(ui: &mut Ui, v: &mut bool, name: &str) -> Response {
    let p = palette(ui);
    // 未选中的单选圈加深描边，浅色主题下也清晰可辨
    ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, p.text_weak);
    ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::new(1.0, p.primary);
    let mut out: Option<Response> = None;
    for (val, text) in [(false, "否"), (true, "是")] {
        let r = ui.radio_value(v, val, text);
        let sel = *v == val;
        r.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::RadioButton,
                true,
                sel,
                format!("{name}：{text}"),
            )
        });
        out = Some(match out {
            Some(a) => a.union(r),
            None => r,
        });
    }
    out.expect("两个选项")
}

/// 行首的标签格（与 `row` 的标签列对齐），返回标签的 Id（供控件 labelled_by）。
fn label_cell(ui: &mut Ui, text: &str) -> Id {
    let p = palette(ui);
    ui.allocate_ui_with_layout(
        Vec2::new(LABEL_W, 30.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(LABEL_W);
            ui.label(RichText::new(text).color(p.text)).id
        },
    )
    .inner
}

/// 控件格（与 `row` 的控件列对齐）。
fn control_cell(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    ui.allocate_ui_with_layout(
        Vec2::new(CTRL_W + 60.0, 30.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(CTRL_W + 60.0);
            add(ui)
        },
    )
    .inner
}

/// 附加税费合计比例卡片：按已生效设置列出各征收项目的法定费率、减征比例与实际征收率；
/// 本页有未生效的修改时，另附含未生效修改的预览。
fn surcharge_summary_card(ui: &mut Ui, applied: &SettingsDraft, edit: &SettingsDraft, f: &Form) {
    let p = palette(ui);
    let pct = crate::rules::pct;
    let page_pending = f
        .pending
        .iter()
        .any(|k| Section::of_key(k) == Some(Section::Surcharge));
    Frame::new()
        .fill(p.surface_alt)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("附加税费合计比例")
                        .size(theme::FS_SECTION)
                        .strong()
                        .color(p.text),
                );
                small(
                    ui,
                    format!(
                        "占计税依据（{}），按已生效的设置",
                        surcharge_base_text(applied.surcharge_base)
                    ),
                );
            });
            let Some(sum) = applied.surcharge_summary() else {
                return;
            };
            ui.horizontal(|ui| {
                if sum.relief.is_zero() {
                    ui.label(
                        RichText::new(pct(sum.actual_total))
                            .size(theme::FS_DISPLAY)
                            .strong()
                            .color(p.primary),
                    );
                    small(ui, "未享受减征优惠");
                } else {
                    ui.label(
                        RichText::new(pct(sum.statutory_total))
                            .size(theme::FS_TITLE)
                            .strikethrough()
                            .color(p.text_weak),
                    );
                    ui.label(
                        RichText::new(icons::RIGHT)
                            .size(theme::FS_TITLE)
                            .color(p.text_weak),
                    );
                    ui.label(
                        RichText::new(pct(sum.actual_total))
                            .size(theme::FS_DISPLAY)
                            .strong()
                            .color(p.primary),
                    );
                    small(
                        ui,
                        format!(
                            "减征前 {} → 减征 {} 后 {}",
                            pct(sum.statutory_total),
                            pct(sum.relief),
                            pct(sum.actual_total)
                        ),
                    );
                }
            });
            if sum.lines.is_empty() {
                small(ui, "未征收任何附加税费项目");
            } else {
                egui::Grid::new("surcharge_summary")
                    .striped(true)
                    .num_columns(4)
                    .min_col_width(96.0)
                    .spacing(Vec2::new(24.0, 4.0))
                    .show(ui, |ui| {
                        for h in ["税费项目", "法定费率", "减征", "实际征收率"] {
                            ui.label(
                                RichText::new(h)
                                    .size(theme::FS_SMALL)
                                    .strong()
                                    .color(p.text_weak),
                            );
                        }
                        ui.end_row();
                        let relief = |r: labor_fee_core::Decimal| {
                            if r.is_zero() {
                                "—".to_string()
                            } else {
                                pct(r)
                            }
                        };
                        for l in &sum.lines {
                            ui.label(RichText::new(&l.name).color(p.text));
                            ui.label(RichText::new(pct(l.statutory)).font(theme::mono(14.0)));
                            ui.label(RichText::new(relief(l.relief)).font(theme::mono(14.0)));
                            ui.label(
                                RichText::new(pct(l.actual))
                                    .font(theme::mono(14.0))
                                    .color(p.primary),
                            );
                            ui.end_row();
                        }
                        ui.label(RichText::new("合计").strong().color(p.text));
                        ui.label(
                            RichText::new(pct(sum.statutory_total))
                                .font(theme::mono(14.0))
                                .strong(),
                        );
                        ui.label(RichText::new(relief(sum.relief)).font(theme::mono(14.0)));
                        ui.label(
                            RichText::new(pct(sum.actual_total))
                                .font(theme::mono(14.0))
                                .strong()
                                .color(p.primary),
                        );
                        ui.end_row();
                    });
            }
            if page_pending {
                if let Some(prev) = edit.surcharge_summary().filter(|x| *x != sum) {
                    ui.add_space(theme::SP1);
                    ui.label(
                        RichText::new(format!(
                            "{} 含未生效修改的预览：合计 {} → 减征后 {}（点击 ✔ 生效后计入）",
                            icons::INFO,
                            pct(prev.statutory_total),
                            pct(prev.actual_total)
                        ))
                        .size(theme::FS_SMALL)
                        .color(p.warning),
                    );
                }
            }
        });
    ui.add_space(theme::SP1);
}

/// 减征政策来源（小字，附链接）。
fn relief_source(ui: &mut Ui) {
    use crate::rules::{
        RELIEF_POLICY_NAME, RELIEF_POLICY_URL, RELIEF_POLICY_URL_TAX, RELIEF_QA_NAME, RELIEF_QA_URL,
    };
    let p = palette(ui);
    let t = |s: &str| RichText::new(s).size(theme::FS_SMALL).color(p.text_weak);
    let link = |s: &str| RichText::new(s).size(theme::FS_SMALL);
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + 8.0);
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.label(t("政策依据："));
                ui.hyperlink_to(link(RELIEF_POLICY_NAME), RELIEF_POLICY_URL)
                    .on_hover_text(RELIEF_POLICY_URL);
                ui.label(t("第二条（"));
                ui.hyperlink_to(link("国家税务总局政策法规库"), RELIEF_POLICY_URL_TAX)
                    .on_hover_text(RELIEF_POLICY_URL_TAX);
                ui.label(t("）"));
            });
            ui.label(t(
                "2023 年 1 月 1 日至 2027 年 12 月 31 日，对增值税小规模纳税人减半征收城市维护建设税、教育费附加、地方教育附加。",
            ));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.label(t("自然人按小规模纳税人享受，可在代开发票环节适用（"));
                ui.hyperlink_to(link(RELIEF_QA_NAME), RELIEF_QA_URL)
                    .on_hover_text(RELIEF_QA_URL);
                ui.label(t("）"));
            });
        });
    });
    ui.add_space(2.0);
}

/// 行下方的展开内容（缩进到控件列）。
fn indented<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + 8.0);
        add(ui)
    })
    .inner
}

fn surcharge_section(ui: &mut Ui, d: &mut SettingsDraft, applied: &SettingsDraft, f: &mut Form) {
    let def = d.defaults();
    let p = palette(ui);
    small(
        ui,
        "各项附加税费 = 计税依据 × 法定费率 × (1 − 减征比例)。“是否征收”选“是”后显示具体配置；所有修改点击 ✔ 后生效并保存。",
    );
    ui.add_space(theme::SP1);
    surcharge_summary_card(ui, applied, d, f);

    // ---- 减征优惠（全局） ----
    group_title(ui, "减征优惠");
    {
        let k = key::RELIEF;
        let pending = f.pending(k);
        let can = f.row_error(k).is_none();
        let bg = field_bg(ui, !can && pending, pending);
        let was = d.relief_enabled;
        ui.horizontal(|ui| {
            label_cell(ui, "享受减征优惠");
            let r = control_cell(ui, |ui| yes_no(ui, &mut d.relief_enabled, "享受减征优惠"));
            mark_pending(ui, &r, pending);
            f.keys(ui, k, &r, can);
            f.actions(ui, k, true);
        });
        if was && !d.relief_enabled {
            // 改为“否”时，隐藏的减征比例恢复为生效值，避免看不见的修改
            d.relief_rate = applied.relief_rate.clone();
        }
        if d.relief_enabled {
            indented(ui, |ui| {
                let lid = ui.label(RichText::new("减征比例").color(p.text)).id;
                let r = text_field(ui, lid, &mut d.relief_rate, "%", bg, true);
                mark_pending(ui, &r, pending);
                f.keys(ui, k, &r, can);
                small(ui, "统一适用于全部征收项目；50% 即减半征收");
            });
        }
        let default_text = if def.relief_enabled {
            format!("是，减征 {}%", def.relief_rate)
        } else {
            "否".to_string()
        };
        if f.modified(k) || (pending && !can) {
            f.notes(ui, k, "", Some(&default_text));
        }
        relief_source(ui);
    }

    // ---- 征收项目 ----
    group_title(ui, "征收项目");
    for i in 0..d.items.len() {
        let k = format!("{}.{i}", key::ITEMS);
        let standard = d.standard_item(i, &def).cloned();
        let pending = f.pending(&k);
        let can = f.row_error(&k).is_none();
        let bg = field_bg(ui, !can && pending, pending);
        let applied_rate = applied.items.get(i).map(|x| x.rate.clone());
        let it = &mut d.items[i];
        let was = it.enabled;
        match standard {
            Some(std_item) => {
                let name = it.name.clone();
                ui.horizontal(|ui| {
                    label_cell(ui, &name);
                    let r = control_cell(ui, |ui| {
                        ui.label(RichText::new("是否征收").color(p.text_weak));
                        yes_no(ui, &mut it.enabled, &format!("{name}是否征收"))
                    });
                    mark_pending(ui, &r, pending);
                    f.keys(ui, &k, &r, can);
                    f.actions(ui, &k, true);
                });
                if was && !it.enabled {
                    if let Some(r) = &applied_rate {
                        it.rate = r.clone();
                    }
                    if name == labor_fee_core::URBAN_CONSTRUCTION_TAX {
                        d.urban_region = applied.urban_region;
                        d.urban_rates = applied.urban_rates.clone();
                    }
                }
                let urban = name == labor_fee_core::URBAN_CONSTRUCTION_TAX;
                if it.enabled {
                    if urban {
                        indented(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.visuals_mut().widgets.inactive.bg_stroke =
                                    Stroke::new(1.0, p.text_weak);
                                ui.label(
                                    RichText::new("纳税人所在地（城市维护建设税法第四条）")
                                        .size(theme::FS_SMALL)
                                        .color(p.text_weak),
                                );
                                let mut all: Option<Response> = None;
                                for (idx, region) in
                                    labor_fee_core::UrbanRegion::ALL.into_iter().enumerate()
                                {
                                    ui.horizontal(|ui| {
                                        let r = ui.radio(
                                            d.urban_region == Some(region),
                                            format!("{}：税率", region.label()),
                                        );
                                        if r.clicked() {
                                            d.urban_region = Some(region);
                                        }
                                        // 三档税率输入框左对齐
                                        ui.add_space((URBAN_RADIO_W - r.rect.width()).max(0.0));
                                        let t = ui
                                            .add(
                                                TextEdit::singleline(&mut d.urban_rates[idx])
                                                    .font(theme::mono(14.0))
                                                    .desired_width(56.0)
                                                    .background_color(bg),
                                            )
                                            .labelled_by(r.id);
                                        ui.label(RichText::new("%").color(p.text_weak));
                                        let dv = &def.urban_rates[idx];
                                        if !crate::settings::same_value(&d.urban_rates[idx], dv) {
                                            small(ui, format!("（默认：{}%）", dv.trim()));
                                        }
                                        let u = r.union(t);
                                        all = Some(match all.take() {
                                            Some(a) => a.union(u),
                                            None => u,
                                        });
                                    });
                                }
                                if let Some(region) = d.urban_region {
                                    it.rate = d.urban_rates[region.index()].clone();
                                }
                                if let Some(r) = &all {
                                    mark_pending(ui, r, pending);
                                    f.keys(ui, &k, r, can);
                                }
                                if d.urban_region.is_none() {
                                    ui.label(
                                        RichText::new(format!(
                                            "{} 当前税率 {}% 不属于任一档位，请选择纳税人所在地",
                                            icons::WARN,
                                            it.rate.trim()
                                        ))
                                        .size(theme::FS_SMALL)
                                        .color(p.warning),
                                    );
                                }
                            });
                        });
                    } else {
                        indented(ui, |ui| {
                            let lid = ui.label(RichText::new("征收比例").color(p.text)).id;
                            let r = text_field(ui, lid, &mut it.rate, "%", bg, true);
                            mark_pending(ui, &r, pending);
                            f.keys(ui, &k, &r, can);
                        });
                    }
                }
                let help = match name.as_str() {
                    labor_fee_core::URBAN_CONSTRUCTION_TAX => {
                        "税率按纳税人所在地，默认为法定税率：市区 7%、县城或镇 5%、其他地区 1%（《中华人民共和国城市维护建设税法》第四条）；各档税率可修改"
                    }
                    labor_fee_core::EDUCATION_SURCHARGE => {
                        "征收率 3%（《征收教育费附加的暂行规定》第三条）"
                    }
                    labor_fee_core::LOCAL_EDUCATION_SURCHARGE => {
                        "征收标准 2%（《财政部关于统一地方教育附加政策有关问题的通知》财综〔2010〕98号）"
                    }
                    _ => "",
                };
                let default_text = if std_item.enabled && urban {
                    let r = def
                        .urban_region
                        .unwrap_or(labor_fee_core::DEFAULT_URBAN_REGION);
                    format!(
                        "征收，{} {}%",
                        r.label().trim_start_matches("纳税人所在地"),
                        def.urban_rates[r.index()]
                    )
                } else if std_item.enabled {
                    format!("征收，{}%", std_item.rate)
                } else if urban {
                    format!(
                        "不征收；选“是”时预填“不在市区、县城或者镇”{}%",
                        std_item.rate
                    )
                } else {
                    format!("不征收；选“是”时预填 {}%", std_item.rate)
                };
                f.notes(ui, &k, help, Some(&default_text));
            }
            None => {
                let is_new = i >= applied.items.len();
                ui.horizontal(|ui| {
                    let a = ui
                        .allocate_ui_with_layout(
                            Vec2::new(LABEL_W, 30.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.set_width(LABEL_W);
                                ui.add(
                                    TextEdit::singleline(&mut it.name)
                                        .desired_width(LABEL_W - 12.0)
                                        .background_color(bg)
                                        .hint_text("项目名称"),
                                )
                            },
                        )
                        .inner;
                    let acc = format!("{}是否征收", it.name.trim());
                    let r = control_cell(ui, |ui| {
                        ui.label(RichText::new("是否征收").color(p.text_weak));
                        yes_no(ui, &mut it.enabled, &acc)
                    });
                    let all = a.union(r);
                    mark_pending(ui, &all, pending);
                    f.keys(ui, &k, &all, can);
                    f.actions(ui, &k, false);
                    if !is_new
                        && ui
                            .small_button(format!("{} 删除", icons::ERROR))
                            .on_hover_text("删除该项目（立即生效）")
                            .clicked()
                    {
                        f.acts.push(Act::DeleteItem(i));
                    }
                });
                if was && !it.enabled {
                    if let Some(r) = &applied_rate {
                        it.rate = r.clone();
                    }
                }
                if it.enabled {
                    indented(ui, |ui| {
                        let lid = ui.label(RichText::new("征收比例").color(p.text)).id;
                        let r = text_field(ui, lid, &mut it.rate, "%", bg, true);
                        mark_pending(ui, &r, pending);
                        f.keys(ui, &k, &r, can);
                    });
                }
                let help = if is_new {
                    "新增的项目：填写名称和征收比例后点击 ✔ 生效"
                } else {
                    "自行添加的项目（内置默认无此项）；减征优惠同样适用"
                };
                f.notes(ui, &k, help, None);
            }
        }
    }
    if f.modified(key::ITEMS_REMOVED) || f.pending(key::ITEMS_REMOVED) {
        // 删除或调整了默认项目：单独一行标记，计入“与内置默认不同”的项数
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + 8.0);
            small(ui, "已删除或调整默认项目顺序");
            f.actions(ui, key::ITEMS_REMOVED, true);
        });
        f.notes(
            ui,
            key::ITEMS_REMOVED,
            "",
            Some("城市维护建设税、教育费附加、地方教育附加三项"),
        );
    }
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + 8.0);
        if ui
            .button("＋ 添加其他项目")
            .on_hover_text("新增一行，填写后点击 ✔ 生效")
            .clicked()
        {
            d.items.push(ItemDraft {
                name: "其他附加".into(),
                rate: "0".into(),
                enabled: true,
            });
        }
    });

    // ---- 计税依据 ----
    group_title(ui, "计税依据");
    row(
        ui,
        f,
        key::BASE,
        "计税依据",
        "依法以实际缴纳的增值税税额为计税依据",
        surcharge_base_text(def.surcharge_base),
        |ui, l| {
            ComboBox::from_id_salt("surcharge_base")
                .selected_text(surcharge_base_text(d.surcharge_base))
                .width(CTRL_W)
                .show_ui(ui, |ui| {
                    for b in [SurchargeBase::Vat, SurchargeBase::NetRevenue] {
                        ui.selectable_value(&mut d.surcharge_base, b, surcharge_base_text(b));
                    }
                })
                .response
                .labelled_by(l)
        },
    );
}

fn iit_section(ui: &mut Ui, d: &mut SettingsDraft, f: &mut Form) {
    let def = d.defaults();
    let p = palette(ui);
    group_title(ui, "减除费用（国家税务总局公告2018年第61号第八条）");
    for (k, label, help, unit) in [
        (
            key::FIXED_LIMIT,
            "定额减除适用上限",
            "每次收入不超过该金额时，减除定额费用",
            "元",
        ),
        (key::FIXED_DEDUCTION, "定额减除费用", "", "元"),
        (
            key::PCT_DEDUCTION,
            "定率减除比例",
            "每次收入超过上限时，减除收入的该比例",
            "%",
        ),
    ] {
        let bg = field_bg(ui, f.row_error(k).is_some(), f.pending(k));
        let (value, dv) = match k {
            key::FIXED_LIMIT => (&mut d.fixed_limit, &def.fixed_limit),
            key::FIXED_DEDUCTION => (&mut d.fixed_deduction, &def.fixed_deduction),
            _ => (&mut d.pct_deduction, &def.pct_deduction),
        };
        let default_text = if unit == "%" {
            format!("{dv}%")
        } else {
            format!("{dv} 元")
        };
        row(ui, f, k, label, help, &default_text, |ui, l| {
            text_field(ui, l, value, unit, bg, true)
        });
    }
    group_title(
        ui,
        "预扣率表（个人所得税预扣率表二，居民个人劳务报酬所得适用）",
    );
    let k = key::BRACKETS;
    let pending = f.pending(k);
    ui.horizontal(|ui| {
        small(
            ui,
            "上限含本数；最后一档无上限。整张表作为一项，修改后统一确认。",
        );
        f.actions(ui, k, true);
    });
    let mut all: Option<Response> = None;
    let mut add = |r: Response| {
        all = Some(match all.take() {
            Some(a) => a.union(r),
            None => r,
        });
    };
    let grid = egui::Grid::new("brackets_grid")
        .num_columns(6)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            let heads: Vec<Id> = [
                "档次",
                "预扣预缴应纳税所得额上限",
                "",
                "预扣率",
                "速算扣除数",
                "",
            ]
            .iter()
            .map(|h| {
                ui.label(RichText::new(*h).size(theme::FS_SMALL).color(p.text_weak))
                    .id
            })
            .collect();
            ui.end_row();
            let n = d.brackets.len();
            let mut remove = None;
            for (i, b) in d.brackets.iter_mut().enumerate() {
                let ek = format!("{}.{i}", key::BRACKETS);
                let err = if pending { f.errors.get(&ek) } else { None };
                let bg = field_bg(ui, err.is_some(), pending);
                ui.label(RichText::new(format!("第 {} 档", i + 1)).color(p.text));
                if i + 1 == n {
                    ui.label(RichText::new("无上限").color(p.text_weak));
                } else {
                    add(ui
                        .add(
                            TextEdit::singleline(&mut b.upper)
                                .font(theme::mono(14.0))
                                .desired_width(110.0)
                                .background_color(bg),
                        )
                        .labelled_by(heads[1]));
                }
                ui.label(RichText::new("元").color(p.text_weak));
                ui.horizontal(|ui| {
                    add(ui
                        .add(
                            TextEdit::singleline(&mut b.rate)
                                .font(theme::mono(14.0))
                                .desired_width(60.0)
                                .background_color(bg),
                        )
                        .labelled_by(heads[3]));
                    ui.label(RichText::new("%").color(p.text_weak));
                });
                ui.horizontal(|ui| {
                    add(ui
                        .add(
                            TextEdit::singleline(&mut b.quick)
                                .font(theme::mono(14.0))
                                .desired_width(90.0)
                                .background_color(bg),
                        )
                        .labelled_by(heads[4]));
                    ui.label(RichText::new("元").color(p.text_weak));
                });
                ui.horizontal(|ui| {
                    if n > 1
                        && ui
                            .small_button("删除")
                            .on_hover_text("删除该档（点击 ✔ 后生效）")
                            .clicked()
                    {
                        remove = Some(i);
                    }
                    if let Some(e) = err {
                        ui.label(RichText::new(e).size(theme::FS_SMALL).color(p.error));
                    }
                });
                ui.end_row();
            }
            if let Some(i) = remove {
                d.brackets.remove(i);
            }
        });
    if pending {
        let p = palette(ui);
        ui.painter().rect_stroke(
            grid.response.rect.expand(4.0),
            CornerRadius::same(theme::RADIUS_CTRL),
            Stroke::new(1.5, p.warning),
            egui::StrokeKind::Outside,
        );
    }
    if let Some(r) = all {
        let can = f.row_error(k).is_none();
        f.keys(ui, k, &r, can);
    }
    if pending {
        if let Some(e) = f.errors.get(k) {
            ui.label(RichText::new(e).color(p.error));
        }
    }
    let rates: Vec<String> = def
        .brackets
        .iter()
        .map(|b| format!("{}%", b.rate))
        .collect();
    if f.modified(k) {
        ui.label(
            RichText::new(format!(
                "{} 当前值与默认值不同（默认：预扣率表二，{} 三档）",
                icons::INFO,
                rates.join("、")
            ))
            .size(theme::FS_SMALL)
            .color(p.warning),
        );
    }
    if ui
        .button("＋ 插入一档（在最后一档之前）")
        .on_hover_text("插入后填写本档数值，点击 ✔ 生效")
        .clicked()
    {
        let at = d.brackets.len().saturating_sub(1);
        d.brackets.insert(
            at,
            BracketDraft {
                upper: String::new(),
                rate: String::new(),
                quick: "0".into(),
            },
        );
    }
    group_title(ui, "适用范围");
    ui.horizontal(|ui| {
        ui.label(RichText::new("非居民个人").color(p.text));
        small(ui, "仅适用于居民个人劳务报酬所得预扣预缴（预扣率表二）；非居民个人适用个人所得税税率表三，本工具未提供。");
    });
}

fn combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    l: Id,
    width: f32,
    value: &mut T,
    options: &[T],
    text: fn(T) -> &'static str,
) -> Response {
    ComboBox::from_id_salt(id)
        .selected_text(text(*value))
        .width(width)
        .show_ui(ui, |ui| {
            for &m in options {
                ui.selectable_value(value, m, text(m));
            }
        })
        .response
        .labelled_by(l)
}

fn rounding_section(ui: &mut Ui, d: &mut SettingsDraft, f: &mut Form) {
    let def = d.defaults();
    group_title(ui, "金额舍入");
    let bg = field_bg(ui, f.row_error(key::SCALE).is_some(), f.pending(key::SCALE));
    row(
        ui,
        f,
        key::SCALE,
        "金额小数位",
        "2 表示到分；增值税税额、各项附加税费、20% 减除费用、应预扣预缴税额分别舍入",
        &format!("{} 位", def.scale),
        |ui, l| text_field(ui, l, &mut d.scale, "位", bg, true),
    );
    row(
        ui,
        f,
        key::MODE,
        "舍入方式",
        "请确认与开票、申报系统一致",
        rounding_mode_text(def.mode),
        |ui, l| {
            combo(
                ui,
                "rounding_mode",
                l,
                CTRL_W,
                &mut d.mode,
                &[
                    RoundingMode::HalfUp,
                    RoundingMode::HalfEven,
                    RoundingMode::Down,
                    RoundingMode::Up,
                ],
                rounding_mode_text,
            )
        },
    );
    row(
        ui,
        f,
        key::SURCHARGE_ROUNDING,
        "附加税费舍入方式",
        "征收多个项目时适用；默认各项目分别按分舍入",
        surcharge_rounding_text(def.surcharge_rounding),
        |ui, l| {
            combo(
                ui,
                "surcharge_rounding",
                l,
                CTRL_W,
                &mut d.surcharge_rounding,
                &[SurchargeRounding::PerItem, SurchargeRounding::Total],
                surcharge_rounding_text,
            )
        },
    );
}

fn solving_section(ui: &mut Ui, d: &mut SettingsDraft, f: &mut Form) {
    let def = d.defaults();
    group_title(ui, "按税后实发金额测算");
    small(ui, "适用于“实发测算”及批量测算中的税后实发金额列。");
    ui.add_space(theme::SP1);
    row(
        ui,
        f,
        key::PREFER,
        "多解时取值",
        "多个不含增值税收入得到同一税后实发金额时",
        prefer_text(def.prefer),
        |ui, l| {
            combo(
                ui,
                "prefer",
                l,
                CTRL_W + 40.0,
                &mut d.prefer,
                &[Prefer::MinCost, Prefer::MaxCost],
                prefer_text,
            )
        },
    );
    group_title(ui, "求解参数");
    let bg = field_bg(
        ui,
        f.row_error(key::SEARCH_WINDOW).is_some(),
        f.pending(key::SEARCH_WINDOW),
    );
    row(
        ui,
        f,
        key::SEARCH_WINDOW,
        "逐分校准窗口",
        "二分定位后向前、向后逐分校准的范围",
        &format!("{} 分", def.search_window),
        |ui, l| text_field(ui, l, &mut d.search_window, "分", bg, true),
    );
    let bg = field_bg(
        ui,
        f.row_error(key::MAX_NET).is_some(),
        f.pending(key::MAX_NET),
    );
    row(
        ui,
        f,
        key::MAX_NET,
        "测算搜索上限",
        "不含增值税收入的最大值",
        &format!("{} 元", def.max_net_income),
        |ui, l| text_field(ui, l, &mut d.max_net_income, "元", bg, true),
    );
}

fn appearance_section(ui: &mut Ui, d: &mut SettingsDraft, f: &mut Form) {
    let p = palette(ui);
    let k = key::THEME;
    let pending = f.pending(k);
    group_title(ui, "主题");
    ui.horizontal(|ui| {
        let mut all: Option<Response> = None;
        for t in ThemeChoice::ALL {
            let selected = d.theme == t;
            let b = egui::Button::new(
                RichText::new(format!("{}  {}", t.icon(), t.title())).color(if selected {
                    p.primary
                } else {
                    p.text
                }),
            )
            .fill(if selected { p.primary_soft } else { p.surface })
            .stroke(Stroke::new(
                1.0,
                if selected {
                    if pending {
                        p.warning
                    } else {
                        p.primary
                    }
                } else {
                    p.border
                },
            ))
            .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
            .min_size(Vec2::new(120.0, 40.0));
            let r = ui.add(b);
            if r.clicked() {
                d.theme = t;
            }
            all = Some(match all.take() {
                Some(a) => a.union(r),
                None => r,
            });
        }
        if let Some(r) = all {
            f.keys(ui, k, &r, true);
        }
        f.actions(ui, k, false);
    });
    small(
        ui,
        "选择后即预览，点击 ✔ 生效并保存；“跟随系统”会随操作系统的浅色/深色外观自动切换。默认：跟随系统。",
    );
}

fn storage_section(ui: &mut Ui, store_dir: Option<&str>) {
    let p = palette(ui);
    group_title(ui, "设置保存位置");
    match store_dir {
        Some(dir) => {
            ui.label(RichText::new(dir).font(theme::mono(13.0)).color(p.text));
            small(ui, "外观偏好保存在 preferences.toml，计算参数保存在 rules.toml（与内置默认相同时不保存）。每项设置点击 ✔ 后立即写入，由程序自动维护，无需手工编辑。");
        }
        None => {
            small(ui, "当前环境没有可用的配置目录，设置只在本次运行有效。");
        }
    }
    group_title(ui, "恢复默认");
    small(ui, "窗口左下角的“全部恢复默认”将全部计算参数和外观偏好恢复为内置默认，确认后立即生效，并同步还原上述两个文件。");
}
