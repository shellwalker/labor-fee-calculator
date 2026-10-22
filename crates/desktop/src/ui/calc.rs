//! 正算 / 反算 / 按总成本 三个单笔计算页。
//!
//! 布局随宽度自适应：
//! - 宽（≥ 1120）：输入列 | 结果列 | 试算规则列；
//! - 中（≥ 820）：输入列（下接试算规则）| 结果列；
//! - 窄：单列滚动。

use super::widgets::{
    amount_row, badge, card, card_title, chip, kpi_card, money, secondary_button, small,
    status_banner,
};
use crate::icons;
use crate::rules::{self, POLICY_NAME, POLICY_URL, POLICY_URL_GOV};
use crate::settings::modified_items;
use crate::state::{AppState, Level, Outcome, SingleTab, Tab};
use crate::theme::{self, palette};
use eframe::egui::{
    self, Align, Color32, CornerRadius, Grid, Key, Layout, RichText, ScrollArea, Sense, Stroke,
    TextEdit, Ui, Vec2,
};
use labor_fee_core::{Breakdown, Config, CostGap, Decimal};

struct PageSpec {
    input_label: &'static str,
    hint: &'static str,
    examples: &'static [&'static str],
    page_note: &'static str,
}

fn spec(tab: Tab) -> PageSpec {
    match tab {
        Tab::Forward => PageSpec {
            input_label: "不含增值税收入",
            hint: "例如 10000",
            examples: &["800", "999.99", "1000", "4000", "10000", "50000"],
            page_note: "本页以不含增值税收入为输入，其余各项金额均由其确定。",
        },
        Tab::Reverse => PageSpec {
            input_label: "税后实发金额",
            hint: "例如 8400",
            examples: &["959.99", "960", "3360", "8400", "20000", "50000"],
            page_note: "本页根据税后实发金额测算不含增值税收入，并据此计算各项金额；复核结果与输入一致到分。",
        },
        _ => PageSpec {
            input_label: "企业总支出",
            hint: "例如 10103.00",
            examples: &["999.99", "1005", "1010.30", "10103.00", "50500"],
            page_note: "本页根据企业总支出测算不含增值税收入；金额不可达时，说明原因并给出两侧最近的有效金额。",
        },
    }
}

fn tab_state(st: &mut AppState, tab: Tab) -> &mut SingleTab {
    match tab {
        Tab::Forward => &mut st.forward,
        Tab::Reverse => &mut st.reverse,
        _ => &mut st.from_cost,
    }
}

fn calc(st: &mut AppState, tab: Tab) {
    match tab {
        Tab::Forward => st.calc_forward(),
        Tab::Reverse => st.calc_reverse(),
        _ => st.calc_from_cost(),
    }
}

/// 绘制单笔计算页。
pub fn page(ui: &mut Ui, st: &mut AppState, tab: Tab) {
    let w = ui.available_width();
    let h = ui.available_height();
    let gap = theme::SP4;
    let left_w = 284.0_f32.min(w);
    if w >= 1120.0 {
        let show = st.show_rules;
        let right_w = if show { 320.0 } else { 0.0 };
        let mid_w = w - left_w - right_w - gap * if show { 2.0 } else { 1.0 };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            column(ui, "in", left_w, h, |ui| input_column(ui, st, tab));
            column(ui, "out", mid_w, h, |ui| result_column(ui, st, tab, mid_w));
            if show {
                column(ui, "rules", right_w, h, |ui| rules_card(ui, st, tab, true));
            }
        });
    } else if w >= 820.0 {
        let mid_w = w - left_w - gap;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            column(ui, "in", left_w, h, |ui| {
                input_column(ui, st, tab);
                ui.add_space(theme::SP2);
                rules_card(ui, st, tab, false);
            });
            column(ui, "out", mid_w, h, |ui| result_column(ui, st, tab, mid_w));
        });
    } else {
        ScrollArea::vertical().id_salt("single").show(ui, |ui| {
            input_column(ui, st, tab);
            ui.add_space(theme::SP2);
            result_column(ui, st, tab, w);
            ui.add_space(theme::SP2);
            rules_card(ui, st, tab, false);
        });
    }
}

