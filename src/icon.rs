const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

pub fn iced_icon() -> iced::window::Icon {
    iced::window::icon::from_file_data(ICON_PNG, None).expect("embedded icon.png is valid")
}

#[cfg(target_os = "macos")]
pub fn set_dock_icon() {
    use objc2::AllocAnyThread;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};

    let mtm = MainThreadMarker::new().expect("set_dock_icon must be called on the main thread");
    let data = NSData::with_bytes(ICON_PNG);
    let image = NSImage::initWithData(NSImage::alloc(), &data).expect("valid PNG for NSImage");
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setApplicationIconImage(Some(&image)) };
}
