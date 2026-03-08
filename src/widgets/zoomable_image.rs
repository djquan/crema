use iced::mouse;
use iced::widget::canvas::{self, Action, Event, Frame, Path, Stroke};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::Message;

const MIN_ZOOM: f32 = 1.0;
const ZOOM_STEP: f32 = 1.15;
const HANDLE_RADIUS: f32 = 6.0;
const GRAB_RADIUS: f32 = 14.0;

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

#[derive(Clone, Debug)]
pub struct CropOverlay {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub aspect: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
enum CropHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Interior,
}

struct ZoomableImage {
    handle: iced::widget::image::Handle,
    image_size: Size,
    zoom_state: ZoomState,
    crop: Option<CropOverlay>,
}

#[derive(Default)]
pub struct CanvasState {
    dragging: bool,
    last_cursor: Option<Point>,
    crop_handle: Option<CropHandle>,
}

impl ZoomableImage {
    fn image_dest(&self, bounds: Rectangle) -> Rectangle {
        let vw = bounds.width;
        let vh = bounds.height;
        let iw = self.image_size.width;
        let ih = self.image_size.height;

        let fit_scale = (vw / iw).min(vh / ih);
        let render_scale = fit_scale * self.zoom_state.zoom;
        let rendered_w = iw * render_scale;
        let rendered_h = ih * render_scale;
        let base_x = (vw - rendered_w) / 2.0;
        let base_y = (vh - rendered_h) / 2.0;

        Rectangle {
            x: base_x + self.zoom_state.pan.x,
            y: base_y + self.zoom_state.pan.y,
            width: rendered_w,
            height: rendered_h,
        }
    }

    fn crop_screen_rect(&self, dest: &Rectangle, crop: &CropOverlay) -> Rectangle {
        Rectangle {
            x: dest.x + crop.x * dest.width,
            y: dest.y + crop.y * dest.height,
            width: crop.w * dest.width,
            height: crop.h * dest.height,
        }
    }

    fn hit_test_crop(
        &self,
        cursor: Point,
        dest: &Rectangle,
        crop: &CropOverlay,
    ) -> Option<CropHandle> {
        let cr = self.crop_screen_rect(dest, crop);
        let corners = [
            (Point::new(cr.x, cr.y), CropHandle::TopLeft),
            (Point::new(cr.x + cr.width, cr.y), CropHandle::TopRight),
            (Point::new(cr.x, cr.y + cr.height), CropHandle::BottomLeft),
            (
                Point::new(cr.x + cr.width, cr.y + cr.height),
                CropHandle::BottomRight,
            ),
        ];

        for (pt, handle) in corners {
            if (cursor.x - pt.x).abs() < GRAB_RADIUS && (cursor.y - pt.y).abs() < GRAB_RADIUS {
                return Some(handle);
            }
        }

        if cr.contains(cursor) {
            return Some(CropHandle::Interior);
        }

        None
    }

    fn update_crop(
        &self,
        state: &mut CanvasState,
        event: &Event,
        bounds: Rectangle,
        cursor_pos: Point,
    ) -> Option<Action<Message>> {
        let dest = self.image_dest(bounds);
        let crop = self.crop.as_ref()?;

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(handle) = self.hit_test_crop(cursor_pos, &dest, crop) {
                    state.crop_handle = Some(handle);
                    state.last_cursor = Some(cursor_pos);
                    Some(Action::capture())
                } else {
                    None
                }
            }

            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let handle = state.crop_handle?;
                let last = state.last_cursor?;

                let dx = (cursor_pos.x - last.x) / dest.width;
                let dy = (cursor_pos.y - last.y) / dest.height;
                state.last_cursor = Some(cursor_pos);

                let (mut nx, mut ny, mut nw, mut nh) = (crop.x, crop.y, crop.w, crop.h);

                match handle {
                    CropHandle::Interior => {
                        nx = (nx + dx).clamp(0.0, 1.0 - nw);
                        ny = (ny + dy).clamp(0.0, 1.0 - nh);
                    }
                    CropHandle::TopLeft => {
                        let new_x = (nx + dx).clamp(0.0, nx + nw - 0.05);
                        let new_y = (ny + dy).clamp(0.0, ny + nh - 0.05);
                        nw += nx - new_x;
                        nh += ny - new_y;
                        nx = new_x;
                        ny = new_y;
                    }
                    CropHandle::TopRight => {
                        nw = (nw + dx).clamp(0.05, 1.0 - nx);
                        let new_y = (ny + dy).clamp(0.0, ny + nh - 0.05);
                        nh += ny - new_y;
                        ny = new_y;
                    }
                    CropHandle::BottomLeft => {
                        let new_x = (nx + dx).clamp(0.0, nx + nw - 0.05);
                        nw += nx - new_x;
                        nx = new_x;
                        nh = (nh + dy).clamp(0.05, 1.0 - ny);
                    }
                    CropHandle::BottomRight => {
                        nw = (nw + dx).clamp(0.05, 1.0 - nx);
                        nh = (nh + dy).clamp(0.05, 1.0 - ny);
                    }
                }

                if let Some(aspect) = crop.aspect {
                    let iw = self.image_size.width;
                    let ih = self.image_size.height;
                    let pw = nw * iw;
                    let ph = nh * ih;
                    let current = pw / ph;

                    if current > aspect {
                        let corrected_w = ph * aspect / iw;
                        match handle {
                            CropHandle::TopLeft | CropHandle::BottomLeft => {
                                nx += nw - corrected_w;
                            }
                            _ => {}
                        }
                        nw = corrected_w;
                    } else {
                        let corrected_h = pw / aspect / ih;
                        match handle {
                            CropHandle::TopLeft | CropHandle::TopRight => {
                                ny += nh - corrected_h;
                            }
                            _ => {}
                        }
                        nh = corrected_h;
                    }
                }

