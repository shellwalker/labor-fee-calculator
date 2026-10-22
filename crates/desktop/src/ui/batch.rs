//! 批量页：步骤化（1 选择文件 → 2 选择列 → 3 执行 → 4 导出），统计卡片 + 结果预览表。

use super::widgets::{
    badge, card, card_frame, primary_button, secondary_button, small, status_banner,
};
use crate::icons;
use crate::state::AppState;
use crate::theme::{self, palette, Palette};
use eframe::egui::{
    self, Align, Color32, ComboBox, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke,
    TextEdit, Ui, Vec2,
};
use egui_extras::{Column, TableBuilder};
use labor_fee_batch::InputKind;
use labor_fee_core::Decimal;
use std::str::FromStr;

pub fn kind_text(k: InputKind) -> &'static str {
    match k {
        InputKind::TakeHome => "到手金额（反算）",
        InputKind::NetIncome => "不含税收入（正算）",
        InputKind::TotalCost => "企业总成本（按总成本）",
    }
}

/// 弹出文件选择框。
pub fn pick_batch_file() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("表格文件", &["xlsx", "xlsm", "xls", "ods", "csv"])
        .pick_file()
}

fn step_header(ui: &mut Ui, n: usize, title: &str, done: bool, active: bool) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
        let (fill, fg) = if done {
            (p.success, p.on_primary)
        } else if active {
            (p.primary, p.on_primary)
        } else {
            (p.surface_alt, p.text_weak)
        };
        ui.painter().circle_filled(r.center(), 12.0, fill);
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            if done {
                icons::OK.to_string()
            } else {
                n.to_string()
            },
            theme::prop(13.0),
            fg,
        );
        ui.label(
            RichText::new(title)
                .size(theme::FS_SECTION)
                .strong()
                .color(p.text),
        );
    });
    ui.add_space(theme::SP1);
}

fn step_card(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui)) {
    let p = palette(ui);
    card_frame(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(height);
        add(ui);
    });
}

