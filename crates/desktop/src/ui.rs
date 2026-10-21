//! 界面绘制。布局：顶部为标题和页签栏，下方为当前页签内容。

use crate::state::{AppState, Field, Level, Outcome, SingleTab, Status, Tab};
use eframe::egui::{
    self, Align, Color32, ComboBox, Grid, Key, Layout, RichText, ScrollArea, TextEdit, Ui,
};
use labor_fee_batch::InputKind;
use labor_fee_core::Decimal;

const RED: Color32 = Color32::from_rgb(200, 40, 40);
const GREEN: Color32 = Color32::from_rgb(20, 130, 60);
const ORANGE: Color32 = Color32::from_rgb(200, 110, 0);

fn level_color(level: Level) -> Color32 {
    match level {
        Level::Ok => GREEN,
        Level::Warn => ORANGE,
        Level::Error => RED,
    }
}

fn status_label(ui: &mut Ui, s: &Status) {
    ui.label(RichText::new(&s.text).color(level_color(s.level)));
}

/// 绘制整个程序。
pub fn draw(ui: &mut Ui, st: &mut AppState) {
    st.poll_batch();
    egui::Panel::top("top").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading("劳务报酬计算工具");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("口径：{}", st.config.source))
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });
        ui.horizontal(|ui| {
            for tab in Tab::ALL {
                ui.selectable_value(&mut st.tab, tab, RichText::new(tab.title()).size(16.0));
            }
        });
        ui.add_space(2.0);
    });
    egui::CentralPanel::default_margins().show(ui, |ui| match st.tab {
        Tab::Forward => forward_tab(ui, st),
        Tab::Reverse => reverse_tab(ui, st),
        Tab::FromCost => from_cost_tab(ui, st),
        Tab::Batch => batch_tab(ui, st),
        Tab::Config => config_tab(ui, st),
    });
}

/// 输入行：标签 + 输入框 + “计算”按钮；回车或点击按钮返回 true。
fn input_row(ui: &mut Ui, label: &str, hint: &str, tab: &mut SingleTab) -> bool {
    ui.horizontal(|ui| {
        let l = ui.label(label);
        let r = ui
            .add(
                TextEdit::singleline(&mut tab.input)
                    .desired_width(200.0)
                    .hint_text(hint),
            )
            .labelled_by(l.id);
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        ui.button("计算").clicked() || enter
    })
    .inner
}

fn fields_table(ui: &mut Ui, id: &str, fields: &[Field], status: &Status) {
    Grid::new(id)
        .striped(true)
        .num_columns(2)
        .min_col_width(140.0)
        .spacing([24.0, 6.0])
        .show(ui, |ui| {
            ui.label(RichText::new("项目").strong());
            ui.label(RichText::new("金额（元）").strong());
            ui.end_row();
            for f in fields {
                ui.label(&f.label);
                // 金额列固定宽度、右对齐，便于对位
                ui.allocate_ui_with_layout(
                    egui::vec2(160.0, ui.spacing().interact_size.y),
                    Layout::right_to_left(Align::Center),
                    |ui| ui.label(RichText::new(&f.value).monospace()),
                );
                ui.end_row();
            }
            ui.label("校验状态");
            ui.scope(|ui| {
                // 说明较长时在窗口内换行
                ui.set_max_width((ui.available_width()).clamp(240.0, 640.0));
                ui.add(
                    egui::Label::new(RichText::new(&status.text).color(level_color(status.level)))
                        .wrap(),
                );
            });
            ui.end_row();
        });
}

/// 显示单笔结果；空位时返回被点击的有效总成本。
fn outcome_view(ui: &mut Ui, id: &str, outcome: &Outcome) -> Option<Decimal> {
    ui.add_space(8.0);
    match outcome {
        Outcome::None => {
            ui.label(
                RichText::new("输入金额后点击“计算”或按回车。")
                    .color(ui.visuals().weak_text_color()),
            );
            None
        }
        Outcome::Error(e) => {
            ui.label(RichText::new(e).color(RED));
            None
        }
        Outcome::Done { fields, status } => {
            ScrollArea::vertical().show(ui, |ui| fields_table(ui, id, fields, status));
            None
        }
        Outcome::Gap(g) => {
            let mut clicked = None;
            ui.label(
                RichText::new(crate::state::gap_reason_text(g))
                    .color(RED)
                    .strong(),
            );
            ui.label(RichText::new(g.to_string()).color(RED));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("两侧最近的有效总成本（点击重算）：");
                for side in [g.lower, g.upper] {
                    if ui
                        .button(format!(
                            "{}（不含税收入 {}）",
                            side.total_cost, side.net_income
                        ))
                        .clicked()
                    {
                        clicked = Some(side.total_cost);
                    }
                }
            });
            clicked
        }
    }
}

fn forward_tab(ui: &mut Ui, st: &mut AppState) {
    ui.label("输入不含税收入，计算增值税、附加税、企业总成本、个税和实际到手。");
    if input_row(ui, "不含税收入（元）", "例如 10000", &mut st.forward) {
        st.calc_forward();
    }
    outcome_view(ui, "forward_table", &st.forward.outcome);
}

fn reverse_tab(ui: &mut Ui, st: &mut AppState) {
    ui.label("输入实际到手金额，反算不含税收入及全部金额（多解时取最小不含税收入）。");
    if input_row(ui, "实际到手（元）", "例如 8400", &mut st.reverse) {
        st.calc_reverse();
    }
    outcome_view(ui, "reverse_table", &st.reverse.outcome);
}

