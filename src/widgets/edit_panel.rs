use iced::widget::{column, container, row, slider, text};
use iced::Element;

use photors_core::image_buf::EditParams;

use crate::app::Message;

pub fn view(params: &EditParams) -> Element<'_, Message> {
    let exposure = labeled_slider(
        "Exposure",
        format!("{:+.1} EV", params.exposure),
        -5.0..=5.0,
        params.exposure,
        Message::ExposureChanged,
    );

    let wb_temp = labeled_slider(
        "Temperature",
        format!("{:.0} K", params.wb_temp),
        2000.0..=12000.0,
        params.wb_temp,
        Message::WbTempChanged,
    );

    let wb_tint = labeled_slider(
        "Tint",
        format!("{:+.0}", params.wb_tint),
        -100.0..=100.0,
        params.wb_tint,
        Message::WbTintChanged,
    );

    container(
        column![text("Develop").size(16), exposure, wb_temp, wb_tint,].spacing(12),
    )
    .padding(10)
    .into()
}

fn labeled_slider<'a>(
    label: &'static str,
    value_text: String,
    range: std::ops::RangeInclusive<f32>,
    value: f32,
    on_change: impl Fn(f32) -> Message + 'a,
) -> Element<'a, Message> {
    column![
        row![text(label).size(12), text(value_text).size(12),].spacing(8),
        slider(range, value, on_change).step(0.01),
    ]
    .spacing(4)
    .into()
}
