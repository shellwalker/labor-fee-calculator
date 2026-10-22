//! macOS 原生应用菜单（muda）：
//! 应用菜单（关于… / 偏好设置… Cmd+, / 服务 / 隐藏 Cmd+H / 退出 Cmd+Q）、文件（打开批量文件… Cmd+O）、
//! 窗口（最小化 Cmd+M / 关闭窗口 Cmd+W / 缩放 / 全屏）、帮助。
//!
//! “最小化”“关闭窗口”用自定义菜单项，由界面以 `ViewportCommand::Minimized(true)` 实现：
//! 预置的 Close Window 会关闭主窗口并退出程序，不符合“先关对话框，否则最小化”的要求。
//!
//! 菜单事件经通道转给界面，并请求重绘。不提供“编辑”菜单：文本框的 Cmd+C/Cmd+V 由 egui 直接处理。

use crate::state::Action;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::sync::mpsc::{channel, Receiver};

pub struct NativeMenu {
    _menu: Menu,
    rx: Receiver<Action>,
}

fn item(id: &str, text: &str, acc: Option<Accelerator>) -> MenuItem {
    MenuItem::with_id(id, text, true, acc)
}

impl NativeMenu {
    /// 安装菜单（须在 NSApplication 已创建后调用，例如 eframe 的创建回调里）。
    pub fn install(ctx: &eframe::egui::Context) -> Result<Self, muda::Error> {
        let cmd = |c: Code| Some(Accelerator::new(Some(Modifiers::SUPER), c));
        let app = Submenu::with_items(
            crate::APP_NAME,
            true,
            &[
                &item("about", &format!("关于{}", crate::APP_NAME), None),
                &PredefinedMenuItem::separator(),
                &item("settings", "偏好设置…", cmd(Code::Comma)),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &item("quit", &format!("退出{}", crate::APP_NAME), cmd(Code::KeyQ)),
            ],
        )?;
        let file = Submenu::with_items(
            "文件",
            true,
            &[&item("open", "打开批量文件…", cmd(Code::KeyO))],
        )?;
        let window = Submenu::with_items(
            "窗口",
            true,
            &[
                &item("minimize", "最小化", cmd(Code::KeyM)),
                &item("close_window", "关闭窗口", cmd(Code::KeyW)),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::maximize(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::fullscreen(None),
            ],
        )?;
        let help = Submenu::with_items("帮助", true, &[&item("help", "使用说明", None)])?;
        let menu = Menu::with_items(&[&app, &file, &window, &help])?;
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
        help.set_as_help_menu_for_nsapp();

        let (tx, rx) = channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let a = match e.id().0.as_str() {
                "about" => Action::About,
                "settings" => Action::Settings,
                "open" => Action::OpenBatchFile,
                "help" => Action::Help,
                "quit" => Action::Quit,
                "minimize" => Action::Minimize,
                "close_window" => Action::CloseWindow,
                _ => return,
            };
            let _ = tx.send(a);
            ctx.request_repaint();
        }));
        Ok(NativeMenu { _menu: menu, rx })
    }

    /// 取出待处理的菜单操作。
    pub fn poll(&self) -> Vec<Action> {
        self.rx.try_iter().collect()
    }
}
