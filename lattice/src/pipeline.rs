//! Compute pipeline: descriptor layout, push constants, single dispatch.

use ash::vk;
use log::info;
use std::io::Cursor;

use crate::error::{Result, RtxError};

static SHADER_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/passthrough.comp.spv"));

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PushConstants {
    pub width:  i32,
    pub height: i32,
    pub time:   f32,
}

pub struct ComputePipeline {
    pub pipeline:        vk::Pipeline,
    pub layout:          vk::PipelineLayout,
    pub desc_layout:     vk::DescriptorSetLayout,
    pub desc_pool:       vk::DescriptorPool,
    pub desc_set:        vk::DescriptorSet,
    shader_module:       vk::ShaderModule,
}

impl ComputePipeline {
    pub fn new(device: &ash::Device) -> Result<Self> {
        // shader module 
        let words = read_spv(SHADER_SPV)?;
        let smci = vk::ShaderModuleCreateInfo::default().code(&words);
        // SAFETY: SPIR-V was compiled by shaderc at build time.
        let shader_module = unsafe { device.create_shader_module(&smci, None) }
            .map_err(|e| RtxError::Init(format!("create_shader_module: {e:?}")))?;

        // descriptor set layout (2 storage images)     
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let dslci = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let desc_layout = unsafe { device.create_descriptor_set_layout(&dslci, None) }?;

        // pipeline layout (set 0 + push constants) 
        let pc_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(std::mem::size_of::<PushConstants>() as u32);
        let plci = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(std::slice::from_ref(&desc_layout))
            .push_constant_ranges(std::slice::from_ref(&pc_range));
        let layout = unsafe { device.create_pipeline_layout(&plci, None) }?;

        // compute pipeline 
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(shader_module)
            .name(c"main");
        let cpci = vk::ComputePipelineCreateInfo::default()
            .stage(stage)
            .layout(layout);
        let pipeline = unsafe {
            device.create_compute_pipelines(vk::PipelineCache::null(), &[cpci], None)
        }
        .map_err(|(_partial, e)| RtxError::Vulkan(e))?[0];

        // descriptor pool + set -
        let ps = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_IMAGE)
            .descriptor_count(2);
        let dpci = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(std::slice::from_ref(&ps))
            .max_sets(1);
        let desc_pool = unsafe { device.create_descriptor_pool(&dpci, None) }?;

        let dsai = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(desc_pool)
            .set_layouts(std::slice::from_ref(&desc_layout));
        let desc_set = unsafe { device.allocate_descriptor_sets(&dsai) }?[0];

        info!("Compute pipeline created");

        Ok(Self { pipeline, layout, desc_layout, desc_pool, desc_set, shader_module })
    }

    /// Point the descriptor set at the given image views (both GENERAL layout).
    pub fn update_descriptors(
        &self,
        device: &ash::Device,
        input_view: vk::ImageView,
        output_view: vk::ImageView,
    ) {
        let infos_in  = [vk::DescriptorImageInfo::default()
            .image_view(input_view)
            .image_layout(vk::ImageLayout::GENERAL)];
        let infos_out = [vk::DescriptorImageInfo::default()
            .image_view(output_view)
            .image_layout(vk::ImageLayout::GENERAL)];

        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.desc_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&infos_in),
            vk::WriteDescriptorSet::default()
                .dst_set(self.desc_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(&infos_out),
        ];
        // SAFETY: descriptor set and image views are valid.
        unsafe { device.update_descriptor_sets(&writes, &[]) };
    }

    pub fn destroy(&self, device: &ash::Device) {
        // SAFETY: objects were created by us; device is idle.
        unsafe {
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_pipeline_layout(self.layout, None);
            device.destroy_descriptor_pool(self.desc_pool, None);
            device.destroy_descriptor_set_layout(self.desc_layout, None);
            device.destroy_shader_module(self.shader_module, None);
        }
    }
}

/// Convert a byte slice to a SPIR-V word vector (LE).
fn read_spv(bytes: &[u8]) -> Result<Vec<u32>> {
    if bytes.len() % 4 != 0 {
        return Err(RtxError::Init("SPIR-V size not aligned to 4".into()));
    }
    // Try ash::util if available, otherwise manual
    ash::util::read_spv(&mut Cursor::new(bytes))
        .map_err(|e| RtxError::Init(format!("read_spv: {e}")))
}