fn column(ui: &mut Ui, id: &str, w: f32, h: f32, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(Vec2::new(w, h), Layout::top_down(Align::Min), |ui| {
        ui.set_width(w);
        ScrollArea::vertical()
            .id_salt(("col", id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(w - 2.0);
                add(ui);
            });
    });
}

// ---------------- 输入列 ----------------

fn input_column(ui: &mut Ui, st: &mut AppState, tab: Tab) {
    let sp = spec(tab);
    let p = palette(ui);
    let mut go = false;
    card(ui, |ui| {
        card_title(ui, icons::MONEY, "输入金额", |_| {});
        let l = ui.label(RichText::new(format!("{}（元）", sp.input_label)).color(p.text_weak));
        let t = tab_state(st, tab);
        ui.horizontal(|ui| {
            let r = ui
                .add(
                    TextEdit::singleline(&mut t.input)
                        .font(theme::mono(22.0))
                        .hint_text(sp.hint)
                        .margin(egui::Margin::symmetric(10, 8))
                        .desired_width(ui.available_width() - 28.0),
                )
                .labelled_by(l.id);
            ui.label(RichText::new("元").size(16.0).color(p.text_weak));
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                go = true;
            }
        });
        ui.add_space(theme::SP1);
        let w = ui.available_width();
        ui.scope(|ui| {
            ui.spacing_mut().interact_size.y = 36.0;
            if ui
                .add_sized(
                    [w, 36.0],
                    egui::Button::new(
                        RichText::new("计算")
                            .color(p.on_primary)
                            .strong()
                            .size(15.0),
                    )
                    .fill(p.primary)
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(theme::RADIUS_CTRL)),
                )
                .clicked()
            {
                go = true;
            }
        });
        small(ui, "按回车键亦可计算；支持千分位分隔符和 ¥ 符号。");
        ui.add_space(theme::SP3);
        ui.label(
            RichText::new("快捷示例")
                .color(p.text_weak)
                .size(theme::FS_SMALL),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            for ex in sp.examples {
                if chip(ui, ex).clicked() {
                    tab_state(st, tab).input = (*ex).to_string();
                    go = true;
                }
            }
        });
    });
    if go {
        calc(st, tab);
    }
    ui.add_space(theme::SP2);
    history_card(ui, st, tab);
}

fn history_card(ui: &mut Ui, st: &mut AppState, tab: Tab) {
    let p = palette(ui);
    let mut pick = None;
    card(ui, |ui| {
        card_title(ui, icons::CHART, "最近记录", |_| {});
        let t = tab_state(st, tab);
        if t.history.is_empty() {
            small(ui, "已计算的金额将列示于此，点击可重新查看。");
        }
        for (i, h) in t.history.iter().enumerate() {
            let text = format!("{}  →  实发 {}", money(h.input), money(h.take_home));
            let r = ui.add(
                egui::Button::new(RichText::new(text).font(theme::mono(13.0)).color(p.text))
                    .fill(if i % 2 == 0 { p.surface_alt } else { p.surface })
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::new(ui.available_width(), 26.0)),
            );
            if r.on_hover_text(format!(
                "不含增值税收入 {}，企业总支出 {}",
                money(h.net_income),
                money(h.total_cost)
            ))
            .clicked()
            {
                pick = Some(h.input);
            }
        }
    });
    if let Some(v) = pick {
        tab_state(st, tab).input = v.to_string();
        calc(st, tab);
    }
}

// ---------------- 结果列 ----------------

/// 附加税费项目的费率说明：未减征为“3%”，减征时为“3% × 减征 50% = 1.5%”。
fn rate_label(statutory: Decimal, relief: Decimal) -> String {
    if relief.is_zero() {
        rules::pct(statutory)
    } else {
        format!("{}，减征 {}", rules::pct(statutory), rules::pct(relief))
    }
}

