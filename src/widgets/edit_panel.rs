use iced::widget::{Space, button, column, container, row, slider, text};
use iced::{Element, Length};

use crema_core::image_buf::EditParams;

use crate::app::Message;

pub fn view(params: &EditParams, has_preview: bool) -> Element<'_, Message> {
    let auto_btn = if has_preview {
        button("Auto").on_press(Message::AutoEnhance)
    } else {
        button("Auto")
    };
    let reset_btn = button("Reset").on_press(Message::ResetEdits);
    let action_row = row![auto_btn, Space::new().width(Length::Fill), reset_btn,].spacing(8);

    let light_header = text("Light").size(12);
    let exposure = labeled_slider(
        "Exposure",
        format!("{:+.1} EV", params.exposure),
        -5.0..=5.0,
        params.exposure,
        0.01,
        Message::ExposureChanged,
    );
    let contrast = labeled_slider(
        "Contrast",
        format!("{:.0}", params.contrast),
        -100.0..=100.0,
        params.contrast,
        1.0,
        Message::ContrastChanged,
    );
    let highlights = labeled_slider(
        "Highlights",
        format!("{:.0}", params.highlights),
        -100.0..=100.0,
        params.highlights,
        1.0,
        Message::HighlightsChanged,
    );
    let shadows = labeled_slider(
        "Shadows",
        format!("{:.0}", params.shadows),
        -100.0..=100.0,
        params.shadows,
        1.0,
        Message::ShadowsChanged,
    );
    let blacks = labeled_slider(
        "Blacks",
        format!("{:.0}", params.blacks),
        -100.0..=100.0,
        params.blacks,
        1.0,
        Message::BlacksChanged,
    );

    let color_header = text("Color").size(12);
    let wb_temp = labeled_slider(
        "Temperature",
        format!("{:.0} K", params.wb_temp),
        2000.0..=25000.0,
        params.wb_temp,
        10.0,
        Message::WbTempChanged,
    );
    let wb_tint = labeled_slider(
        "Tint",
        format!("{:+.0}", params.wb_tint),
        -150.0..=150.0,
        params.wb_tint,
        1.0,
        Message::WbTintChanged,
    );
    let vibrance = labeled_slider(
        "Vibrance",
        format!("{:.0}", params.vibrance),
        -100.0..=100.0,
        params.vibrance,
        1.0,
        Message::VibranceChanged,
    );
    let saturation = labeled_slider(
        "Saturation",
        format!("{:.0}", params.saturation),
        -100.0..=100.0,
        params.saturation,
        1.0,
        Message::SaturationChanged,
    );

    container(
        column![
            text("Develop").size(16),
            action_row,
            light_header,
            exposure,
            contrast,
            highlights,
            shadows,
            blacks,
            color_header,
            wb_temp,
            wb_tint,
            vibrance,
            saturation,
        ]
        .spacing(12),
    )
    .padding(10)
    .into()
}

fn labeled_slider<'a>(
    label: &'static str,
    value_text: String,
    range: std::ops::RangeInclusive<f32>,
    value: f32,
    step: f32,
    on_change: impl Fn(f32) -> Message + 'a,
) -> Element<'a, Message> {
    column![
        row![text(label).size(12), text(value_text).size(12),].spacing(8),
        slider(range, value, on_change).step(step),
    ]
    .spacing(4)
    .into()
}
