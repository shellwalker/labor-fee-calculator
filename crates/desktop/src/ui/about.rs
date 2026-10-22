//! “关于”与“使用说明”窗口。

use super::widgets::{card, small};
use crate::rules::{
    POLICY_DETAIL, POLICY_NAME, POLICY_RELIEF_NAME, POLICY_RELIEF_URL, POLICY_URL, POLICY_URL_GOV,
    POLICY_VAT_NAME, POLICY_VAT_URL,
};
use crate::state::AppState;
use crate::theme::{self, palette};
use crate::{icons, APP_NAME, BUILD_DATE, GIT_HASH, VERSION};
use eframe::egui::{self, Align, Id, Layout, Modal, RichText, TextureHandle, Ui};

/// 程序图标纹理（缓存于 egui 内存）。
pub fn icon_texture(ctx: &egui::Context) -> TextureHandle {
    let id = Id::unique("lfd_icon_texture");
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let icon = eframe::icon_data::from_png_bytes(crate::ICON_PNG).expect("内置图标有效");
    let img = egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    );
    let t = ctx.load_texture("lfd_icon", img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

fn kv(ui: &mut Ui, k: &str, v: &str) {
    let p = palette(ui);
    ui.label(RichText::new(k).color(p.text_weak));
    ui.label(RichText::new(v).color(p.text));
    ui.end_row();
}

pub fn about_window(ctx: &egui::Context, st: &mut AppState) {
    if !st.show_about {
        return;
    }
    let tex = icon_texture(ctx);
    let mut close = false;
    let r = Modal::new(Id::unique("about_modal")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
        let p = palette(ui);
        ui.set_width(560.0);
        ui.horizontal(|ui| {
            ui.add(egui::Image::new((tex.id(), egui::vec2(64.0, 64.0))));
            ui.add_space(theme::SP2);
            ui.vertical(|ui| {
                ui.label(RichText::new(APP_NAME).size(22.0).strong().color(p.text));
                ui.label(RichText::new(format!("版本 {VERSION}")).color(p.primary));
                small(ui, "劳务报酬所得增值税及附加税费、个人所得税预扣预缴的计算与测算");
            });
        });
        ui.add_space(theme::SP3);
        egui::Grid::new("about_grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
            kv(ui, "版本", VERSION);
            kv(ui, "构建", &format!("{GIT_HASH}（{BUILD_DATE}）"));
            kv(ui, "平台", &format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH));
            kv(ui, "许可", "内部使用（UNLICENSED），版权归仓库所有者");
            kv(ui, "计算参数", &st.source);
        });
        ui.add_space(theme::SP2);
        ui.label(RichText::new("政策依据").strong().color(p.text));
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(POLICY_NAME, POLICY_URL);
            ui.label(RichText::new(POLICY_DETAIL).color(p.text_weak));
        });
        ui.hyperlink_to("中国政府网《国务院公报》2019年第12号（含预扣率表）", POLICY_URL_GOV);
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(POLICY_VAT_NAME, POLICY_VAT_URL);
            ui.label(RichText::new("增值税起征点、1% 优惠征收率").color(p.text_weak));
        });
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(POLICY_RELIEF_NAME, POLICY_RELIEF_URL);
            ui.label(RichText::new("“六税两费”减半征收（默认不启用）").color(p.text_weak));
        });
        ui.add_space(theme::SP2);
        ui.label(RichText::new("字体").strong().color(p.text));
        small(
            ui,
            "界面内嵌 LaborFee Sans SC（Noto Sans CJK SC 子集，Copyright 2014-2021 Adobe）与 LaborFee Icons（Noto Emoji 子集，Copyright Google），均按 SIL Open Font License 1.1 授权，许可证随发布包附带（字体许可证-OFL.txt）。数字使用 egui 自带的 Hack 字体（MIT/Bitstream Vera 许可）。",
        );
        ui.add_space(theme::SP2);
        ui.label(RichText::new("免责声明").strong().color(p.text));
        small(
            ui,
            "本工具按所设参数进行测算，结果仅供参考，不构成税务意见。增值税起征点、征收率、附加税费项目及减征等参数请结合自身情况确认；正式结算、申报以主管税务机关的规定和核定为准。",
        );
        ui.add_space(theme::SP3);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if super::widgets::primary_button(ui, "  好的  ", true).clicked() {
                close = true;
            }
        });
    });
    if close || r.should_close() {
        st.show_about = false;
    }
}

pub fn help_window(
    ctx: &egui::Context,
    st: &mut AppState,
    settings_shortcut: &str,
    open_shortcut: &str,
) {
    if !st.show_help {
        return;
    }
    let mut close = false;
    let r = Modal::new(Id::unique("help_modal")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
        let p = palette(ui);
        ui.set_width(620.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(icons::HELP).size(theme::FS_TITLE).color(p.primary));
            ui.label(RichText::new("使用说明").size(theme::FS_TITLE).strong().color(p.text));
        });
        ui.add_space(theme::SP2);
        let items = [
            (icons::FORWARD, "收入计算", "按不含增值税收入计算：得出增值税税额、增值税附加税费、企业总支出、应预扣预缴税额和税后实发金额。"),
            (icons::REVERSE, "实发测算", "按税后实发金额测算不含增值税收入；多解时取最小值，复核结果与输入一致到分。"),
            (icons::COST, "支出测算", "按企业总支出测算不含增值税收入；起征点附近（默认 1000.00～1010.29）等不可达金额无对应收入，将说明原因并给出两侧最近的有效金额。"),
            (icons::BATCH, "批量测算", "拖放或选择 xlsx / xls / ods / csv 文件，按首行标题识别输入列（兼容旧列名“到手金额”等），执行后导出带结果列的新文件。"),
            (icons::SETTINGS, "设置", "逐项修改计算参数（征收率、起征点、附加税费项目、减除费用、预扣率表、舍入等）和主题；保存后自动生效。"),
        ];
        card(ui, |ui| {
            egui::Grid::new("help_grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                for (ic, t, d) in items {
                    ui.label(RichText::new(format!("{ic} {t}")).strong().color(p.text));
                    ui.add(egui::Label::new(RichText::new(d).color(p.text_weak)).wrap());
                    ui.end_row();
                }
            });
        });
        ui.add_space(theme::SP2);
        ui.label(RichText::new("快捷键").strong().color(p.text));
        small(ui, format!("设置 {settings_shortcut}　·　打开批量文件 {open_shortcut}　·　计算：在输入框中按回车键"));
        ui.add_space(theme::SP3);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if super::widgets::primary_button(ui, "  知道了  ", true).clicked() {
                close = true;
            }
        });
    });
    if close || r.should_close() {
        st.show_help = false;
    }
}