fn from_cost_tab(ui: &mut Ui, st: &mut AppState) {
    ui.label("输入企业总成本，求对应的不含税收入；落在空白区间或舍入空位时显示原因。");
    if input_row(ui, "企业总成本（元）", "例如 10103", &mut st.from_cost) {
        st.calc_from_cost();
    }
    if let Some(cost) = outcome_view(ui, "from_cost_table", &st.from_cost.outcome) {
        st.use_cost(cost);
    }
}

fn kind_text(k: InputKind) -> &'static str {
    match k {
        InputKind::TakeHome => "到手金额（反算）",
        InputKind::NetIncome => "不含税收入（正算）",
        InputKind::TotalCost => "企业总成本（按总成本）",
    }
}

fn batch_tab(ui: &mut Ui, st: &mut AppState) {
    ui.label("读取 xlsx/xls/ods/csv 首行标题对应的列，逐行计算并在右侧追加结果列。");
    ui.horizontal(|ui| {
        let l = ui.label("文件");
        ui.add(
            TextEdit::singleline(&mut st.batch.path)
                .desired_width(460.0)
                .hint_text("选择或粘贴 xlsx / csv 文件路径"),
        )
        .labelled_by(l.id);
        if ui.button("选择文件…").clicked() {
            if let Some(p) = rfd::FileDialog::new()
                .add_filter("表格文件", &["xlsx", "xlsm", "xls", "ods", "csv"])
                .pick_file()
            {
                st.batch.path = p.display().to_string();
            }
        }
    });
    ui.horizontal(|ui| {
        let kind_label = ui.label("输入列");
        ComboBox::from_id_salt("batch_kind")
            .selected_text(kind_text(st.batch.kind))
            .width(200.0)
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
        let l = ui.label("列标题");
        ui.add(
            TextEdit::singleline(&mut st.batch.column)
                .desired_width(140.0)
                .hint_text(st.batch.kind.default_column()),
        )
        .labelled_by(l.id);
    });
    ui.horizontal(|ui| {
        let running = st.batch_running();
        if ui
            .add_enabled(!running, egui::Button::new("执行"))
            .clicked()
        {
            let ctx = ui.ctx().clone();
            st.start_batch(move || ctx.request_repaint());
        }
        if running {
            ui.spinner();
            ui.label("计算中…");
        }
    });
    if let Some(m) = &st.batch.message {
        status_label(ui, m);
    }
    let mut save_as = false;
    let mut save_default = false;
    if let Some(res) = &st.batch.result {
        ui.horizontal(|ui| {
            save_as = ui.button("另存为…").clicked();
            save_default = ui
                .button(format!("保存到输入文件旁（{}）", res.default_file_name()))
                .clicked();
        });
        ui.label(format!(
            "预览（前 {} 行，完整结果请保存后查看）：",
            (res.output.preview.len().saturating_sub(1))
        ));
        preview_table(ui, &res.output.preview);
    }
    if save_default {
        if let Some(p) = st.batch.result.as_ref().map(|r| r.default_save_path()) {
            st.save_batch(&p);
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
        if let Some(p) = dlg.save_file() {
            st.save_batch(&p);
        }
    }
}

fn preview_table(ui: &mut Ui, rows: &[Vec<String>]) {
    ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Grid::new("batch_preview")
                .striped(true)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for (i, row) in rows.iter().enumerate() {
                        for cell in row {
                            if i == 0 {
                                ui.label(RichText::new(cell).strong());
                            } else if cell.starts_with("错误") || cell.contains("不可达") {
                                ui.label(RichText::new(cell).color(RED));
                            } else {
                                ui.label(cell);
                            }
                        }
                        ui.end_row();
                    }
                });
        });
}

fn config_tab(ui: &mut Ui, st: &mut AppState) {
    ui.horizontal(|ui| {
        ui.label("当前口径：");
        ui.label(RichText::new(&st.config.source).strong());
    });
    let mut load_path: Option<std::path::PathBuf> = None;
    ui.horizontal(|ui| {
        if ui.button("加载 TOML 文件…").clicked() {
            load_path = rfd::FileDialog::new()
                .add_filter("TOML 配置", &["toml"])
                .pick_file();
        }
        let l = ui.label("或输入路径");
        ui.add(
            TextEdit::singleline(&mut st.config.path)
                .desired_width(320.0)
                .hint_text("自定义口径 .toml 路径"),
        )
        .labelled_by(l.id);
        if ui.button("加载").clicked() && !st.config.path.trim().is_empty() {
            load_path = Some(st.config.path.trim().into());
        }
        if ui.button("恢复内置默认").clicked() {
            st.reset_config();
        }
    });
    if let Some(p) = load_path {
        st.load_config_file(&p);
    }
    if let Some(m) = &st.config.message {
        status_label(ui, m);
    }
    ui.label(
        RichText::new("只读显示当前生效的配置；如需修改，请编辑 TOML 文件后加载。各项含义见 docs/ASSUMPTIONS.md。")
            .color(ui.visuals().weak_text_color()),
    );
    ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut text = st.config.toml.as_str();
            ui.add(
                TextEdit::multiline(&mut text)
                    .code_editor()
                    .desired_width(f32::INFINITY),
            );
        });
}
