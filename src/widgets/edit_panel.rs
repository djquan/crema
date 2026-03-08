use iced::widget::{Space, button, column, row, slider, text};
use iced::{Color, Element, Length};

use crate::app::{App, EditControl, EditSection, Message, PanelSection, Workspace};
use crate::views::unified::section_card;

const MUTED: Color = Color::from_rgb(0.66, 0.66, 0.69);
const ACTIVE: Color = Color::from_rgb(0.82, 0.86, 0.95);
const ACCENT: Color = Color::from_rgb(0.26, 0.52, 0.94);

pub fn view(app: &App) -> Element<'_, Message> {
    let mut sections = column![
        section_card(
            "Light",
            app.is_panel_open(PanelSection::Light),
            Message::TogglePanelSection(PanelSection::Light),
            Some(Message::ResetSection(EditSection::Light)),
            light_controls(app),
        ),
        section_card(
            "Color",
            app.is_panel_open(PanelSection::Color),
            Message::TogglePanelSection(PanelSection::Color),
            Some(Message::ResetSection(EditSection::Color)),
            color_controls(app),
        ),
    ]
    .spacing(10);

    sections = sections.push(section_card(
        "HSL",
        app.is_panel_open(PanelSection::Hsl),
        Message::TogglePanelSection(PanelSection::Hsl),
        Some(Message::ResetSection(EditSection::Hsl)),
        hsl_controls(app),
    ));

    sections = sections.push(section_card(
        "Split Tone",
        app.is_panel_open(PanelSection::SplitTone),
        Message::TogglePanelSection(PanelSection::SplitTone),
        Some(Message::ResetSection(EditSection::SplitTone)),
        split_tone_controls(app),
    ));

    sections = sections.push(section_card(
        "Detail",
        app.is_panel_open(PanelSection::Detail),
        Message::TogglePanelSection(PanelSection::Detail),
        Some(Message::ResetSection(EditSection::Detail)),
        detail_controls(app),
    ));

    if app.workspace() == Workspace::Develop {
        sections = sections.push(section_card(
            "Crop",
            app.is_panel_open(PanelSection::Crop),
            Message::TogglePanelSection(PanelSection::Crop),
            Some(Message::ResetCrop),
            crop_controls(app),
        ));
    }

    sections.into()
}

fn light_controls(app: &App) -> Element<'_, Message> {
    let params = app.edit_params();

    column![
        control(
            "Exposure",
            format!("{:+.1} EV", params.exposure),
            -5.0..=5.0,
            params.exposure,
            0.01,
            app.is_control_adjusted(EditControl::Exposure),
            Message::ExposureChanged,
            Message::ResetControl(EditControl::Exposure),
        ),
        control(
            "Contrast",
            format!("{:.0}", params.contrast),
            -100.0..=100.0,
            params.contrast,
            1.0,
            app.is_control_adjusted(EditControl::Contrast),
            Message::ContrastChanged,
            Message::ResetControl(EditControl::Contrast),
        ),
        control(
            "Highlights",
            format!("{:.0}", params.highlights),
            -100.0..=100.0,
            params.highlights,
            1.0,
            app.is_control_adjusted(EditControl::Highlights),
            Message::HighlightsChanged,
            Message::ResetControl(EditControl::Highlights),
        ),
        control(
            "Shadows",
            format!("{:.0}", params.shadows),
            -100.0..=100.0,
            params.shadows,
            1.0,
            app.is_control_adjusted(EditControl::Shadows),
            Message::ShadowsChanged,
            Message::ResetControl(EditControl::Shadows),
        ),
        control(
            "Blacks",
            format!("{:.0}", params.blacks),
            -100.0..=100.0,
            params.blacks,
            1.0,
            app.is_control_adjusted(EditControl::Blacks),
            Message::BlacksChanged,
            Message::ResetControl(EditControl::Blacks),
        ),
    ]
    .spacing(10)
    .into()
}

