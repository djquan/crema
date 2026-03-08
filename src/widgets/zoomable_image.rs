use iced::mouse;
use iced::widget::canvas::{self, Action, Event, Frame};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::Message;

const MIN_ZOOM: f32 = 1.0;
const ZOOM_STEP: f32 = 1.15;

#[derive(Clone, Debug)]
pub struct ZoomState {
    pub zoom: f32,
    pub pan: Vector,
}

impl Default for ZoomState {
    fn default() -> Self {
        Self {
            zoom: MIN_ZOOM,
            pan: Vector::ZERO,
        }
    }
}

impl ZoomState {
    pub fn is_fit(&self) -> bool {
        self.zoom <= MIN_ZOOM
    }

    pub fn zoom_label(&self) -> String {
        if self.is_fit() {
            "Fit".into()
        } else {
            format!("{:.0}%", self.zoom * 100.0)
        }
    }
}

struct ZoomableImage {
    handle: iced::widget::image::Handle,
    image_size: Size,
    zoom_state: ZoomState,
}

#[derive(Default)]
pub struct CanvasState {
    dragging: bool,
    last_cursor: Option<Point>,
}

impl canvas::Program<Message> for ZoomableImage {
    type State = CanvasState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let Some(cursor_pos) = cursor.position_in(bounds) else {
            state.dragging = false;
            state.last_cursor = None;
            return None;
        };

        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let scroll_y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 60.0,
                };
                if scroll_y == 0.0 {
                    return None;
                }

                let factor = if scroll_y > 0.0 {
                    ZOOM_STEP
                } else {
                    1.0 / ZOOM_STEP
                };

                Some(
                    Action::publish(Message::ZoomAtPoint(
                        factor,
                        cursor_pos.x,
                        cursor_pos.y,
                        bounds.width,
                        bounds.height,
                    ))
                    .and_capture(),
                )
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if self.zoom_state.zoom > MIN_ZOOM {
                    state.dragging = true;
                    state.last_cursor = Some(cursor_pos);
                    Some(Action::capture())
                } else {
                    None
                }
            }

            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if state.dragging
                    && let Some(last) = state.last_cursor
                {
                    let dx = cursor_pos.x - last.x;
                    let dy = cursor_pos.y - last.y;
                    state.last_cursor = Some(cursor_pos);
                    return Some(Action::publish(Message::PanDelta(dx, dy)).and_capture());
                }
                None
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.dragging {
                    state.dragging = false;
                    state.last_cursor = None;
                    Some(Action::capture())
                } else {
                    None
                }
            }

            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        let vw = bounds.width;
        let vh = bounds.height;
        let iw = self.image_size.width;
        let ih = self.image_size.height;

        if iw <= 0.0 || ih <= 0.0 {
            return vec![frame.into_geometry()];
        }

        let fit_scale = (vw / iw).min(vh / ih);
        let render_scale = fit_scale * self.zoom_state.zoom;

        let rendered_w = iw * render_scale;
        let rendered_h = ih * render_scale;

        let base_x = (vw - rendered_w) / 2.0;
        let base_y = (vh - rendered_h) / 2.0;
        let draw_x = base_x + self.zoom_state.pan.x;
        let draw_y = base_y + self.zoom_state.pan.y;

        let dest = Rectangle {
            x: draw_x,
            y: draw_y,
            width: rendered_w,
            height: rendered_h,
        };

        let clip = Rectangle {
            x: 0.0,
            y: 0.0,
            width: vw,
            height: vh,
        };

        frame.with_clip(clip, |frame| {
            frame.draw_image(dest, iced::advanced::image::Image::new(&self.handle));
        });

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.position_in(bounds).is_none() {
            return mouse::Interaction::default();
        }
        if state.dragging {
            mouse::Interaction::Grabbing
        } else if self.zoom_state.zoom > MIN_ZOOM {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

pub fn view<'a>(
    handle: &iced::widget::image::Handle,
    image_width: u32,
    image_height: u32,
    zoom_state: &ZoomState,
) -> Element<'a, Message> {
    iced::widget::canvas(ZoomableImage {
        handle: handle.clone(),
        image_size: Size::new(image_width as f32, image_height as f32),
        zoom_state: zoom_state.clone(),
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
