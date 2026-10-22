//! 设置窗口：逐项编辑口径与外观，带校验、单项/整体恢复默认、保存/取消。

use super::widgets::{badge, primary_button, secondary_button, small};
use crate::icons;
use crate::settings::{
    key, prefer_text, rounding_mode_text, same_value, surcharge_base_text, surcharge_rounding_text,
    BracketDraft, Errors, ItemDraft, Section, SettingsDraft, Tag, ThemeChoice,
};
use crate::state::AppState;
use crate::theme::{self, palette};
use eframe::egui::{
    self, Align, ComboBox, CornerRadius, Id, Layout, Modal, RichText, ScrollArea, Stroke, TextEdit,
    Ui, Vec2,
};
use labor_fee_core::{Prefer, RoundingMode, SurchargeBase, SurchargeRounding};

pub fn tag_badge(ui: &mut Ui, t: Tag) {
    let p = palette(ui);
    let (fg, bg) = match t {
        Tag::Confirmed => (p.success, p.success_soft),
        Tag::Verified => (p.primary, p.primary_soft),
        Tag::Pending => (p.warning, p.warning_soft),
        Tag::Implementation => (p.text_weak, p.surface_alt),
        Tag::Modified => (p.error, p.error_soft),
    };
    badge(ui, t.text(), fg, bg).on_hover_text(t.hint());
}

const LABEL_W: f32 = 168.0;
const CTRL_W: f32 = 200.0;

/// 一行设置：标签 | 控件 | 状态标签 | 单项恢复默认。返回是否点了恢复默认。
fn row(
    ui: &mut Ui,
    label: &str,
    help: &str,
    tag: Tag,
    modified: bool,
    error: Option<&String>,
    control: impl FnOnce(&mut Ui, Id),
) -> bool {
    let p = palette(ui);
    let mut reset = false;
    ui.horizontal(|ui| {
        let label_id = ui.allocate_ui_with_layout(
            Vec2::new(LABEL_W, 30.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(LABEL_W);
                ui.label(RichText::new(label).color(p.text)).id
            },
        );
        let label_id = label_id.inner;
        ui.allocate_ui_with_layout(
            Vec2::new(CTRL_W + 60.0, 30.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(CTRL_W + 60.0);
                control(ui, label_id);
            },
        );
        tag_badge(ui, if modified { Tag::Modified } else { tag });
        if modified {
            ui.add_space(2.0);
            reset = ui
                .small_button(format!("{} 默认", icons::RESET))
                .on_hover_text("恢复本项的内置默认值")
                .clicked();
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + 8.0);
        ui.vertical(|ui| {
            if let Some(e) = error {
                ui.label(
                    RichText::new(format!("{} {e}", icons::ERROR))
                        .size(theme::FS_SMALL)
                        .color(p.error),
                );
            }
            if !help.is_empty() {
                small(ui, help);
            }
        });
    });
    ui.add_space(2.0);
    reset
}

