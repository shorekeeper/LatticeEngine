//! Top-level orchestrator: owns context + interop + pipeline, runs per-frame.

use ash::vk;
use log::info;

use crate::error::{Result, RtxError};
use crate::interop::InteropResources;
use crate::pipeline::{ComputePipeline, PushConstants};
use crate::vulkan_context::VulkanContext;

pub struct Renderer {
    pub context:  VulkanContext,
    interop:      InteropResources,
    pipeline:     ComputePipeline,
    cmd:          vk::CommandBuffer,
    width:        u32,
    height:       u32,
    frame_count:  u32,
}

impl Renderer {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let context  = VulkanContext::new()?;
        let interop  = InteropResources::new(&context, width, height)?;
        let pipeline = ComputePipeline::new(&context.device)?;
        pipeline.update_descriptors(
            &context.device,
            interop.input.view,
            interop.output.view,
        );

        let ai = vk::CommandBufferAllocateInfo::default()
            .command_pool(context.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: pool valid.
        let cmd = unsafe { context.device.allocate_command_buffers(&ai) }?[0];

        info!("Renderer ready ({width}×{height})");
        Ok(Self { context, interop, pipeline, cmd, width, height, frame_count: 0 })
    }

    pub fn gpu_name(&self) -> &str { &self.context.gpu_name }
    pub fn rt_supported(&self) -> bool { self.context.rt_supported }

    /// Returns [input_handle, input_alloc_size, output_handle, output_alloc_size].
    pub fn get_handles(&self) -> Vec<i64> {
        vec![
            self.interop.input.handle,
            self.interop.input.alloc_size as i64,
            self.interop.output.handle,
            self.interop.output.alloc_size as i64,
        ]
    }

    /// Run the compute pass: read input → vignette → write output.
    pub fn process_frame(&mut self) -> Result<()> {
        let dev = &self.context.device;

        // SAFETY: command buffer was allocated for reset.
        unsafe {
            dev.reset_command_buffer(self.cmd, vk::CommandBufferResetFlags::empty())?;
            dev.begin_command_buffer(
                self.cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }

        let color_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0, level_count: 1,
            base_array_layer: 0, layer_count: 1,
        };

        // Barrier: make GL writes visible to compute read.
        let pre = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.interop.input.image)
            .subresource_range(color_range);

        unsafe {
            dev.cmd_pipeline_barrier(
                self.cmd,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[], &[], &[pre],
            );
        }

        // Bind pipeline + descriptors, push constants, dispatch.
        let pc = PushConstants {
            width:  self.width as i32,
            height: self.height as i32,
            time:   self.frame_count as f32 * (1.0 / 60.0),
        };
        // SAFETY: PushConstants is repr(C) with no padding.
        let pc_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                &pc as *const PushConstants as *const u8,
                std::mem::size_of::<PushConstants>(),
            )
        };

        unsafe {
            dev.cmd_bind_pipeline(self.cmd, vk::PipelineBindPoint::COMPUTE, self.pipeline.pipeline);
            dev.cmd_bind_descriptor_sets(
                self.cmd,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline.layout,
                0,
                &[self.pipeline.desc_set],
                &[],
            );
            dev.cmd_push_constants(
                self.cmd,
                self.pipeline.layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                pc_bytes,
            );
            dev.cmd_dispatch(
                self.cmd,
                (self.width  + 15) / 16,
                (self.height + 15) / 16,
                1,
            );
        }

        // Barrier: make compute writes visible before GL reads.
        let post = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.interop.output.image)
            .subresource_range(color_range);

        unsafe {
            dev.cmd_pipeline_barrier(
                self.cmd,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[], &[], &[post],
            );
            dev.end_command_buffer(self.cmd)?;
        }

        // Submit and wait (brute-force sync — fine for v0).
        let si = vk::SubmitInfo::default()
            .command_buffers(std::slice::from_ref(&self.cmd));
        unsafe {
            dev.queue_submit(self.context.queue, &[si], vk::Fence::null())?;
            dev.queue_wait_idle(self.context.queue)?;
        }

        self.frame_count += 1;
        Ok(())
    }

    /// Recreate shared images at a new resolution; returns fresh handles.
    pub fn resize(&mut self, w: u32, h: u32) -> Result<Vec<i64>> {
        info!("Renderer resize → {w}×{h}");
        unsafe { self.context.device.device_wait_idle()?; }

        self.interop.destroy(&self.context.device);
        self.interop = InteropResources::new(&self.context, w, h)?;
        self.pipeline.update_descriptors(
            &self.context.device,
            self.interop.input.view,
            self.interop.output.view,
        );
        self.width  = w;
        self.height = h;
        Ok(self.get_handles())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.context.device.device_wait_idle().ok();
            self.context
                .device
                .free_command_buffers(self.context.command_pool, &[self.cmd]);
        }
        self.pipeline.destroy(&self.context.device);
        self.interop.destroy(&self.context.device);
        info!("Renderer destroyed");
    }
}