fn color_controls(app: &App) -> Element<'_, Message> {
    let params = app.edit_params();

    column![
        control(
            "Temperature",
            format!("{:.0} K", params.wb_temp),
            2000.0..=25000.0,
            params.wb_temp,
            10.0,
            app.is_control_adjusted(EditControl::WbTemp),
            Message::WbTempChanged,
            Message::ResetControl(EditControl::WbTemp),
        ),
        control(
            "Tint",
            format!("{:+.0}", params.wb_tint),
            -150.0..=150.0,
            params.wb_tint,
            1.0,
            app.is_control_adjusted(EditControl::WbTint),
            Message::WbTintChanged,
            Message::ResetControl(EditControl::WbTint),
        ),
        control(
            "Vibrance",
            format!("{:.0}", params.vibrance),
            -100.0..=100.0,
            params.vibrance,
            1.0,
            app.is_control_adjusted(EditControl::Vibrance),
            Message::VibranceChanged,
            Message::ResetControl(EditControl::Vibrance),
        ),
        control(
            "Saturation",
            format!("{:.0}", params.saturation),
            -100.0..=100.0,
            params.saturation,
            1.0,
            app.is_control_adjusted(EditControl::Saturation),
            Message::SaturationChanged,
            Message::ResetControl(EditControl::Saturation),
        ),
    ]
    .spacing(10)
    .into()
}

fn hsl_controls(app: &App) -> Element<'_, Message> {
    let params = app.edit_params();

    column![
        control(
            "Hue",
            format!("{:+.0}°", params.hsl_hue),
            -180.0..=180.0,
            params.hsl_hue,
            1.0,
            app.is_control_adjusted(EditControl::HslHue),
            Message::HslHueChanged,
            Message::ResetControl(EditControl::HslHue),
        ),
        control(
            "HSL Saturation",
            format!("{:.0}", params.hsl_saturation),
            -100.0..=100.0,
            params.hsl_saturation,
            1.0,
            app.is_control_adjusted(EditControl::HslSaturation),
            Message::HslSaturationChanged,
            Message::ResetControl(EditControl::HslSaturation),
        ),
        control(
            "Lightness",
            format!("{:.0}", params.hsl_lightness),
            -100.0..=100.0,
            params.hsl_lightness,
            1.0,
            app.is_control_adjusted(EditControl::HslLightness),
            Message::HslLightnessChanged,
            Message::ResetControl(EditControl::HslLightness),
        ),
    ]
    .spacing(10)
    .into()
}

fn split_tone_controls(app: &App) -> Element<'_, Message> {
    let params = app.edit_params();

    column![
        control(
            "Shadow Hue",
            format!("{:.0}", params.split_shadow_hue),
            0.0..=360.0,
            params.split_shadow_hue,
            1.0,
            app.is_control_adjusted(EditControl::SplitShadowHue),
            Message::SplitShadowHueChanged,
            Message::ResetControl(EditControl::SplitShadowHue),
        ),
        control(
            "Shadow Saturation",
            format!("{:.0}", params.split_shadow_sat),
            0.0..=100.0,
            params.split_shadow_sat,
            1.0,
            app.is_control_adjusted(EditControl::SplitShadowSat),
            Message::SplitShadowSatChanged,
            Message::ResetControl(EditControl::SplitShadowSat),
        ),
        control(
            "Highlight Hue",
            format!("{:.0}", params.split_highlight_hue),
            0.0..=360.0,
            params.split_highlight_hue,
            1.0,
            app.is_control_adjusted(EditControl::SplitHighlightHue),
            Message::SplitHighlightHueChanged,
            Message::ResetControl(EditControl::SplitHighlightHue),
        ),
        control(
            "Highlight Saturation",
            format!("{:.0}", params.split_highlight_sat),
            0.0..=100.0,
            params.split_highlight_sat,
            1.0,
            app.is_control_adjusted(EditControl::SplitHighlightSat),
            Message::SplitHighlightSatChanged,
            Message::ResetControl(EditControl::SplitHighlightSat),
        ),
        control(
            "Balance",
            format!("{:+.0}", params.split_balance),
            -100.0..=100.0,
            params.split_balance,
            1.0,
            app.is_control_adjusted(EditControl::SplitBalance),
            Message::SplitBalanceChanged,
            Message::ResetControl(EditControl::SplitBalance),
        ),
    ]
    .spacing(10)
    .into()
}