fn text_field(ui: &mut Ui, label: Id, value: &mut String, unit: &str, error: bool, enabled: bool) {
    let p = palette(ui);
    ui.add_enabled(
        enabled,
        TextEdit::singleline(value)
            .font(theme::mono(14.0))
            .desired_width(CTRL_W - 30.0)
            .background_color(if error { p.error_soft } else { p.surface }),
    )
    .labelled_by(label);
    ui.label(RichText::new(unit).color(p.text_weak));
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

/// 绘制设置窗口（打开时）。
pub fn window(ctx: &egui::Context, st: &mut AppState) {
    if st.settings.is_none() {
        return;
    }
    let store_dir = st.store.as_ref().map(|s| s.dir.display().to_string());
    let mut save = false;
    let mut cancel = false;
    let resp = Modal::new(Id::unique("settings_modal"))
        .frame(super::modal_frame(ctx))
        .show(ctx, |ui| {
            let p = palette(ui);
            let Some(d) = st.settings.as_mut() else {
                return;
            };
            let size = ctx.content_rect().size();
            let w = (size.x - 80.0).clamp(640.0, 900.0);
            let h = (size.y - 220.0).clamp(300.0, 560.0);
            ui.set_width(w);
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
                small(ui, "修改后点击“保存”生效；“取消”放弃本次修改。");
            });
            ui.add_space(theme::SP2);
            let errors = d.build().err().unwrap_or_default();
            ui.horizontal_top(|ui| {
                // 左侧分组导航
                ui.allocate_ui_with_layout(
                    Vec2::new(170.0, h),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(170.0);
                        for s in Section::ALL {
                            let n = section_errors(&errors, s);
                            let text = if n > 0 {
                                format!("{}  ({n})", s.title())
                            } else {
                                s.title().to_string()
                            };
                            let selected = d.section == s;
                            let b = egui::Button::new(RichText::new(text).color(if selected {
                                p.primary
                            } else if n > 0 {
                                p.error
                            } else {
                                p.text
                            }))
                            .fill(if selected { p.primary_soft } else { p.surface })
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
                            .min_size(Vec2::new(170.0, 34.0));
                            if ui.add(b).clicked() {
                                d.section = s;
                            }
                        }
                        ui.add_space(theme::SP3);
                        small(ui, "状态说明：");
                        for t in [
                            Tag::Confirmed,
                            Tag::Verified,
                            Tag::Pending,
                            Tag::Implementation,
                            Tag::Modified,
                        ] {
                            ui.horizontal(|ui| {
                                tag_badge(ui, t);
                            });
                        }
                    },
                );
                let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, h), egui::Sense::hover());
                ui.painter().rect_filled(sep, 0.0, p.border);
                ui.allocate_ui_with_layout(
                    Vec2::new(w - 190.0, h),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_max_height(h);
                        ScrollArea::vertical()
                            .id_salt("settings_form")
                            .max_height(h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| match d.section {
                                Section::Vat => vat_section(ui, d, &errors),
                                Section::Iit => iit_section(ui, d, &errors),
                                Section::Rounding => rounding_section(ui, d, &errors),
                                Section::Appearance => {
                                    appearance_section(ui, d, store_dir.as_deref())
                                }
                            });
                    },
                );
            });
            ui.add_space(theme::SP2);
            if let Some(e) = errors.get(key::GENERAL) {
                super::widgets::status_banner(ui, crate::state::Level::Error, e);
            } else if let Some(m) = &d.message {
                super::widgets::status_banner(ui, crate::state::Level::Error, m);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if secondary_button(ui, &format!("{} 全部恢复默认", icons::RESET))
                    .on_hover_text("所有口径恢复为内置默认，主题恢复为跟随系统（保存后生效）")
                    .clicked()
                {
                    d.reset_all();
                }
                let n = d.modified_count();
                if n > 0 {
                    badge(
                        ui,
                        format!("与内置默认不同 {n} 项"),
                        p.warning,
                        p.warning_soft,
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.scope(|ui| {
                        ui.spacing_mut().button_padding.x = 20.0;
                        if primary_button(ui, "保存", errors.is_empty()).clicked() {
                            save = true;
                        }
                    });
                    if secondary_button(ui, "取消").clicked() {
                        cancel = true;
                    }
                });
            });
        });
    if resp.should_close() {
        cancel = true;
    }
    if save {
        st.save_settings();
    } else if cancel {
        st.cancel_settings();
    }
}

fn section_errors(errors: &Errors, s: Section) -> usize {
    errors
        .keys()
        .filter(|k| match s {
            Section::Vat => k.starts_with("vat.") || k.starts_with("surcharge."),
            Section::Iit => k.starts_with("iit."),
            Section::Rounding => k.starts_with("rounding.") || k.starts_with("reverse."),
            Section::Appearance => false,
        })
        .count()
}