pub fn page(ui: &mut Ui, st: &mut AppState) {
    let p = palette(ui);
    let has_file = !st.batch.path.trim().is_empty();
    let has_result = st.batch.result.is_some();
    let running = st.batch_running();
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    let step_h = 150.0;
    let mut pick = false;
    let mut run = false;
    let mut save_as = false;
    let mut save_default = false;

    let mut steps = |ui: &mut Ui, i: usize, st: &mut AppState| match i {
        0 => step_card(ui, step_h, |ui| {
            step_header(ui, 1, "选择文件", has_file, !has_file);
            drop_zone(ui, p, hovering, &mut pick, &st.batch.path);
            let l = ui.label(
                RichText::new("文件路径")
                    .size(theme::FS_SMALL)
                    .color(p.text_weak),
            );
            ui.add(
                TextEdit::singleline(&mut st.batch.path)
                    .desired_width(ui.available_width())
                    .hint_text("也可粘贴 xlsx / csv 文件路径"),
            )
            .labelled_by(l.id);
        }),
        1 => step_card(ui, step_h, |ui| {
            step_header(
                ui,
                2,
                "选择输入列",
                has_file && has_result,
                has_file && !has_result,
            );
            let kind_label = ui.label(
                RichText::new("输入列")
                    .size(theme::FS_SMALL)
                    .color(p.text_weak),
            );
            ComboBox::from_id_salt("batch_kind")
                .selected_text(kind_text(st.batch.kind))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for k in [
                        InputKind::TakeHome,
                        InputKind::NetIncome,
                        InputKind::TotalCost,
                    ] {
                        ui.selectable_value(&mut st.batch.kind, k, kind_text(k));
                    }
                })
                .response
                .labelled_by(kind_label.id);
            let l = ui.label(
                RichText::new("列标题")
                    .size(theme::FS_SMALL)
                    .color(p.text_weak),
            );
            ui.add(
                TextEdit::singleline(&mut st.batch.column)
                    .desired_width(ui.available_width())
                    .hint_text(st.batch.kind.default_column()),
            )
            .labelled_by(l.id);
        }),
        2 => step_card(ui, step_h, |ui| {
            step_header(ui, 3, "执行计算", has_result, has_file && !has_result);
            small(ui, "按首行标题找到所选列，逐行计算，结果列追加在右侧。");
            ui.add_space(theme::SP2);
            ui.horizontal(|ui| {
                ui.scope(|ui| {
                    ui.spacing_mut().button_padding.x = 22.0;
                    if primary_button(ui, &format!("{} 执行", icons::RUN), !running).clicked() {
                        run = true;
                    }
                });
                if running {
                    ui.spinner();
                    ui.label("计算中…");
                }
            });
        }),
        _ => step_card(ui, step_h, |ui| {
            step_header(ui, 4, "导出结果", false, has_result);
            match &st.batch.result {
                None => {
                    small(ui, "执行后可导出为新文件（原文件不会被修改）。");
                }
                Some(res) => {
                    let name = res.default_file_name();
                    save_default =
                        primary_button(ui, &format!("{} 保存到输入文件旁", icons::SAVE), true)
                            .on_hover_text(res.default_save_path().display().to_string())
                            .clicked();
                    small(ui, name);
                    save_as =
                        secondary_button(ui, &format!("{} 另存为…", icons::DOWNLOAD)).clicked();
                }
            }
        }),
    };
    if ui.available_width() >= 880.0 {
        ui.columns(4, |c| {
            for (i, ui) in c.iter_mut().enumerate() {
                steps(ui, i, st);
            }
        });
    } else {
        ui.columns(2, |c| {
            steps(&mut c[0], 0, st);
            steps(&mut c[1], 1, st);
        });
        ui.columns(2, |c| {
            steps(&mut c[0], 2, st);
            steps(&mut c[1], 3, st);
        });
    }
    if let Some(m) = &st.batch.message {
        ui.add_space(theme::SP1);
        status_banner(ui, m.level, &m.text);
    }
    ui.add_space(theme::SP1);
    stats_row(ui, st);
    ui.add_space(theme::SP1);
    preview_card(ui, st);

    if pick {
        if let Some(pth) = pick_batch_file() {
            st.open_batch_file(&pth);
        }
    }
    if run {
        let ctx = ui.ctx().clone();
        st.start_batch(move || ctx.request_repaint());
    }
    if save_default {
        if let Some(pth) = st.batch.result.as_ref().map(|r| r.default_save_path()) {
            st.save_batch(&pth);
        }
    }
    if save_as {
        let Some(res) = &st.batch.result else { return };
        let ext = res.output.format.output_extension();
        let mut dlg = rfd::FileDialog::new()
            .set_file_name(res.default_file_name())
            .add_filter(ext, &[ext]);
        if let Some(dir) = res.input.parent() {
            dlg = dlg.set_directory(dir);
        }
        if let Some(pth) = dlg.save_file() {
            st.save_batch(&pth);
        }
    }
}

fn drop_zone(ui: &mut Ui, p: &Palette, hovering: bool, pick: &mut bool, path: &str) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 58.0), Sense::click());
    let fill = if hovering || resp.hovered() {
        p.primary_soft
    } else {
        p.surface_alt
    };
    let stroke_c = if hovering { p.primary } else { p.border };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS_CTRL), fill);
    // 虚线边框
    let dash = |a: egui::Pos2, b: egui::Pos2| {
        ui.painter().add(egui::Shape::dashed_line(
            &[a, b],
            Stroke::new(1.2, stroke_c),
            5.0,
            4.0,
        ));
    };
    let r = rect.shrink(0.5);
    dash(r.left_top(), r.right_top());
    dash(r.right_top(), r.right_bottom());
    dash(r.right_bottom(), r.left_bottom());
    dash(r.left_bottom(), r.left_top());
    let name = std::path::Path::new(path.trim())
        .file_name()
        .map(|n| n.to_string_lossy().to_string());
    let (line1, line2) = match (&name, hovering) {
        (_, true) => ("松开鼠标即可导入".to_string(), String::new()),
        (Some(n), _) => (
            format!("{} {n}", icons::FILE),
            "点击更换，或拖放新文件".into(),
        ),
        (None, _) => (
            format!("{} 拖放表格文件到这里", icons::FOLDER),
            "或点击选择 xlsx / xls / ods / csv".into(),
        ),
    };
    ui.painter().text(
        rect.center() - egui::vec2(0.0, 9.0),
        egui::Align2::CENTER_CENTER,
        line1,
        theme::prop(13.5),
        if name.is_some() { p.text } else { p.primary },
    );
    ui.painter().text(
        rect.center() + egui::vec2(0.0, 11.0),
        egui::Align2::CENTER_CENTER,
        line2,
        theme::prop(11.5),
        p.text_weak,
    );
    // 无障碍：拖放区作为“选择文件…”按钮
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "选择文件…"));
    if resp.clicked() {
        *pick = true;
    }
}