fn detail_controls(app: &App) -> Element<'_, Message> {
    let params = app.edit_params();

    column![
        control(
            "Amount",
            format!("{:.0}", params.sharpen_amount),
            0.0..=150.0,
            params.sharpen_amount,
            1.0,
            app.is_control_adjusted(EditControl::SharpenAmount),
            Message::SharpenAmountChanged,
            Message::ResetControl(EditControl::SharpenAmount),
        ),
        control(
            "Radius",
            format!("{:.1}", params.sharpen_radius),
            0.5..=3.0,
            params.sharpen_radius,
            0.1,
            app.is_control_adjusted(EditControl::SharpenRadius),
            Message::SharpenRadiusChanged,
            Message::ResetControl(EditControl::SharpenRadius),
        ),
    ]
    .spacing(10)
    .into()
}

fn crop_controls(app: &App) -> Element<'_, Message> {
    let params = app.edit_params();
    let crop_active = app.crop_mode();
    let current_aspect = app.crop_aspect();

    let toggle_label = if crop_active { "Done" } else { "Crop" };
    let toggle_btn = button(toggle_label)
        .on_press(Message::ToggleCropMode)
        .padding([6, 14])
        .style(move |theme: &iced::Theme, status| {
            if crop_active {
                use iced::{Background, Border, Shadow};
                iced::widget::button::Style {
                    background: Some(Background::Color(ACCENT)),
                    text_color: Color::WHITE,
                    border: Border {
                        color: ACCENT,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    shadow: Shadow::default(),
                    snap: false,
                }
            } else {
                iced::widget::button::secondary(theme, status)
            }
        });

    let aspects: &[(&str, Option<f32>)] = &[
        ("Free", None),
        ("1:1", Some(1.0)),
        ("4:3", Some(4.0 / 3.0)),
        ("3:2", Some(3.0 / 2.0)),
        ("16:9", Some(16.0 / 9.0)),
    ];

    let mut aspect_row = row![].spacing(4);
    for &(label, ratio) in aspects {
        let is_active = current_aspect == ratio;
        let label_color = if is_active { ACCENT } else { MUTED };
        aspect_row = aspect_row.push(
            button(text(label).size(11).color(label_color))
                .on_press(Message::SetCropAspect(ratio))
                .padding([3, 6])
                .style(button::text),
        );
    }

    column![
        row![toggle_btn, Space::new().width(Length::Fill),]
            .align_y(iced::Alignment::Center)
            .spacing(8),
        aspect_row,
        control(
            "Straighten",
            format!("{:+.1}°", params.rotation),
            -45.0..=45.0,
            params.rotation,
            0.1,
            app.is_control_adjusted(EditControl::Rotation),
            Message::RotationChanged,
            Message::ResetControl(EditControl::Rotation),
        ),
    ]
    .spacing(8)
    .into()
}

#[allow(clippy::too_many_arguments)]
fn control<'a>(
    label: &'static str,
    value_text: String,
    range: std::ops::RangeInclusive<f32>,
    value: f32,
    step: f32,
    adjusted: bool,
    on_change: impl Fn(f32) -> Message + 'a,
    reset: Message,
) -> Element<'a, Message> {
    let label_color = if adjusted { ACTIVE } else { MUTED };

    column![
        row![
            text(label).size(12).color(label_color),
            Space::new().width(Length::Fill),
            text(value_text).size(12).color(label_color),
            Space::new().width(8),
            button("Reset")
                .on_press(reset)
                .padding([2, 6])
                .style(button::text),
        ]
        .align_y(iced::Alignment::Center),
        slider(range, value, on_change).step(step),
    ]
    .spacing(5)
    .into()
}
