use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::debug;

/// Manages compute shader modules and pipeline caching.
pub struct ShaderManager {
    modules: HashMap<String, wgpu::ShaderModule>,
    pipelines: HashMap<String, wgpu::ComputePipeline>,
}

impl ShaderManager {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            pipelines: HashMap::new(),
        }
    }

    pub fn load_shader(&mut self, device: &wgpu::Device, name: &str, source: &str) {
        debug!(name, "loading compute shader");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        self.modules.insert(name.to_string(), module);
    }

    pub fn get_or_create_pipeline(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Result<&wgpu::ComputePipeline> {
        if !self.pipelines.contains_key(name) {
            let module = self
                .modules
                .get(name)
                .with_context(|| format!("shader not loaded: {name}"))?;

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&format!("{name}_layout")),
                bind_group_layouts: &[bind_group_layout],
                push_constant_ranges: &[],
            });

            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });

            self.pipelines.insert(name.to_string(), pipeline);
        }

        Ok(&self.pipelines[name])
    }
}

impl Default for ShaderManager {
    fn default() -> Self {
        Self::new()
    }
}
