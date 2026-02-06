use iced::mouse;
use iced::widget::canvas::{self, Frame, Path};
use iced::{Color, Element, Length, Rectangle, Renderer, Theme};

use crate::app::Message;

const HISTOGRAM_HEIGHT: f32 = 120.0;
const NUM_BINS: usize = 256;

#[derive(Clone, Debug)]
pub struct HistogramData {
    pub r: [u32; NUM_BINS],
    pub g: [u32; NUM_BINS],
    pub b: [u32; NUM_BINS],
    pub max_count: u32,
}

impl HistogramData {
    pub fn from_rgba_u8(pixels: &[u8]) -> Self {
        let mut r = [0u32; NUM_BINS];
        let mut g = [0u32; NUM_BINS];
        let mut b = [0u32; NUM_BINS];

        for pixel in pixels.chunks_exact(4) {
            r[pixel[0] as usize] += 1;
            g[pixel[1] as usize] += 1;
            b[pixel[2] as usize] += 1;
        }

        let max_count = r
            .iter()
            .chain(g.iter())
            .chain(b.iter())
            .copied()
            .max()
            .unwrap_or(0)
            .max(1);

        Self { r, g, b, max_count }
    }
}

struct HistogramCanvas {
    data: Option<HistogramData>,
}

impl<Message> canvas::Program<Message> for HistogramCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        // Dark background
        frame.fill_rectangle(
            iced::Point::ORIGIN,
            bounds.size(),
            Color::from_rgb(0.1, 0.1, 0.1),
        );

        let Some(data) = &self.data else {
            return vec![frame.into_geometry()];
        };

        if data.max_count == 0 {
            return vec![frame.into_geometry()];
        }

        let w = bounds.width;
        let h = bounds.height;
        let bin_width = w / NUM_BINS as f32;
        let max = (data.max_count as f32).ln_1p();

        // Draw each channel as a semi-transparent filled area
        for (bins, color) in [
            (&data.r, Color::from_rgba(1.0, 0.0, 0.0, 0.4)),
            (&data.g, Color::from_rgba(0.0, 1.0, 0.0, 0.4)),
            (&data.b, Color::from_rgba(0.0, 0.4, 1.0, 0.4)),
        ] {
            let path = Path::new(|builder| {
                builder.move_to(iced::Point::new(0.0, h));
                for (i, &count) in bins.iter().enumerate() {
                    let x = i as f32 * bin_width;
                    let normalized = (count as f32).ln_1p() / max;
                    let bar_h = normalized * h;
                    builder.line_to(iced::Point::new(x, h - bar_h));
                }
                builder.line_to(iced::Point::new(w, h));
                builder.close();
            });
            frame.fill(&path, color);
        }

        vec![frame.into_geometry()]
    }
}

pub fn view<'a>(histogram: Option<&HistogramData>) -> Element<'a, Message> {
    iced::widget::canvas(HistogramCanvas {
        data: histogram.cloned(),
    })
    .width(Length::Fill)
    .height(HISTOGRAM_HEIGHT)
    .into()
}

/// Compute histogram from raw RGBA u8 pixel data. Call this from the
/// processing pipeline when image data is available.
pub fn compute_histogram(rgba_pixels: &[u8]) -> HistogramData {
    HistogramData::from_rgba_u8(rgba_pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_from_solid_color() {
        // 2x2 image, all red (255,0,0,255)
        let pixels = vec![
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ];
        let hist = HistogramData::from_rgba_u8(&pixels);
        assert_eq!(hist.r[255], 4);
        assert_eq!(hist.r[0], 0);
        assert_eq!(hist.g[0], 4);
        assert_eq!(hist.b[0], 4);
        assert_eq!(hist.max_count, 4);
    }

    #[test]
    fn histogram_empty_image() {
        let pixels: Vec<u8> = Vec::new();
        let hist = HistogramData::from_rgba_u8(&pixels);
        assert_eq!(hist.max_count, 1); // clamped to 1 to avoid div-by-zero
    }

    #[test]
    fn histogram_gradient() {
        // 256 pixels, each with incrementing R value
        let mut pixels = Vec::with_capacity(256 * 4);
        for i in 0..=255u8 {
            pixels.extend_from_slice(&[i, 128, 0, 255]);
        }
        let hist = HistogramData::from_rgba_u8(&pixels);
        for i in 0..256 {
            assert_eq!(hist.r[i], 1);
        }
        assert_eq!(hist.g[128], 256);
        assert_eq!(hist.b[0], 256);
    }
}