fn vat_section(ui: &mut Ui, d: &mut SettingsDraft, errors: &Errors) {
    let def = d.defaults();
    group_title(ui, "增值税");
    if row(
        ui,
        "增值税征收率",
        "增值税 = round(不含税收入 × 征收率)",
        Tag::Confirmed,
        !same_value(&d.vat_rate, &def.vat_rate),
        errors.get(key::VAT_RATE),
        |ui, l| {
            text_field(
                ui,
                l,
                &mut d.vat_rate,
                "%",
                errors.contains_key(key::VAT_RATE),
                true,
            )
        },
    ) {
        d.vat_rate = def.vat_rate.clone();
    }
    if row(
        ui,
        "启用征收门槛",
        "业务口径：低于门槛不征增值税和附加税（不是法定起征点；个税不受影响）",
        Tag::Confirmed,
        d.threshold_enabled != def.threshold_enabled,
        None,
        |ui, _| {
            ui.checkbox(&mut d.threshold_enabled, "启用");
        },
    ) {
        d.threshold_enabled = def.threshold_enabled;
    }
    let en = d.threshold_enabled;
    if row(
        ui,
        "征收门槛（不含税收入）",
        "按不含税收入判断，不用总成本",
        Tag::Confirmed,
        !same_value(&d.threshold, &def.threshold),
        errors.get(key::THRESHOLD),
        |ui, l| {
            text_field(
                ui,
                l,
                &mut d.threshold,
                "元",
                errors.contains_key(key::THRESHOLD),
                en,
            )
        },
    ) {
        d.threshold = def.threshold.clone();
    }
    if row(
        ui,
        "恰好等于门槛时",
        "勾选：≥ 门槛征收（含本数）；不勾选：> 门槛才征收",
        Tag::Confirmed,
        d.levy_at_threshold != def.levy_at_threshold,
        None,
        |ui, _| {
            ui.add_enabled(en, egui::Checkbox::new(&mut d.levy_at_threshold, "征收"));
        },
    ) {
        d.levy_at_threshold = def.levy_at_threshold;
    }
    group_title(ui, "附加税");
    if row(
        ui,
        "计税依据",
        "",
        Tag::Confirmed,
        d.surcharge_base != def.surcharge_base,
        None,
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
                .labelled_by(l);
        },
    ) {
        d.surcharge_base = def.surcharge_base;
    }
    if row(
        ui,
        "减半征收",
        "对所有附加税目减半",
        Tag::Confirmed,
        d.halve != def.halve,
        None,
        |ui, _| {
            ui.checkbox(&mut d.halve, "减半");
        },
    ) {
        d.halve = def.halve;
    }
    // 税目列表
    let p = palette(ui);
    let items_label = ui
        .horizontal(|ui| {
            let id = ui
                .allocate_ui_with_layout(
                    Vec2::new(LABEL_W, 26.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_width(LABEL_W);
                        ui.label(RichText::new("附加税税目").color(p.text)).id
                    },
                )
                .inner;
            small(
                ui,
                "默认一个合计税目 3%；也可拆成城建税、教育费附加、地方教育附加",
            );
            id
        })
        .inner;
    let mut remove = None;
    let n_items = d.items.len();
    for (i, it) in d.items.iter_mut().enumerate() {
        let k = format!("{}.{i}", key::ITEMS);
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + 8.0);
            ui.add(
                TextEdit::singleline(&mut it.name)
                    .desired_width(140.0)
                    .hint_text("税目名称"),
            )
            .labelled_by(items_label);
            ui.add(
                TextEdit::singleline(&mut it.rate)
                    .font(theme::mono(14.0))
                    .desired_width(70.0)
                    .background_color(if errors.contains_key(&k) {
                        p.error_soft
                    } else {
                        p.surface
                    }),
            )
            .labelled_by(items_label);
            ui.label(RichText::new("%").color(p.text_weak));
            if n_items > 0
                && ui
                    .small_button(icons::ERROR)
                    .on_hover_text("删除该税目")
                    .clicked()
            {
                remove = Some(i);
            }
            if let Some(e) = errors.get(&k) {
                ui.label(RichText::new(e).size(theme::FS_SMALL).color(p.error));
            }
        });
    }
    if let Some(i) = remove {
        d.items.remove(i);
    }
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + 8.0);
        if ui.button("＋ 添加税目").clicked() {
            d.items.push(ItemDraft {
                name: "新税目".into(),
                rate: "0".into(),
            });
        }
        let modified = d.items != def.items;
        tag_badge(
            ui,
            if modified {
                Tag::Modified
            } else {
                Tag::Confirmed
            },
        );
        if modified && ui.small_button(format!("{} 默认", icons::RESET)).clicked() {
            d.items = def.items.clone();
        }
    });
}

