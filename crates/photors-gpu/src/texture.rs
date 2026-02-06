use anyhow::Result;
use photors_core::image_buf::ImageBuf;

/// A GPU texture holding RGBA f32 image data.
pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl GpuTexture {
    /// Upload an ImageBuf to a GPU texture (Rgba32Float format).
    pub fn from_image_buf(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buf: &ImageBuf,
        label: &str,
    ) -> Self {
        let size = wgpu::Extent3d {
            width: buf.width,
            height: buf.height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });

        let rgba = buf.to_rgba_f32();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&rgba),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(buf.width * 4 * 4),
                rows_per_image: Some(buf.height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            texture,
            view,
            width: buf.width,
            height: buf.height,
        }
    }

    /// Create an empty texture for use as a compute shader output.
    pub fn create_storage(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        label: &str,
    ) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            texture,
            view,
            width,
            height,
        }
    }

    /// Read texture data back to CPU as an ImageBuf (blocking).
    pub fn download(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<ImageBuf> {
        let bytes_per_row_unpadded = self.width * 4 * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let bytes_per_row_padded = bytes_per_row_unpadded.div_ceil(align) * align;

        let buffer_size = (bytes_per_row_padded * self.height) as u64;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("texture_download_staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("texture_download"),
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row_padded),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        queue.submit(std::iter::once(encoder.finish()));

        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .map_err(|e| anyhow::anyhow!("GPU poll error: {e}"))?;
        receiver
            .recv()
            .map_err(|_| anyhow::anyhow!("buffer map cancelled"))??;

        let mapped = staging.slice(..).get_mapped_range();
        let mut rgb_data = Vec::with_capacity((self.width * self.height * 3) as usize);

        for row in 0..self.height {
            let row_offset = (row * bytes_per_row_padded) as usize;
            let row_bytes = &mapped[row_offset..row_offset + (self.width * 4 * 4) as usize];
            let row_floats: &[f32] = bytemuck::cast_slice(row_bytes);

            for pixel in row_floats.chunks_exact(4) {
                rgb_data.push(pixel[0]);
                rgb_data.push(pixel[1]);
                rgb_data.push(pixel[2]);
            }
        }

        drop(mapped);
        staging.unmap();

        ImageBuf::from_data(self.width, self.height, rgb_data)
    }
}
