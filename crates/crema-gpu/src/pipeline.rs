use anyhow::Result;
use tracing::debug;

use crema_core::image_buf::EditParams;
use crema_core::pipeline::modules::wb_matrix;

use crate::context::GpuContext;
use crate::shader::ShaderManager;
use crate::texture::GpuTexture;

const WORKGROUP_SIZE: u32 = 16;

/// GPU processing pipeline that applies edits via compute shaders.
pub struct GpuPipeline {
    shaders: ShaderManager,
    exposure_bgl: wgpu::BindGroupLayout,
    white_balance_bgl: wgpu::BindGroupLayout,
}

impl GpuPipeline {
    pub fn new(ctx: &GpuContext) -> Self {
        let mut shaders = ShaderManager::new();

        shaders.load_shader(
            &ctx.device,
            "exposure",
            include_str!("../shaders/exposure.wgsl"),
        );
        shaders.load_shader(
            &ctx.device,
            "white_balance",
            include_str!("../shaders/white_balance.wgsl"),
        );

        let exposure_bgl = Self::create_image_params_layout(&ctx.device, "exposure_bgl");
        let white_balance_bgl = Self::create_image_params_layout(&ctx.device, "white_balance_bgl");

        Self {
            shaders,
            exposure_bgl,
            white_balance_bgl,
        }
    }

    fn create_image_params_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[
                // Input texture
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
                // Output texture
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
                // Params uniform
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

    /// Run the full GPU pipeline: white balance -> exposure.
    pub fn process(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        let wb_output = self.apply_white_balance(ctx, input, params)?;
        let exp_output = self.apply_exposure(ctx, &wb_output, params)?;
        Ok(exp_output)
    }

    fn apply_exposure(
        &mut self,
        ctx: &GpuContext,
        input: &GpuTexture,
        params: &EditParams,
    ) -> Result<GpuTexture> {
        debug!(exposure = params.exposure, "GPU exposure");

        let output =
            GpuTexture::create_storage(&ctx.device, input.width, input.height, "exposure_out");

        let multiplier = 2.0_f32.powf(params.exposure);
        let params_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure_params"),
            size: 16, // vec4<f32> alignment
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue.write_buffer(
            &params_buf,
            0,
            bytemuck::cast_slice(&[multiplier, 0.0_f32, 0.0_f32, 0.0_f32]),
        );

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exposure_bg"),
            layout: &self.exposure_bgl,
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
                .get_or_create_pipeline(&ctx.device, "exposure", &self.exposure_bgl)?;

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("exposure_encoder"),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("exposure_pass"),
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

        // Use the same Bradford CAT matrix as the CPU pipeline.
        let matrix = wb_matrix(params.wb_temp, params.wb_tint);

        let params_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wb_params"),
            size: 48, // 3 x vec4<f32>
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue.write_buffer(
            &params_buf,
            0,
            bytemuck::cast_slice(&[
                matrix[0], matrix[1], matrix[2], 0.0_f32, matrix[3], matrix[4], matrix[5], 0.0_f32,
                matrix[6], matrix[7], matrix[8], 0.0_f32,
            ]),
        );

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wb_bg"),
            layout: &self.white_balance_bgl,
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

        let pipeline = self.shaders.get_or_create_pipeline(
            &ctx.device,
            "white_balance",
            &self.white_balance_bgl,
        )?;

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("wb_encoder"),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("wb_pass"),
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
}