/// 按宽度排布 KPI：很宽一行排开；中等优先 3+2（五张时）或对半；较窄两列；更窄纵向。
fn kpi_row(ui: &mut Ui, width: f32, cards: &[(&str, Option<&str>, Color32, &str)]) {
    let n = cards.len();
    if n == 0 {
        return;
    }
    let paint = |ui: &mut Ui, slice: &[(&str, Option<&str>, Color32, &str)]| {
        if slice.is_empty() {
            return;
        }
        if slice.len() == 1 {
            let (label, value, accent, note) = slice[0];
            kpi_card(ui, label, value, accent, note, false);
            return;
        }
        ui.columns(slice.len(), |cols| {
            for (i, &(label, value, accent, note)) in slice.iter().enumerate() {
                kpi_card(&mut cols[i], label, value, accent, note, false);
            }
        });
    };
    if width >= 880.0 {
        paint(ui, cards);
    } else if width >= 520.0 {
        let mid = if n == 5 { 3 } else { n.div_ceil(2) };
        paint(ui, &cards[..mid]);
        paint(ui, &cards[mid..]);
    } else if width >= 360.0 {
        for chunk in cards.chunks(2) {
            paint(ui, chunk);
        }
    } else {
        for &(label, value, accent, note) in cards {
            kpi_card(ui, label, value, accent, note, false);
        }
    }
}