fn stat_card(ui: &mut Ui, label: &str, value: Option<usize>, color: Color32, bg: Color32) {
    let p = palette(ui);
    egui::Frame::new()
        .fill(if value.is_some() { bg } else { p.surface })
        .stroke(Stroke::new(
            1.0,
            if value.is_some() {
                color.gamma_multiply(0.35)
            } else {
                p.border
            },
        ))
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(label)
                    .size(theme::FS_SMALL)
                    .color(p.text_weak),
            );
            ui.label(
                RichText::new(value.map_or("—".to_string(), |v| v.to_string()))
                    .font(theme::mono(24.0))
                    .color(if value.is_some() { color } else { p.text_weak }),
            );
        });
}

fn stats_row(ui: &mut Ui, st: &AppState) {
    let p = palette(ui);
    let s = st
        .batch
        .result
        .as_ref()
        .map(|r| (&r.output.summary, r.kind));
    let third_label = match st.batch.result.as_ref().map_or(st.batch.kind, |r| r.kind) {
        InputKind::TakeHome => "不可精确达到",
        InputKind::TotalCost => "总成本不可达",
        InputKind::NetIncome => "需注意",
    };
    let third = s.map(|(s, k)| match k {
        InputKind::TakeHome => s.inexact,
        InputKind::TotalCost => s.unreachable_cost,
        InputKind::NetIncome => 0,
    });
    ui.columns(5, |c| {
        stat_card(
            &mut c[0],
            "总行数",
            s.map(|x| x.0.rows),
            p.primary,
            p.primary_soft,
        );
        stat_card(
            &mut c[1],
            "成功",
            s.map(|x| x.0.exact),
            p.success,
            p.success_soft,
        );
        stat_card(&mut c[2], third_label, third, p.warning, p.warning_soft);
        stat_card(
            &mut c[3],
            "错误",
            s.map(|x| x.0.errors),
            p.error,
            p.error_soft,
        );
        stat_card(
            &mut c[4],
            "空白",
            s.map(|x| x.0.empty),
            p.text_weak,
            p.surface_alt,
        );
    });
}

fn is_number(s: &str) -> bool {
    let t = s.trim().trim_end_matches('%');
    !t.is_empty() && Decimal::from_str(t).is_ok()
}

fn text_width(s: &str) -> f32 {
    s.chars()
        .map(|c| if c.is_ascii() { 7.6 } else { 13.5 })
        .sum()
}

fn preview_card(ui: &mut Ui, st: &AppState) {
    let p = palette(ui);
    card(ui, |ui| {
        let rows = st.batch.result.as_ref().map(|r| &r.output.preview);
        super::widgets::card_title(ui, icons::BATCH, "结果预览", |ui| {
            if let Some(rows) = rows {
                badge(
                    ui,
                    format!(
                        "前 {} 行 · 完整结果请导出后查看",
                        rows.len().saturating_sub(1)
                    ),
                    p.text_weak,
                    p.surface_alt,
                );
            }
        });
        match rows {
            None => empty_preview(ui, p),
            Some(rows) if rows.is_empty() => empty_preview(ui, p),
            Some(rows) => preview_table(ui, p, rows),
        }
    });
}