fn iit_section(ui: &mut Ui, d: &mut SettingsDraft, errors: &Errors) {
    let def = d.defaults();
    let p = palette(ui);
    group_title(ui, "减除费用（国家税务总局公告 2018 年第 61 号第八条）");
    if row(
        ui,
        "定额减除适用上限",
        "每次收入不超过该金额时，减除定额费用",
        Tag::Verified,
        !same_value(&d.fixed_limit, &def.fixed_limit),
        errors.get(key::FIXED_LIMIT),
        |ui, l| {
            text_field(
                ui,
                l,
                &mut d.fixed_limit,
                "元",
                errors.contains_key(key::FIXED_LIMIT),
                true,
            )
        },
    ) {
        d.fixed_limit = def.fixed_limit.clone();
    }
    if row(
        ui,
        "定额减除费用",
        "",
        Tag::Verified,
        !same_value(&d.fixed_deduction, &def.fixed_deduction),
        errors.get(key::FIXED_DEDUCTION),
        |ui, l| {
            text_field(
                ui,
                l,
                &mut d.fixed_deduction,
                "元",
                errors.contains_key(key::FIXED_DEDUCTION),
                true,
            )
        },
    ) {
        d.fixed_deduction = def.fixed_deduction.clone();
    }
    if row(
        ui,
        "定率减除比例",
        "每次收入超过上限时，减除收入的该比例",
        Tag::Verified,
        !same_value(&d.pct_deduction, &def.pct_deduction),
        errors.get(key::PCT_DEDUCTION),
        |ui, l| {
            text_field(
                ui,
                l,
                &mut d.pct_deduction,
                "%",
                errors.contains_key(key::PCT_DEDUCTION),
                true,
            )
        },
    ) {
        d.pct_deduction = def.pct_deduction.clone();
    }
    group_title(ui, "预扣率表（居民个人劳务报酬，表二）");
    ui.horizontal(|ui| {
        let modified = d.brackets != def.brackets;
        tag_badge(
            ui,
            if modified {
                Tag::Modified
            } else {
                Tag::Verified
            },
        );
        if modified
            && ui
                .small_button(format!("{} 恢复默认预扣率表", icons::RESET))
                .clicked()
        {
            d.brackets = def.brackets.clone();
        }
        small(ui, "上限含本数；最后一档无上限。");
    });
    egui::Grid::new("brackets_grid")
        .num_columns(6)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            let heads: Vec<Id> = ["档次", "应纳税所得额上限", "", "预扣率", "速算扣除数", ""]
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
                let k = format!("{}.{i}", key::BRACKETS);
                let bad = errors.contains_key(&k);
                let bg = if bad { p.error_soft } else { p.surface };
                ui.label(RichText::new(format!("第 {} 档", i + 1)).color(p.text));
                if i + 1 == n {
                    ui.label(RichText::new("无上限").color(p.text_weak));
                } else {
                    ui.add(
                        TextEdit::singleline(&mut b.upper)
                            .font(theme::mono(14.0))
                            .desired_width(110.0)
                            .background_color(bg),
                    )
                    .labelled_by(heads[1]);
                }
                ui.label(RichText::new("元").color(p.text_weak));
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut b.rate)
                            .font(theme::mono(14.0))
                            .desired_width(60.0)
                            .background_color(bg),
                    )
                    .labelled_by(heads[3]);
                    ui.label(RichText::new("%").color(p.text_weak));
                });
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut b.quick)
                            .font(theme::mono(14.0))
                            .desired_width(90.0)
                            .background_color(bg),
                    )
                    .labelled_by(heads[4]);
                    ui.label(RichText::new("元").color(p.text_weak));
                });
                ui.horizontal(|ui| {
                    if n > 1
                        && ui
                            .small_button(icons::ERROR)
                            .on_hover_text("删除该档")
                            .clicked()
                    {
                        remove = Some(i);
                    }
                    if let Some(e) = errors.get(&k) {
                        ui.label(RichText::new(e).size(theme::FS_SMALL).color(p.error));
                    }
                });
                ui.end_row();
            }
            if let Some(i) = remove {
                d.brackets.remove(i);
            }
        });
    if let Some(e) = errors.get(key::BRACKETS) {
        ui.label(RichText::new(e).color(p.error));
    }
    if ui.button("＋ 插入一档（在最后一档之前）").clicked() {
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
        tag_badge(ui, Tag::Pending);
        small(ui, "仅实现居民个人预扣预缴（预扣率表二）；非居民个人适用税率表三，未实现。是否有非居民个人请确认。");
    });
}