fn result_column(ui: &mut Ui, st: &mut AppState, tab: Tab, width: f32) {
    let p = palette(ui);
    let outcome = tab_state(st, tab).outcome.clone();
    let b: Option<&Breakdown> = match &outcome {
        Outcome::Done { breakdown, .. } => Some(breakdown),
        _ => None,
    };
    let vat_rate = rules::pct(st.config().vat.rate);
    let sc = &st.config().surcharge;
    let item_labels: Vec<(String, String)> = sc
        .enabled_items()
        .map(|i| (i.name.clone(), rate_label(i.rate, sc.applied_relief())))
        .collect();
    if !st.show_rules && width >= 700.0 {
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if secondary_button(ui, &format!("{} 显示计算规则", icons::RULES)).clicked() {
                st.show_rules = true;
            }
        });
    }
    // 关键指标：用户输入项不做卡片；增值税税额必有；附加税费合计独立卡片（明细区已列项，
    // 卡片给一眼合计）。各页 KPI 见 docs/UI_DESIGN.md。
    let vat_card_note = b.map_or(format!("征收率 {vat_rate}"), |b| {
        if b.vat_exempt {
            "未达到起征点，不征收".to_string()
        } else {
            format!("征收率 {vat_rate}")
        }
    });
    let surcharge_note = b.map_or("= 增值税税额 × 各项目费率".to_string(), |b| {
        if b.vat_exempt {
            "未达到起征点，不征收".to_string()
        } else if b.surcharge_relief.is_zero() {
            "各项分别按分舍入".to_string()
        } else {
            format!(
                "减征前 {}，减征 {}",
                money(b.surcharge_before_relief),
                money(b.surcharge_relief)
            )
        }
    });
    let cost_note = b.map_or("= 收入 + 增值税 + 附加税费".to_string(), |b| {
        if b.vat_exempt {
            "未达到起征点，不征收增值税".to_string()
        } else {
            format!("其中税费 {}", money(b.vat + b.surcharge_total))
        }
    });
    let iit_note = b.map_or("劳务报酬所得预扣预缴".to_string(), |b| {
        format!("预扣率 {}", rules::pct(b.iit.rate))
    });
    let net_note = "劳务报酬每次收入".to_string();
    let home_note = "= 收入 − 应预扣预缴税额".to_string();
    let net_amt = b.map(|b| money(b.net_income));
    let vat_amt = b.map(|b| money(b.vat));
    let surcharge_amt = b.map(|b| money(b.surcharge_total));
    let total = b.map(|b| money(b.total_cost));
    let tax = b.map(|b| money(b.iit.tax));
    let home = b.map(|b| money(b.take_home));
    // 各页 KPI（不含用户输入）：(标签, 金额, 色, 说明)
    type Kpi<'a> = (&'a str, Option<&'a str>, Color32, &'a str);
    let cards: Vec<Kpi<'_>> = match tab {
        Tab::Forward => vec![
            ("增值税税额", vat_amt.as_deref(), p.primary, &vat_card_note),
            (
                "附加税费合计",
                surcharge_amt.as_deref(),
                p.primary,
                &surcharge_note,
            ),
            ("企业总支出", total.as_deref(), p.primary, &cost_note),
            ("应预扣预缴税额", tax.as_deref(), p.warning, &iit_note),
            ("税后实发金额", home.as_deref(), p.success, &home_note),
        ],
        Tab::Reverse => vec![
            ("不含增值税收入", net_amt.as_deref(), p.primary, &net_note),
            ("增值税税额", vat_amt.as_deref(), p.primary, &vat_card_note),
            (
                "附加税费合计",
                surcharge_amt.as_deref(),
                p.primary,
                &surcharge_note,
            ),
            ("企业总支出", total.as_deref(), p.primary, &cost_note),
            ("应预扣预缴税额", tax.as_deref(), p.warning, &iit_note),
        ],
        Tab::FromCost => vec![
            ("不含增值税收入", net_amt.as_deref(), p.primary, &net_note),
            ("增值税税额", vat_amt.as_deref(), p.primary, &vat_card_note),
            (
                "附加税费合计",
                surcharge_amt.as_deref(),
                p.primary,
                &surcharge_note,
            ),
            ("应预扣预缴税额", tax.as_deref(), p.warning, &iit_note),
            ("税后实发金额", home.as_deref(), p.success, &home_note),
        ],
        Tab::Batch => vec![],
    };
    kpi_row(ui, width, &cards);
    ui.add_space(theme::SP1);
    // 状态 / 警示
    match &outcome {
        Outcome::None => status_hint(ui),
        Outcome::Error(e) => status_banner(ui, Level::Error, e),
        Outcome::Done { status, .. } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new("校验状态").color(p.text_weak));
                let (fg, bg) = super::widgets::level_colors(p, status.level);
                badge(
                    ui,
                    format!(
                        "{} {}",
                        super::widgets::level_icon(status.level),
                        match status.level {
                            Level::Ok => "通过",
                            Level::Warn => "注意",
                            Level::Error => "失败",
                        }
                    ),
                    fg,
                    bg,
                );
            });
            status_banner(ui, status.level, &status.text);
        }
        Outcome::Gap(g) => {
            if let Some(cost) = gap_card(ui, st.config(), g) {
                st.use_cost(cost);
            }
        }
    }
    ui.add_space(theme::SP1);
    // 明细分组
    let two = width >= 600.0;
    let is_input = |name: &str| match tab {
        Tab::Forward => name == "不含增值税收入",
        Tab::Reverse => name == "税后实发金额",
        _ => name == "企业总支出",
    };
    let vat_card = |ui: &mut Ui| {
        card(ui, |ui| {
            card_title(ui, icons::TAX, "增值税及附加税费", |ui| {
                badge(ui, "企业承担", p.primary, p.primary_soft);
            });
            let v = |f: fn(&Breakdown) -> Decimal| b.map_or("—".into(), |b| money(f(b)));
            amount_row(
                ui,
                &mark("不含增值税收入", is_input("不含增值税收入")),
                &v(|b| b.net_income),
                "元",
                false,
                p.text,
            );
            amount_row(
                ui,
                &format!("增值税税额（{vat_rate}）"),
                &v(|b| b.vat),
                "元",
                false,
                p.text,
            );
            match b {
                Some(b) => {
                    for l in &b.surcharges {
                        amount_row(
                            ui,
                            &format!(
                                "　{}（{}）",
                                l.name,
                                rate_label(l.statutory_rate, l.relief_rate)
                            ),
                            &money(l.amount),
                            "元",
                            false,
                            p.text,
                        );
                        if !l.relief_rate.is_zero() {
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 16.0),
                                Layout::right_to_left(Align::Center),
                                |ui| {
                                    ui.add_space(22.0);
                                    small(
                                        ui,
                                        format!(
                                            "减征前 {} 元，减征 {} 元",
                                            money(l.amount_before_relief),
                                            money(l.relief_amount)
                                        ),
                                    );
                                },
                            );
                        }
                    }
                    if !b.surcharge_relief.is_zero() {
                        amount_row(
                            ui,
                            "附加税费减征额",
                            &money(b.surcharge_relief),
                            "元",
                            false,
                            p.text,
                        );
                    }
                }
                None => {
                    for (name, rate) in &item_labels {
                        amount_row(ui, &format!("　{name}（{rate}）"), "—", "元", false, p.text);
                    }
                }
            }
            amount_row(
                ui,
                "增值税附加税费合计",
                &v(|b| b.surcharge_total),
                "元",
                false,
                p.text,
            );
            amount_row(
                ui,
                &mark("企业总支出", is_input("企业总支出")),
                &v(|b| b.total_cost),
                "元",
                true,
                p.primary,
            );
        });
    };
    let iit_card = |ui: &mut Ui| {
        card(ui, |ui| {
            card_title(ui, icons::MONEY, "个人所得税", |ui| {
                badge(ui, "预扣预缴", p.warning, p.warning_soft);
            });
            let v = |f: fn(&Breakdown) -> Decimal| b.map_or("—".into(), |b| money(f(b)));
            amount_row(ui, "减除费用", &v(|b| b.iit.deduction), "元", false, p.text);
            amount_row(
                ui,
                "预扣预缴应纳税所得额",
                &v(|b| b.iit.taxable_income),
                "元",
                false,
                p.text,
            );
            amount_row(
                ui,
                "预扣率",
                &b.map_or("—".into(), |b| rules::pct(b.iit.rate)),
                "",
                false,
                p.text,
            );
            amount_row(
                ui,
                "速算扣除数",
                &v(|b| b.iit.quick_deduction),
                "元",
                false,
                p.text,
            );
            amount_row(ui, "应预扣预缴税额", &v(|b| b.iit.tax), "元", false, p.text);
            amount_row(
                ui,
                &mark("税后实发金额", is_input("税后实发金额")),
                &v(|b| b.take_home),
                "元",
                true,
                p.success,
            );
        });
    };
    if two {
        ui.columns(2, |c| {
            vat_card(&mut c[0]);
            iit_card(&mut c[1]);
        });
    } else {
        vat_card(ui);
        ui.add_space(theme::SP2);
        iit_card(ui);
    }
    if let Some(b) = b {
        ui.add_space(theme::SP1);
        formula_strip(ui, b);
    }
}

