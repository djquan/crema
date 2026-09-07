use crate::PreviewPixels;
use crema_core::edit::EditRecipe;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedSdr {
    width: u32,
    height: u32,
    rgba8: Vec<u8>,
}

impl RenderedSdr {
    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }
}

pub fn render_exposure_srgb8(input: &PreviewPixels, recipe: &EditRecipe) -> RenderedSdr {
    if recipe.exposure().value() == 0 {
        return RenderedSdr {
            width: input.width(),
            height: input.height(),
            rgba8: input.rgba8().to_vec(),
        };
    }

    let table = exposure_table(recipe);
    let mut rgba8 = Vec::with_capacity(input.rgba8().len());
    for pixel in input.rgba8().as_chunks::<4>().0 {
        rgba8.extend_from_slice(&[
            table[usize::from(pixel[0])],
            table[usize::from(pixel[1])],
            table[usize::from(pixel[2])],
            pixel[3],
        ]);
    }
    RenderedSdr {
        width: input.width(),
        height: input.height(),
        rgba8,
    }
}

fn exposure_table(recipe: &EditRecipe) -> [u8; 256] {
    let multiplier = 2.0_f64.powf(f64::from(recipe.exposure().value()) / 100.0);
    std::array::from_fn(|channel| {
        let encoded = channel as f64 / 255.0;
        let linear = if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        };
        let adjusted = (linear * multiplier).clamp(0.0, 1.0);
        let encoded = if adjusted <= 0.003_130_8 {
            adjusted * 12.92
        } else {
            1.055 * adjusted.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    })
}
