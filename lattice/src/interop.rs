//! GL ↔ VK shared images via VK_KHR_external_memory.

use ash::vk;
use log::info;

use crate::error::{Result, RtxError};
use crate::vulkan_context::VulkanContext;

pub const IMAGE_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

/// A single VkImage whose memory is exportable for GL import.
pub struct SharedImage {
    pub image:  vk::Image,
    pub memory: vk::DeviceMemory,
    pub view:   vk::ImageView,
    pub alloc_size: u64,
    /// Platform handle: fd on Linux, HANDLE-as-i64 on Windows.
    pub handle: i64,
}

/// Two shared images: MC-frame-in and processed-frame-out.
pub struct InteropResources {
    pub input:  SharedImage,
    pub output: SharedImage,
    pub width:  u32,
    pub height: u32,
}

impl InteropResources {
    pub fn new(ctx: &VulkanContext, width: u32, height: u32) -> Result<Self> {
        info!("Creating interop images {width}×{height}");
        let input  = create_shared_image(ctx, width, height)?;
        let output = create_shared_image(ctx, width, height)?;

        // Transition both to GENERAL so compute can use them immediately.
        transition_to_general(ctx, &[input.image, output.image])?;

        Ok(Self { input, output, width, height })
    }

    pub fn destroy(&self, device: &ash::Device) {
        // SAFETY: images were created by us and are no longer in use
        // (caller must ensure device idle).
        unsafe {
            destroy_shared(device, &self.input);
            destroy_shared(device, &self.output);
        }
        info!("Interop images destroyed");
    }
}

// 
// platform helpers
// 

#[cfg(target_os = "linux")]
fn handle_type() -> vk::ExternalMemoryHandleTypeFlags {
    vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD
}
#[cfg(target_os = "windows")]
fn handle_type() -> vk::ExternalMemoryHandleTypeFlags {
    vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32
}

fn export_handle(ctx: &VulkanContext, memory: vk::DeviceMemory) -> Result<i64> {
    #[cfg(target_os = "linux")]
    {
        let gi = vk::MemoryGetFdInfoKHR::default()
            .memory(memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        // SAFETY: memory was allocated with OPAQUE_FD export.
        let fd = unsafe { ctx.ext_mem_fd.get_memory_fd(&gi) }?;
        Ok(fd as i64)
    }
    #[cfg(target_os = "windows")]
    {
        let gi = vk::MemoryGetWin32HandleInfoKHR::default()
            .memory(memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32);
        // SAFETY: memory was allocated with OPAQUE_WIN32 export.
        let h = unsafe { ctx.ext_mem_win32.get_memory_win32_handle(&gi) }?;
        Ok(h as i64)
    }
}

// 
// creation / destruction
// 

fn create_shared_image(ctx: &VulkanContext, w: u32, h: u32) -> Result<SharedImage> {
    let ht = handle_type();

    let mut ext_info = vk::ExternalMemoryImageCreateInfo::default().handle_types(ht);

    let ici = vk::ImageCreateInfo::default()
        .push_next(&mut ext_info)
        .image_type(vk::ImageType::TYPE_2D)
        .format(IMAGE_FORMAT)
        .extent(vk::Extent3D { width: w, height: h, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(
            vk::ImageUsageFlags::STORAGE
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST,
        )
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    // SAFETY: create-info is valid.
    let image = unsafe { ctx.device.create_image(&ici, None) }?;

    // SAFETY: image just created.
    let req = unsafe { ctx.device.get_image_memory_requirements(image) };
    let mem_idx = ctx.find_memory_type(&req, vk::MemoryPropertyFlags::DEVICE_LOCAL)?;

    let mut export_ai = vk::ExportMemoryAllocateInfo::default().handle_types(ht);
    let mut dedicated  = vk::MemoryDedicatedAllocateInfo::default().image(image);

    let ai = vk::MemoryAllocateInfo::default()
        .push_next(&mut export_ai)
        .push_next(&mut dedicated)
        .allocation_size(req.size)
        .memory_type_index(mem_idx);

    // SAFETY: alloc-info is valid.
    let memory = unsafe { ctx.device.allocate_memory(&ai, None) }?;
    // SAFETY: image + memory are compatible.
    unsafe { ctx.device.bind_image_memory(image, memory, 0) }?;

    let vci = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(IMAGE_FORMAT)
        .subresource_range(full_color_range());
    // SAFETY: image is valid.
    let view = unsafe { ctx.device.create_image_view(&vci, None) }?;

    let handle = export_handle(ctx, memory)?;

    Ok(SharedImage { image, memory, view, alloc_size: req.size, handle })
}

unsafe fn destroy_shared(device: &ash::Device, img: &SharedImage) {
    device.destroy_image_view(img.view, None);
    device.destroy_image(img.image, None);
    device.free_memory(img.memory, None);
}

fn transition_to_general(ctx: &VulkanContext, images: &[vk::Image]) -> Result<()> {
    ctx.one_shot(|cb| {
        let barriers: Vec<_> = images
            .iter()
            .map(|&img| {
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::empty())
                    .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(img)
                    .subresource_range(full_color_range())
            })
            .collect();
        // SAFETY: command buffer is recording, images are valid.
        unsafe {
            ctx.device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers,
            );
        }
    })
}

fn full_color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}