fn mark(label: &str, input: bool) -> String {
    if input {
        format!("{label}（输入）")
    } else {
        label.to_string()
    }
}

fn status_hint(ui: &mut Ui) {
    let p = palette(ui);
    egui::Frame::new()
        .fill(p.info_soft)
        .corner_radius(CornerRadius::same(theme::RADIUS_CTRL + 2))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::TIP).color(p.primary));
                ui.label(
                    RichText::new("输入金额后按回车键或点击“计算”，也可选用左侧的快捷示例。")
                        .color(p.primary),
                );
            });
        });
}

/// 底部算式条：把本次结果代入公式，便于核对。
fn formula_strip(ui: &mut Ui, b: &Breakdown) {
    let p = palette(ui);
    egui::Frame::new()
        .fill(p.surface_alt)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(theme::RADIUS_CTRL + 2))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let f = theme::mono(12.5);
            ui.label(
                RichText::new("算式核对")
                    .size(theme::FS_SMALL)
                    .color(p.text_weak),
            );
            ui.label(
                RichText::new(format!(
                    "企业总支出 {} = 收入 {} + 增值税 {} + 附加税费 {}",
                    money(b.total_cost),
                    money(b.net_income),
                    money(b.vat),
                    money(b.surcharge_total)
                ))
                .font(f.clone())
                .color(p.text),
            );
            ui.label(
                RichText::new(format!(
                    "应预扣预缴税额 {} = 应纳税所得额 {} × {} − {}",
                    money(b.iit.tax),
                    money(b.iit.taxable_income),
                    rules::pct(b.iit.rate),
                    money(b.iit.quick_deduction)
                ))
                .font(f.clone())
                .color(p.text),
            );
            ui.label(
                RichText::new(format!(
                    "税后实发金额 {} = 收入 {} − 预扣税额 {}",
                    money(b.take_home),
                    money(b.net_income),
                    money(b.iit.tax)
                ))
                .font(f)
                .color(p.text),
            );
        });
}

