use std::time::Duration;

use iced::Subscription;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::app::Message;

pub struct AppMenu {
    _menu: Menu,
    pub export_item: MenuItem,
}

pub fn build() -> AppMenu {
    let menu = Menu::new();

    // macOS uses the first submenu as the app menu (title replaced with app name)
    let app_menu = Submenu::with_items(
        "crema",
        true,
        &[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    )
    .expect("failed to create app menu");

    let export_item = MenuItem::with_id(
        "export",
        "Export...",
        false,
        Some(Accelerator::new(Some(Modifiers::META), Code::KeyE)),
    );

    let file_menu = Submenu::with_id_and_items(
        "file",
        "File",
        true,
        &[
            &MenuItem::with_id(
                "import",
                "Import...",
                true,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyI)),
            ),
            &export_item,
        ],
    )
    .expect("failed to create File menu");

    menu.append_items(&[&app_menu, &file_menu])
        .expect("failed to append menus");

    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();

    AppMenu {
        _menu: menu,
        export_item,
    }
}

pub fn subscription() -> Subscription<Message> {
    iced::time::every(Duration::from_millis(50)).map(|_| match MenuEvent::receiver().try_recv() {
        Ok(event) if event.id == "import" => Message::Import,
        Ok(event) if event.id == "export" => Message::Export,
        _ => Message::Noop,
    })
}