fn rounding_section(ui: &mut Ui, d: &mut SettingsDraft, errors: &Errors) {
    let def = d.defaults();
    group_title(ui, "舍入");
    if row(
        ui,
        "金额小数位",
        "2 表示到分；每一步金额（增值税、附加税、20% 减除费用、个税）单独舍入",
        Tag::Pending,
        d.scale.trim() != def.scale,
        errors.get(key::SCALE),
        |ui, l| {
            text_field(
                ui,
                l,
                &mut d.scale,
                "位",
                errors.contains_key(key::SCALE),
                true,
            )
        },
    ) {
        d.scale = def.scale.clone();
    }
    if row(
        ui,
        "舍入方式",
        "是否与开票、申报系统一致待确认",
        Tag::Pending,
        d.mode != def.mode,
        None,
        |ui, l| {
            ComboBox::from_id_salt("rounding_mode")
                .selected_text(rounding_mode_text(d.mode))
                .width(CTRL_W)
                .show_ui(ui, |ui| {
                    for m in [
                        RoundingMode::HalfUp,
                        RoundingMode::HalfEven,
                        RoundingMode::Down,
                        RoundingMode::Up,
                    ] {
                        ui.selectable_value(&mut d.mode, m, rounding_mode_text(m));
                    }
                })
                .response
                .labelled_by(l);
        },
    ) {
        d.mode = def.mode;
    }
    if row(
        ui,
        "附加税舍入时点",
        "多个税目时生效",
        Tag::Pending,
        d.surcharge_rounding != def.surcharge_rounding,
        None,
        |ui, l| {
            ComboBox::from_id_salt("surcharge_rounding")
                .selected_text(surcharge_rounding_text(d.surcharge_rounding))
                .width(CTRL_W)
                .show_ui(ui, |ui| {
                    for m in [SurchargeRounding::PerItem, SurchargeRounding::Total] {
                        ui.selectable_value(
                            &mut d.surcharge_rounding,
                            m,
                            surcharge_rounding_text(m),
                        );
                    }
                })
                .response
                .labelled_by(l);
        },
    ) {
        d.surcharge_rounding = def.surcharge_rounding;
    }
    group_title(ui, "反算");
    if row(
        ui,
        "多解时取值",
        "多个不含税收入得到同一到手金额时",
        Tag::Confirmed,
        d.prefer != def.prefer,
        None,
        |ui, l| {
            ComboBox::from_id_salt("prefer")
                .selected_text(prefer_text(d.prefer))
                .width(CTRL_W + 40.0)
                .show_ui(ui, |ui| {
                    for m in [Prefer::MinCost, Prefer::MaxCost] {
                        ui.selectable_value(&mut d.prefer, m, prefer_text(m));
                    }
                })
                .response
                .labelled_by(l);
        },
    ) {
        d.prefer = def.prefer;
    }
    if row(
        ui,
        "逐分校准窗口",
        "二分定位后前后各逐分扫描的范围",
        Tag::Implementation,
        d.search_window.trim() != def.search_window,
        errors.get(key::SEARCH_WINDOW),
        |ui, l| {
            text_field(
                ui,
                l,
                &mut d.search_window,
                "分",
                errors.contains_key(key::SEARCH_WINDOW),
                true,
            )
        },
    ) {
        d.search_window = def.search_window.clone();
    }
    if row(
        ui,
        "反算搜索上限",
        "不含税收入的最大值",
        Tag::Implementation,
        !same_value(&d.max_net_income, &def.max_net_income),
        errors.get(key::MAX_NET),
        |ui, l| {
            text_field(
                ui,
                l,
                &mut d.max_net_income,
                "元",
                errors.contains_key(key::MAX_NET),
                true,
            )
        },
    ) {
        d.max_net_income = def.max_net_income.clone();
    }
}

fn appearance_section(ui: &mut Ui, d: &mut SettingsDraft, store_dir: Option<&str>) {
    let p = palette(ui);
    group_title(ui, "主题");
    ui.horizontal(|ui| {
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
                if selected { p.primary } else { p.border },
            ))
            .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
            .min_size(Vec2::new(120.0, 40.0));
            if ui.add(b).clicked() {
                d.theme = t;
            }
        }
    });
    small(
        ui,
        "“跟随系统”会随操作系统的浅色/深色外观自动切换。选择后立即预览，保存后记住。",
    );
    group_title(ui, "设置保存位置");
    match store_dir {
        Some(dir) => {
            ui.label(RichText::new(dir).font(theme::mono(13.0)).color(p.text));
            small(ui, "主题保存在 preferences.toml，口径保存在 rules.toml（与内置默认相同时不保存）。由程序自动维护，无需手工编辑。");
        }
        None => {
            small(ui, "当前环境没有可用的配置目录，设置只在本次运行有效。");
        }
    }
}
