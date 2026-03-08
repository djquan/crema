use std::time::Duration;

use iced::Subscription;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::app::Message;

pub struct AppMenu {
    _menu: Menu,
    pub export_item: MenuItem,
    pub save_sidecar_item: MenuItem,
    pub load_sidecar_item: MenuItem,
    pub undo_item: MenuItem,
    pub redo_item: MenuItem,
    pub paste_edits_item: MenuItem,
}

pub fn build() -> AppMenu {
    let menu = Menu::new();

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

    let save_sidecar_item = MenuItem::with_id(
        "save_sidecar",
        "Save Sidecar",
        false,
        Some(Accelerator::new(Some(Modifiers::META), Code::KeyS)),
    );

    let load_sidecar_item = MenuItem::with_id("load_sidecar", "Load Sidecar", false, None);

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
            &PredefinedMenuItem::separator(),
            &save_sidecar_item,
            &load_sidecar_item,
        ],
    )
    .expect("failed to create File menu");

    let undo_item = MenuItem::with_id(
        "undo",
        "Undo",
        false,
        Some(Accelerator::new(Some(Modifiers::META), Code::KeyZ)),
    );

    let redo_item = MenuItem::with_id(
        "redo",
        "Redo",
        false,
        Some(Accelerator::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::KeyZ,
        )),
    );

    let copy_edits_item = MenuItem::with_id(
        "copy_edits",
        "Copy Edits",
        true,
        Some(Accelerator::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::KeyC,
        )),
    );

    let paste_edits_item = MenuItem::with_id(
        "paste_edits",
        "Paste Edits",
        false,
        Some(Accelerator::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::KeyV,
        )),
    );

    let edit_menu = Submenu::with_id_and_items(
        "edit",
        "Edit",
        true,
        &[
            &undo_item,
            &redo_item,
            &PredefinedMenuItem::separator(),
            &copy_edits_item,
            &paste_edits_item,
        ],
    )
    .expect("failed to create Edit menu");

    menu.append_items(&[&app_menu, &file_menu, &edit_menu])
        .expect("failed to append menus");

    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();

    AppMenu {
        _menu: menu,
        export_item,
        save_sidecar_item,
        load_sidecar_item,
        undo_item,
        redo_item,
        paste_edits_item,
    }
}

pub fn subscription() -> Subscription<Message> {
    iced::time::every(Duration::from_millis(50)).map(|_| match MenuEvent::receiver().try_recv() {
        Ok(event) if event.id == "import" => Message::Import,
        Ok(event) if event.id == "export" => Message::Export,
        Ok(event) if event.id == "save_sidecar" => Message::SaveSidecar,
        Ok(event) if event.id == "load_sidecar" => Message::LoadSidecar,
        Ok(event) if event.id == "undo" => Message::Undo,
        Ok(event) if event.id == "redo" => Message::Redo,
        Ok(event) if event.id == "copy_edits" => Message::CopyEdits,
        Ok(event) if event.id == "paste_edits" => Message::PasteEdits,
        _ => Message::Noop,
    })
}
