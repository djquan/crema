use anyhow::{Context, Result};
use tracing::info;

/// Holds the wgpu device and queue for GPU operations.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    pub async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .context("no suitable GPU adapter found")?;

        info!(
            adapter = adapter.get_info().name,
            backend = ?adapter.get_info().backend,
            "selected GPU adapter"
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("crema"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .context("failed to create GPU device")?;

        Ok(Self { device, queue })
    }
}