fn empty_preview(ui: &mut Ui, p: &Palette) {
    let h = (ui.available_height() - 24.0).max(140.0);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), h),
        Layout::top_down(Align::Center),
        |ui| {
            ui.add_space((h / 2.0 - 70.0).max(8.0));
            ui.label(RichText::new(icons::BATCH).size(40.0).color(p.primary.gamma_multiply(0.6)));
            ui.label(RichText::new("还没有结果").size(theme::FS_SECTION).color(p.text));
            ui.label(
                RichText::new("表格首行为标题行，默认读取“到手金额”列；结果列（不含税收入、增值税、附加税、企业总成本、个税、实际到手、校准状态等）追加在原有列右侧。")
                    .color(p.text_weak),
            );
            ui.label(RichText::new("非法金额（如“12元5角”）不会中断计算，对应行标记为错误并说明原因。").color(p.text_weak));
        },
    );
}

fn preview_table(ui: &mut Ui, p: &Palette, rows: &[Vec<String>]) {
    let header = &rows[0];
    let ncol = header
        .iter()
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let status_col = header
        .iter()
        .position(|h| h == "校准状态" || h == "校验状态");
    let widths: Vec<f32> = (0..ncol)
        .map(|c| {
            rows.iter()
                .map(|r| {
                    r.get(c).map_or(0.0, |s| {
                        text_width(&if is_number(s) {
                            super::widgets::group_digits(s)
                        } else {
                            s.clone()
                        })
                    })
                })
                .fold(0.0, f32::max)
                .clamp(36.0, 300.0)
                + 18.0
        })
        .collect();
    let h = (ui.available_height() - 4.0).max(160.0);
    ScrollArea::horizontal()
        .id_salt("preview_h")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let mut tb = TableBuilder::new(ui)
                .id_salt("batch_preview")
                .striped(true)
                .resizable(true)
                .auto_shrink([false, true])
                .min_scrolled_height(h - 34.0)
                .max_scroll_height(h - 34.0)
                .cell_layout(Layout::left_to_right(Align::Center));
            for w in &widths {
                tb = tb.column(Column::initial(*w).at_least(36.0).clip(true));
            }
            tb.header(30.0, |mut hr| {
                for (c, h) in header.iter().enumerate() {
                    hr.col(|ui| {
                        ui.painter().rect_filled(
                            ui.max_rect().expand2(egui::vec2(4.0, 0.0)),
                            0.0,
                            p.surface_alt,
                        );
                        let num_col = rows
                            .iter()
                            .skip(1)
                            .any(|r| r.get(c).is_some_and(|s| is_number(s)));
                        let text = RichText::new(h).strong().color(p.text);
                        if num_col {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(text)
                            });
                        } else {
                            ui.label(text);
                        }
                    });
                }
            })
            .body(|mut body| {
                for r in rows.iter().skip(1) {
                    let status = status_col.and_then(|c| r.get(c)).map(String::as_str);
                    let bad = status == Some("错误") || r.iter().any(|c| c.starts_with("错误"));
                    let warn = !bad && status.is_some_and(|s| s != "精确" && !s.is_empty());
                    body.row(26.0, |mut row| {
                        for c in 0..ncol {
                            let cell = r.get(c).map(String::as_str).unwrap_or("");
                            row.col(|ui| {
                                if bad || warn {
                                    let tint = if bad { p.error_soft } else { p.warning_soft };
                                    ui.painter().rect_filled(
                                        ui.max_rect().expand2(egui::vec2(4.0, 1.0)),
                                        0.0,
                                        tint,
                                    );
                                }
                                if Some(c) == status_col && !cell.is_empty() {
                                    let (fg, bg) = match cell {
                                        "精确" => (p.success, p.success_soft),
                                        "错误" => (p.error, p.surface),
                                        _ => (p.warning, p.surface),
                                    };
                                    badge(ui, cell, fg, bg);
                                } else if is_number(cell) {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new(super::widgets::group_digits(cell))
                                                .font(theme::mono(13.0))
                                                .color(p.text),
                                        );
                                    });
                                } else {
                                    let color = if cell.starts_with("错误")
                                        || cell.contains("无法")
                                        || cell.contains("不可达")
                                    {
                                        p.error
                                    } else {
                                        p.text
                                    };
                                    ui.label(RichText::new(cell).color(color));
                                }
                            });
                        }
                    });
                }
            });
        });
}