/// 不可达金额警示卡；返回被点击的有效企业总支出。
fn gap_card(ui: &mut Ui, cfg: &Config, g: &CostGap) -> Option<Decimal> {
    let p = palette(ui);
    let mut clicked = None;
    let r = egui::Frame::new()
        .fill(p.error_soft)
        .stroke(Stroke::new(1.0, p.error.gamma_multiply(0.5)))
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::WARN).size(18.0).color(p.error));
                ui.label(
                    RichText::new("该企业总支出无对应的不含增值税收入")
                        .size(theme::FS_SECTION)
                        .color(p.error)
                        .strong(),
                );
            });
            ui.label(
                RichText::new(crate::state::gap_reason_text(g))
                    .color(p.error)
                    .strong(),
            );
            ui.add(egui::Label::new(RichText::new(g.to_string()).color(p.text)).wrap());
            ui.add_space(theme::SP2);
            gap_line(ui, cfg, g);
            ui.add_space(theme::SP2);
            ui.label(
                RichText::new("可改用两侧最近的有效企业总支出（点击重新测算）：")
                    .color(p.text_weak),
            );
            ui.columns(2, |c| {
                for (i, (side, arrow)) in [(g.lower, icons::LEFT), (g.upper, icons::RIGHT)]
                    .into_iter()
                    .enumerate()
                {
                    let ui = &mut c[i];
                    let text = if i == 0 {
                        format!("{arrow} {}（收入 {}）", side.total_cost, side.net_income)
                    } else {
                        format!("{}（收入 {}）{arrow}", side.total_cost, side.net_income)
                    };
                    let b = egui::Button::new(
                        RichText::new(text).font(theme::mono(14.0)).color(p.primary),
                    )
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.primary))
                    .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
                    .min_size(Vec2::new(ui.available_width(), 38.0));
                    if ui.add(b).clicked() {
                        clicked = Some(side.total_cost);
                    }
                }
            });
        });
    let _ = r;
    clicked
}

/// 数轴示意：下侧有效值 — 空白段（含输入位置）— 上侧有效值。
fn gap_line(ui: &mut Ui, cfg: &Config, g: &CostGap) {
    let p = palette(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::hover());
    let y = rect.top() + 16.0;
    let x0 = rect.left() + 60.0;
    let x1 = rect.right() - 60.0;
    let painter = ui.painter();
    painter.line_segment(
        [egui::pos2(rect.left() + 8.0, y), egui::pos2(x0, y)],
        Stroke::new(3.0, p.success),
    );
    painter.line_segment(
        [egui::pos2(x1, y), egui::pos2(rect.right() - 8.0, y)],
        Stroke::new(3.0, p.success),
    );
    // 空白段：虚线
    let mut x = x0;
    while x < x1 {
        let e = (x + 6.0).min(x1);
        painter.line_segment(
            [egui::pos2(x, y), egui::pos2(e, y)],
            Stroke::new(3.0, p.error),
        );
        x += 10.0;
    }
    for (xx, v) in [(x0, g.lower.total_cost), (x1, g.upper.total_cost)] {
        painter.circle_filled(egui::pos2(xx, y), 5.0, p.success);
        painter.text(
            egui::pos2(xx, y + 9.0),
            egui::Align2::CENTER_TOP,
            money(v),
            theme::mono(12.0),
            p.text,
        );
    }
    let span = g.upper.total_cost - g.lower.total_cost;
    let t = if span.is_zero() {
        0.5
    } else {
        let f = (g.total_cost - g.lower.total_cost) / span;
        f.to_string()
            .parse::<f32>()
            .unwrap_or(0.5)
            .clamp(0.05, 0.95)
    };
    let xi = x0 + (x1 - x0) * t;
    painter.circle_stroke(egui::pos2(xi, y), 6.0, Stroke::new(2.5, p.error));
    painter.text(
        egui::pos2(xi, y - 9.0),
        egui::Align2::CENTER_BOTTOM,
        format!("输入 {}", money(g.total_cost)),
        theme::prop(11.5),
        p.error,
    );
    let unit = cfg.unit();
    painter.text(
        egui::pos2((x0 + x1) / 2.0, y + 9.0),
        egui::Align2::CENTER_TOP,
        format!(
            "{}～{} 无对应收入",
            money(g.lower.total_cost + unit),
            money(g.upper.total_cost - unit)
        ),
        theme::prop(11.5),
        p.error,
    );
}

