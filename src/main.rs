mod app;
mod icon;
mod menu;
mod views;
mod widgets;

use tracing_subscriber::EnvFilter;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    iced::application(app::App::new, app::App::update, app::App::view)
        .subscription(app::App::subscription)
        .title(app::App::title)
        .theme(app::App::theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1400.0, 900.0),
            icon: Some(icon::iced_icon()),
            ..Default::default()
        })
        .antialiasing(true)
        .run()
}
