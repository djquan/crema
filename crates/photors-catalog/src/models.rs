use serde::{Deserialize, Serialize};

pub type PhotoId = i64;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Photo {
    pub id: PhotoId,
    pub file_path: String,
    pub file_hash: String,
    pub file_size: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub focal_length: Option<f64>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub iso: Option<u32>,
    pub date_taken: Option<String>,
    pub imported_at: String,
    pub thumbnail_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditRecord {
    pub id: i64,
    pub photo_id: PhotoId,
    pub exposure: f32,
    pub wb_temp: f32,
    pub wb_tint: f32,
    pub crop_x: f32,
    pub crop_y: f32,
    pub crop_w: f32,
    pub crop_h: f32,
    pub updated_at: String,
}

impl EditRecord {
    pub fn to_edit_params(&self) -> photors_core::image_buf::EditParams {
        photors_core::image_buf::EditParams {
            exposure: self.exposure,
            wb_temp: self.wb_temp,
            wb_tint: self.wb_tint,
            crop_x: self.crop_x,
            crop_y: self.crop_y,
            crop_w: self.crop_w,
            crop_h: self.crop_h,
        }
    }
}