// ---------------- 试算规则 ----------------

fn rules_card(ui: &mut Ui, st: &mut AppState, tab: Tab, collapsible: bool) {
    let p = palette(ui);
    let sp = spec(tab);
    let cfg = st.config().clone();
    let modified = modified_items(&cfg);
    let mut open_settings = false;
    let mut hide = false;
    card(ui, |ui| {
        card_title(ui, icons::RULES, "计算规则", |ui| {
            if collapsible
                && ui
                    .small_button("收起")
                    .on_hover_text("收起规则面板")
                    .clicked()
            {
                hide = true;
            }
            if modified == 0 {
                badge(ui, "默认参数", p.success, p.success_soft);
            } else {
                badge(
                    ui,
                    format!("已修改 {modified} 项"),
                    p.warning,
                    p.warning_soft,
                );
            }
        });
        small(ui, "根据当前生效的计算参数自动生成。");
        ui.add_space(theme::SP1);
        egui::Frame::new()
            .fill(p.info_soft)
            .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(sp.page_note).color(p.primary)).wrap());
            });
        for g in rules::rule_groups(&cfg) {
            ui.add_space(theme::SP2);
            ui.label(RichText::new(g.title).color(p.text).strong());
            for l in &g.lines {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new("•").color(p.accent));
                    ui.add(egui::Label::new(RichText::new(l).color(p.text_weak)).wrap());
                });
            }
            if g.title.starts_with("个人所得税") {
                bracket_table(ui, &cfg);
            }
        }
        ui.add_space(theme::SP2);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(
                RichText::new("政策依据：")
                    .size(theme::FS_SMALL)
                    .color(p.text_weak),
            );
            ui.hyperlink_to(RichText::new(POLICY_NAME).size(theme::FS_SMALL), POLICY_URL);
            ui.label(
                RichText::new("（第八条、预扣率表二；")
                    .size(theme::FS_SMALL)
                    .color(p.text_weak),
            );
            ui.hyperlink_to(
                RichText::new("国务院公报").size(theme::FS_SMALL),
                POLICY_URL_GOV,
            );
            ui.label(
                RichText::new("）；")
                    .size(theme::FS_SMALL)
                    .color(p.text_weak),
            );
            ui.hyperlink_to(
                RichText::new(rules::POLICY_VAT_NAME).size(theme::FS_SMALL),
                rules::POLICY_VAT_URL,
            );
            ui.label(
                RichText::new("（起征点、优惠征收率）")
                    .size(theme::FS_SMALL)
                    .color(p.text_weak),
            );
        });
        small(
            ui,
            "附加税费项目及费率为可配置参数。本工具仅用于测算，不构成税务意见。",
        );
        ui.add_space(theme::SP1);
        // 口径状态（已确认 / 已核实等）不在界面展示，见 docs/ASSUMPTIONS.md
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.link(format!("{} 修改参数…", icons::SETTINGS)).clicked() {
                    open_settings = true;
                }
            });
        });
    });
    if open_settings {
        st.open_settings();
    }
    if hide {
        st.show_rules = false;
    }
}

fn bracket_table(ui: &mut Ui, cfg: &Config) {
    let p = palette(ui);
    ui.add_space(theme::SP1);
    egui::Frame::new()
        .fill(p.surface_alt)
        .corner_radius(CornerRadius::same(theme::RADIUS_CTRL))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            Grid::new("rules_brackets")
                .num_columns(3)
                .spacing([18.0, 4.0])
                .show(ui, |ui| {
                    for h in ["应纳税所得额（元）", "预扣率", "速算扣除数"] {
                        ui.label(RichText::new(h).size(theme::FS_SMALL).color(p.text_weak));
                    }
                    ui.end_row();
                    for r in rules::bracket_rows(cfg) {
                        ui.label(RichText::new(r.range).font(theme::mono(12.5)).color(p.text));
                        ui.label(RichText::new(r.rate).font(theme::mono(12.5)).color(p.text));
                        ui.label(RichText::new(r.quick).font(theme::mono(12.5)).color(p.text));
                        ui.end_row();
                    }
                });
        });
}