                nx = nx.clamp(0.0, 1.0 - nw);
                ny = ny.clamp(0.0, 1.0 - nh);

                Some(Action::publish(Message::UpdateCrop(nx, ny, nw, nh)).and_capture())
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.crop_handle.is_some() {
                    state.crop_handle = None;
                    state.last_cursor = None;
                    Some(Action::capture())
                } else {
                    None
                }
            }

            _ => None,
        }
    }

    fn draw_crop_overlay(&self, frame: &mut Frame, dest: Rectangle) {
        let Some(crop) = &self.crop else {
            return;
        };

        let cr = self.crop_screen_rect(&dest, crop);
        let dim = Color::from_rgba(0.0, 0.0, 0.0, 0.55);

        // Dim regions outside crop rect
        // Top
        if cr.y > dest.y {
            frame.fill_rectangle(
                Point::new(dest.x, dest.y),
                Size::new(dest.width, cr.y - dest.y),
                dim,
            );
        }
        // Bottom
        let bottom = cr.y + cr.height;
        let dest_bottom = dest.y + dest.height;
        if bottom < dest_bottom {
            frame.fill_rectangle(
                Point::new(dest.x, bottom),
                Size::new(dest.width, dest_bottom - bottom),
                dim,
            );
        }
        // Left
        if cr.x > dest.x {
            frame.fill_rectangle(
                Point::new(dest.x, cr.y),
                Size::new(cr.x - dest.x, cr.height),
                dim,
            );
        }
        // Right
        let right = cr.x + cr.width;
        let dest_right = dest.x + dest.width;
        if right < dest_right {
            frame.fill_rectangle(
                Point::new(right, cr.y),
                Size::new(dest_right - right, cr.height),
                dim,
            );
        }

        // Crop border
        let border_stroke = Stroke::default().with_width(1.5).with_color(Color::WHITE);
        frame.stroke_rectangle(Point::new(cr.x, cr.y), cr.size(), border_stroke);

        // Rule of thirds
        let grid_stroke = Stroke::default()
            .with_width(0.5)
            .with_color(Color::from_rgba(1.0, 1.0, 1.0, 0.4));
        for i in 1..3 {
            let frac = i as f32 / 3.0;
            let hx = cr.x + cr.width * frac;
            let vy = cr.y + cr.height * frac;
            frame.stroke(
                &Path::line(Point::new(hx, cr.y), Point::new(hx, cr.y + cr.height)),
                grid_stroke,
            );
            frame.stroke(
                &Path::line(Point::new(cr.x, vy), Point::new(cr.x + cr.width, vy)),
                grid_stroke,
            );
        }

        // Corner handles
        let handle_color = Color::WHITE;
        let hs = HANDLE_RADIUS;
        let corners = [
            Point::new(cr.x - hs, cr.y - hs),
            Point::new(cr.x + cr.width - hs, cr.y - hs),
            Point::new(cr.x - hs, cr.y + cr.height - hs),
            Point::new(cr.x + cr.width - hs, cr.y + cr.height - hs),
        ];
        for pt in corners {
            frame.fill_rectangle(pt, Size::new(hs * 2.0, hs * 2.0), handle_color);
        }
    }
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
            state.crop_handle = None;
            return None;
        };

        // Scroll wheel always zooms
        if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event {
            let scroll_y = match delta {
                mouse::ScrollDelta::Lines { y, .. } => *y,
                mouse::ScrollDelta::Pixels { y, .. } => *y / 60.0,
            };
            if scroll_y != 0.0 {
                let factor = if scroll_y > 0.0 {
                    ZOOM_STEP
                } else {
                    1.0 / ZOOM_STEP
                };
                return Some(
                    Action::publish(Message::ZoomAtPoint(
                        factor,
                        cursor_pos.x,
                        cursor_pos.y,
                        bounds.width,
                        bounds.height,
                    ))
                    .and_capture(),
                );
            }
            return None;
        }

        // If crop mode, handle crop interactions
        if self.crop.is_some() {
            return self.update_crop(state, event, bounds, cursor_pos);
        }

        // Otherwise, handle zoom/pan
        match event {
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

        if self.image_size.width <= 0.0 || self.image_size.height <= 0.0 {
            return vec![frame.into_geometry()];
        }

        let dest = self.image_dest(bounds);
        let clip = Rectangle {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: bounds.height,
        };

        frame.with_clip(clip, |frame| {
            frame.draw_image(dest, iced::advanced::image::Image::new(&self.handle));
            self.draw_crop_overlay(frame, dest);
        });

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let Some(cursor_pos) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };

        if let Some(crop) = &self.crop {
            if state.crop_handle.is_some() {
                return mouse::Interaction::Grabbing;
            }
            let dest = self.image_dest(bounds);
            return match self.hit_test_crop(cursor_pos, &dest, crop) {
                Some(CropHandle::Interior) => mouse::Interaction::Grab,
                Some(_) => mouse::Interaction::Crosshair,
                None => mouse::Interaction::default(),
            };
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
    crop: Option<CropOverlay>,
) -> Element<'a, Message> {
    iced::widget::canvas(ZoomableImage {
        handle: handle.clone(),
        image_size: Size::new(image_width as f32, image_height as f32),
        zoom_state: zoom_state.clone(),
        crop,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
