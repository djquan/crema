use anyhow::Result;
use tracing::debug;

use crema_core::image_buf::EditParams;
use crema_core::pipeline::modules::wb_matrix;

use crate::context::GpuContext;
use crate::shader::ShaderManager;
use crate::texture::GpuTexture;

const WORKGROUP_SIZE: u32 = 16;

pub struct GpuPipeline {
    shaders: ShaderManager,
    image_params_bgl: wgpu::BindGroupLayout,
    tone_curve_bgl: wgpu::BindGroupLayout,
}

impl GpuPipeline {
    pub fn new(ctx: &GpuContext) -> Self {
        let mut shaders = ShaderManager::new();

        let shader_sources: &[(&str, &str)] = &[
            (
                "white_balance",
                include_str!("../shaders/white_balance.wgsl"),
            ),
            ("exposure", include_str!("../shaders/exposure.wgsl")),
            ("tone_curve", include_str!("../shaders/tone_curve.wgsl")),
            ("vibrance", include_str!("../shaders/vibrance.wgsl")),
            ("saturation", include_str!("../shaders/saturation.wgsl")),
            ("hsl", include_str!("../shaders/hsl.wgsl")),
            ("crop", include_str!("../shaders/crop.wgsl")),
        ];
        for &(name, source) in shader_sources {
            shaders.load_shader(&ctx.device, name, source);
        }

        let image_params_bgl = create_image_params_layout(&ctx.device);
        let tone_curve_bgl = create_tone_curve_layout(&ctx.device);

        Self {
            shaders,
            image_params_bgl,
            tone_curve_bgl,
        }
    }

    /// Run the full GPU pipeline:
    /// WB -> Exposure -> ToneCurve -> Vibrance -> Saturation -> HSL -> Crop
    ///
    /// Sharpening is not included (falls back to CPU for that stage).
    pub fn process(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        let mut current = self.apply_white_balance(ctx, input, params)?;
        current = self.apply_exposure(ctx, &current, params)?;
        current = self.apply_tone_curve(ctx, &current, params)?;
        current = self.apply_vibrance(ctx, &current, params)?;
        current = self.apply_saturation(ctx, &current, params)?;
        current = self.apply_hsl(ctx, &current, params)?;
        // Note: sharpening skipped on GPU (done on CPU after download)
        current = self.apply_crop(ctx, &current, params)?;
        Ok(current)
    }

    fn dispatch_simple(
        &mut self,
        ctx: &GpuContext,
        name: &str,
        input: &GpuTexture,
        output: &GpuTexture,
        params_data: &[f32],
    ) -> Result<()> {
        let params_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{name}_params")),
            size: (params_data.len() * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue
            .write_buffer(&params_buf, 0, bytemuck::cast_slice(params_data));

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{name}_bg")),
            layout: &self.image_params_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&output.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let pipeline =
            self.shaders
                .get_or_create_pipeline(&ctx.device, name, &self.image_params_bgl)?;

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(&format!("{name}_encoder")),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(&format!("{name}_pass")),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                input.width.div_ceil(WORKGROUP_SIZE),
                input.height.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }

        ctx.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    fn apply_white_balance(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        debug!(
            temp = params.wb_temp,
            tint = params.wb_tint,
            "GPU white balance"
        );
        let output = GpuTexture::create_storage(&ctx.device, input.width, input.height, "wb_out");
        let matrix = wb_matrix(params.wb_temp, params.wb_tint);
        self.dispatch_simple(
            ctx,
            "white_balance",
            input,
            &output,
            &[
                matrix[0], matrix[1], matrix[2], 0.0, matrix[3], matrix[4], matrix[5], 0.0,
                matrix[6], matrix[7], matrix[8], 0.0,
            ],
        )?;
        Ok(output)
    }

    fn apply_exposure(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        debug!(exposure = params.exposure, "GPU exposure");
        let output = GpuTexture::create_storage(&ctx.device, input.width, input.height, "exp_out");
        let multiplier = 2.0_f32.powf(params.exposure);
        self.dispatch_simple(
            ctx,
            "exposure",
            input,
            &output,
            &[multiplier, 0.0, 0.0, 0.0],
        )?;
        Ok(output)
    }

    fn apply_tone_curve(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        if params.contrast == 0.0
            && params.highlights == 0.0
            && params.shadows == 0.0
            && params.blacks == 0.0
        {
            return self.passthrough(ctx, input);
        }

        debug!("GPU tone curve");
        let output = GpuTexture::create_storage(&ctx.device, input.width, input.height, "tc_out");

        // Build the LUT on CPU (same as CPU pipeline) and upload as storage buffer
        let lut = crema_core::pipeline::tone_curve_lut(params);
        let lut_size = lut.len() as u32;
        let lut_top = lut[lut.len() - 1];
        let lut_slope = (lut[lut.len() - 1] - lut[lut.len() - 2]) * (lut_size - 1) as f32;

        let lut_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tc_lut"),
            size: (lut.len() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue
            .write_buffer(&lut_buf, 0, bytemuck::cast_slice(&lut));

        let params_data: [f32; 4] = [f32::from_bits(lut_size), lut_top, lut_slope, 0.0];
        let params_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tc_params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue
            .write_buffer(&params_buf, 0, bytemuck::cast_slice(&params_data));

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tc_bg"),
            layout: &self.tone_curve_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&output.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: lut_buf.as_entire_binding(),
                },
            ],
        });

        let pipeline =
            self.shaders
                .get_or_create_pipeline(&ctx.device, "tone_curve", &self.tone_curve_bgl)?;

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tc_encoder"),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("tc_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                input.width.div_ceil(WORKGROUP_SIZE),
                input.height.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }

        ctx.queue.submit(std::iter::once(encoder.finish()));
        Ok(output)
    }

    fn apply_vibrance(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        if params.vibrance == 0.0 {
            return self.passthrough(ctx, input);
        }
        debug!(vibrance = params.vibrance, "GPU vibrance");
        let output = GpuTexture::create_storage(&ctx.device, input.width, input.height, "vib_out");
        let strength = params.vibrance / 100.0;
        self.dispatch_simple(ctx, "vibrance", input, &output, &[strength, 0.0, 0.0, 0.0])?;
        Ok(output)
    }

    fn apply_saturation(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        if params.saturation == 0.0 {
            return self.passthrough(ctx, input);
        }
        debug!(saturation = params.saturation, "GPU saturation");
        let output = GpuTexture::create_storage(&ctx.device, input.width, input.height, "sat_out");
        let blend = 1.0 + params.saturation / 100.0;
        self.dispatch_simple(ctx, "saturation", input, &output, &[blend, 0.0, 0.0, 0.0])?;
        Ok(output)
    }

    fn apply_hsl(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        if params.hsl_hue == 0.0 && params.hsl_saturation == 0.0 && params.hsl_lightness == 0.0 {
            return self.passthrough(ctx, input);
        }
        debug!(
            hue = params.hsl_hue,
            sat = params.hsl_saturation,
            light = params.hsl_lightness,
            "GPU HSL"
        );
        let output = GpuTexture::create_storage(&ctx.device, input.width, input.height, "hsl_out");

        let do_hue = params.hsl_hue != 0.0;
        let do_sat = params.hsl_saturation != 0.0;
        let sat_blend = 1.0 + params.hsl_saturation / 100.0;
        let light_scale = 1.0 + params.hsl_lightness / 100.0;

        let m = if do_hue {
            hue_rotation_matrix(params.hsl_hue)
        } else {
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        };

        #[rustfmt::skip]
        let data = [
            m[0], m[1], m[2], 0.0,
            m[3], m[4], m[5], 0.0,
            m[6], m[7], m[8], 0.0,
            sat_blend, light_scale,
            if do_hue { 1.0 } else { 0.0 },
            if do_sat { 1.0 } else { 0.0 },
        ];

        self.dispatch_simple(ctx, "hsl", input, &output, &data)?;
        Ok(output)
    }

    fn apply_crop(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        if params.crop_x == 0.0
            && params.crop_y == 0.0
            && params.crop_w == 1.0
            && params.crop_h == 1.0
        {
            return self.passthrough(ctx, input);
        }

        debug!("GPU crop");

        let src_x =
            ((params.crop_x * input.width as f32) as u32).min(input.width.saturating_sub(1));
        let src_y =
            ((params.crop_y * input.height as f32) as u32).min(input.height.saturating_sub(1));
        let remaining_w = input.width.saturating_sub(src_x);
        let remaining_h = input.height.saturating_sub(src_y);
        let dst_w = (params.crop_w * input.width as f32).max(1.0) as u32;
        let dst_h = (params.crop_h * input.height as f32).max(1.0) as u32;
        let dst_w = dst_w.min(remaining_w).max(1);
        let dst_h = dst_h.min(remaining_h).max(1);

        let output = GpuTexture::create_storage(&ctx.device, dst_w, dst_h, "crop_out");

        // Pack u32 params as f32 bits
        let data = [f32::from_bits(src_x), f32::from_bits(src_y), 0.0, 0.0];

        // Crop dispatches based on OUTPUT dimensions
        let params_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crop_params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue
            .write_buffer(&params_buf, 0, bytemuck::cast_slice(&data));

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crop_bg"),
            layout: &self.image_params_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&output.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let pipeline =
            self.shaders
                .get_or_create_pipeline(&ctx.device, "crop", &self.image_params_bgl)?;

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("crop_encoder"),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("crop_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                dst_w.div_ceil(WORKGROUP_SIZE),
                dst_h.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }

        ctx.queue.submit(std::iter::once(encoder.finish()));
        Ok(output)
    }

    /// Identity pass: just return a reference-equivalent texture.
    /// For simplicity, we pass through by running exposure with multiplier=1.0.
    fn passthrough(&mut self, ctx: &GpuContext, input: &GpuTexture) -> Result<GpuTexture> {
        let output = GpuTexture::create_storage(&ctx.device, input.width, input.height, "pass");
        self.dispatch_simple(ctx, "exposure", input, &output, &[1.0, 0.0, 0.0, 0.0])?;
        Ok(output)
    }
}

fn create_image_params_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("image_params_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::ReadOnly,
                    format: wgpu::TextureFormat::Rgba32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn create_tone_curve_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("tone_curve_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::ReadOnly,
                    format: wgpu::TextureFormat::Rgba32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn hue_rotation_matrix(degrees: f32) -> [f32; 9] {
    let angle = degrees.to_radians();
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let len = (0.2126_f32.powi(2) + 0.7152_f32.powi(2) + 0.0722_f32.powi(2)).sqrt();
    let (kx, ky, kz) = (0.2126 / len, 0.7152 / len, 0.0722 / len);
    let c1 = 1.0 - cos_a;
    [
        cos_a + c1 * kx * kx,
        c1 * kx * ky - sin_a * kz,
        c1 * kx * kz + sin_a * ky,
        c1 * ky * kx + sin_a * kz,
        cos_a + c1 * ky * ky,
        c1 * ky * kz - sin_a * kx,
        c1 * kz * kx - sin_a * ky,
        c1 * kz * ky + sin_a * kx,
        cos_a + c1 * kz * kz,
    ]
}
