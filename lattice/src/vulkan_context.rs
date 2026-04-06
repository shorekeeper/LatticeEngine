use ash::vk;
use log::{info, warn};
use std::ffi::CStr;

use crate::error::{Result, RtxError};

/// Core Vulkan state: instance, device, queue, command pool.
/// Created once and reused for the lifetime of the mod.
pub struct VulkanContext {
    pub _entry: ash::Entry,
    pub instance: ash::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub queue_family: u32,
    pub command_pool: vk::CommandPool,
    pub gpu_name: String,
    pub rt_supported: bool,
    #[cfg(target_os = "linux")]
    pub ext_mem_fd: ash::khr::external_memory_fd::Device,
    #[cfg(target_os = "windows")]
    pub ext_mem_win32: ash::khr::external_memory_win32::Device,
    pub allocator: Option<gpu_allocator::vulkan::Allocator>,
}

impl VulkanContext {
    /// Spin up Vulkan 1.2 with external-memory + optional RT extensions.
    pub fn new() -> Result<Self> {
        // entry 
        // SAFETY: loads the Vulkan loader dynamically; fails gracefully.
        let entry = unsafe { ash::Entry::load() }?;

        // instance 
        let app_info = vk::ApplicationInfo::default()
            .application_name(c"MinecraftRTX")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"RTXEngine")
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_2);

        let ci = vk::InstanceCreateInfo::default().application_info(&app_info);
        // SAFETY: valid create-info, no custom allocator.
        let instance = unsafe { entry.create_instance(&ci, None) }
            .map_err(|e| RtxError::Init(format!("vkCreateInstance: {e:?}")))?;

        // physical device -
        // SAFETY: instance is valid.
        let pdevices = unsafe { instance.enumerate_physical_devices() }
            .map_err(|e| RtxError::Init(format!("enumerate_physical_devices: {e:?}")))?;
        if pdevices.is_empty() {
            return Err(RtxError::NoSuitableGpu);
        }

        let (physical_device, gpu_name, rt_ext_ok) = pick_gpu(&instance, &pdevices)?;
        info!("Selected GPU: {gpu_name}");

        // query features 
        let mut feat12 = vk::PhysicalDeviceVulkan12Features::default();
        let mut feat_as = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        let mut feat_rt = vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default();
        let mut feat2 = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut feat12)
            .push_next(&mut feat_as)
            .push_next(&mut feat_rt);
        // SAFETY: instance + pdev are valid.
        unsafe { instance.get_physical_device_features2(physical_device, &mut feat2) };

        let storage_ext = feat2.features.shader_storage_image_extended_formats == vk::TRUE;
        if !storage_ext {
            return Err(RtxError::Init(format!(
                "GPU '{gpu_name}' lacks shaderStorageImageExtendedFormats"
            )));
        }

        let rt_supported = rt_ext_ok
            && feat12.buffer_device_address == vk::TRUE
            && feat_as.acceleration_structure == vk::TRUE
            && feat_rt.ray_tracing_pipeline == vk::TRUE;
        info!("RT support: {}", if rt_supported { "yes" } else { "no" });

        // queue family -
        // SAFETY: instance + pdev are valid.
        let qf_props =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
        let queue_family = qf_props
            .iter()
            .enumerate()
            .find(|(_, p)| p.queue_flags.contains(vk::QueueFlags::COMPUTE | vk::QueueFlags::GRAPHICS))
            .or_else(|| {
                qf_props.iter().enumerate().find(|(_, p)| p.queue_flags.contains(vk::QueueFlags::COMPUTE))
            })
            .map(|(i, _)| i as u32)
            .ok_or(RtxError::Init("No compute queue family".into()))?;

        // device extensions 
        let mut dev_exts: Vec<*const i8> = Vec::new();
        #[cfg(target_os = "linux")]
        dev_exts.push(ash::khr::external_memory_fd::NAME.as_ptr());
        #[cfg(target_os = "windows")]
        dev_exts.push(ash::khr::external_memory_win32::NAME.as_ptr());

        if rt_supported {
            dev_exts.push(ash::khr::deferred_host_operations::NAME.as_ptr());
            dev_exts.push(ash::khr::acceleration_structure::NAME.as_ptr());
            dev_exts.push(ash::khr::ray_tracing_pipeline::NAME.as_ptr());
            info!("RT device extensions enabled (idle until needed)");
        }

        // device features to enable 
        let enabled_features = vk::PhysicalDeviceFeatures::default()
            .shader_storage_image_extended_formats(true);

        let mut en12 = vk::PhysicalDeviceVulkan12Features::default()
            .buffer_device_address(rt_supported);

        let mut en_as = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
            .acceleration_structure(rt_supported);
        let mut en_rt = vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default()
            .ray_tracing_pipeline(rt_supported);

        let prio = [1.0f32];
        let qci = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&prio)];

        let mut dci = vk::DeviceCreateInfo::default()
            .queue_create_infos(&qci)
            .enabled_extension_names(&dev_exts)
            .enabled_features(&enabled_features)
            .push_next(&mut en12);

        if rt_supported {
            dci = dci.push_next(&mut en_as).push_next(&mut en_rt);
        }

        // SAFETY: all structs are valid, extensions checked above.
        let device = unsafe { instance.create_device(physical_device, &dci, None) }
            .map_err(|e| RtxError::Init(format!("vkCreateDevice: {e:?}")))?;

        // SAFETY: device is valid, family/index checked.
        let queue = unsafe { device.get_device_queue(queue_family, 0) };

        // command pool -
        let pool_ci = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: device valid.
        let command_pool = unsafe { device.create_command_pool(&pool_ci, None) }
            .map_err(|e| RtxError::Init(format!("create_command_pool: {e:?}")))?;

        // extension loaders (before moving instance/device) -
        #[cfg(target_os = "linux")]
        let ext_mem_fd = ash::khr::external_memory_fd::Device::new(&instance, &device);
        #[cfg(target_os = "windows")]
        let ext_mem_win32 = ash::khr::external_memory_win32::Device::new(&instance, &device);

        // gpu-allocator (for future buffer allocs) 
        let allocator = gpu_allocator::vulkan::Allocator::new(
            &gpu_allocator::vulkan::AllocatorCreateDesc {
                instance: instance.clone(),
                device: device.clone(),
                physical_device,
                debug_settings: Default::default(),
                buffer_device_address: rt_supported,
                allocation_sizes: Default::default(),
            },
        )
        .map(|a| { info!("GPU allocator ready"); a })
        .map_err(|e| warn!("GPU allocator skipped: {e}"))
        .ok();

        Ok(Self {
            _entry: entry,
            instance,
            physical_device,
            device,
            queue,
            queue_family,
            command_pool,
            gpu_name,
            rt_supported,
            #[cfg(target_os = "linux")]
            ext_mem_fd,
            #[cfg(target_os = "windows")]
            ext_mem_win32,
            allocator,
        })
    }

    /// Find a memory type matching `requirements` and `flags`.
    pub fn find_memory_type(
        &self,
        req: &vk::MemoryRequirements,
        flags: vk::MemoryPropertyFlags,
    ) -> Result<u32> {
        // SAFETY: instance + pdev valid.
        let props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        (0..props.memory_type_count)
            .find(|&i| {
                (req.memory_type_bits & (1 << i)) != 0
                    && props.memory_types[i as usize]
                        .property_flags
                        .contains(flags)
            })
            .ok_or_else(|| RtxError::Init("No matching memory type".into()))
    }

    /// Submit a one-shot command buffer and wait.
    pub fn one_shot<F: FnOnce(vk::CommandBuffer)>(&self, f: F) -> Result<()> {
        let ai = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: pool valid.
        let cb = unsafe { self.device.allocate_command_buffers(&ai) }?[0];
        let bi = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.device.begin_command_buffer(cb, &bi)?;
            f(cb);
            self.device.end_command_buffer(cb)?;
            let si = vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cb));
            self.device.queue_submit(self.queue, &[si], vk::Fence::null())?;
            self.device.queue_wait_idle(self.queue)?;
            self.device.free_command_buffers(self.command_pool, &[cb]);
        }
        Ok(())
    }
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        unsafe { self.device.device_wait_idle().ok(); }
        // allocator must be dropped before the device
        drop(self.allocator.take());
        unsafe {
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
        info!("VulkanContext destroyed");
    }
}

// Renderer lives behind Mutex – assert Send.
// SAFETY: all inner ash types are Send (function-pointer tables + u64 handles).
// gpu-allocator::Allocator is Send (uses Arc<Mutex<…>> internally).
unsafe impl Send for VulkanContext {}

// 
// helpers
// 

fn pick_gpu(
    instance: &ash::Instance,
    devs: &[vk::PhysicalDevice],
) -> Result<(vk::PhysicalDevice, String, bool)> {
    let mut best: Option<(vk::PhysicalDevice, String, bool, u32)> = None;

    for &pd in devs {
        // SAFETY: instance valid.
        let props = unsafe { instance.get_physical_device_properties(pd) };
        let name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();

        let qfs = unsafe { instance.get_physical_device_queue_family_properties(pd) };
        if !qfs.iter().any(|q| q.queue_flags.contains(vk::QueueFlags::COMPUTE)) {
            continue;
        }

        let exts = unsafe { instance.enumerate_device_extension_properties(pd) }
            .unwrap_or_default();
        let has = |n: &CStr| exts.iter().any(|e| unsafe { CStr::from_ptr(e.extension_name.as_ptr()) } == n);

        // platform external-memory is mandatory
        #[cfg(target_os = "linux")]
        let ext_mem_ok = has(ash::khr::external_memory_fd::NAME);
        #[cfg(target_os = "windows")]
        let ext_mem_ok = has(ash::khr::external_memory_win32::NAME);

        if !ext_mem_ok {
            warn!("GPU '{name}': no external-memory extension, skipping");
            continue;
        }

        let rt_ok = has(ash::khr::deferred_host_operations::NAME)
            && has(ash::khr::acceleration_structure::NAME)
            && has(ash::khr::ray_tracing_pipeline::NAME);

        let mut score: u32 = 0;
        if props.device_type == vk::PhysicalDeviceType::DISCRETE_GPU { score += 1000; }
        if rt_ok { score += 500; }

        if best.as_ref().map_or(true, |b| score > b.3) {
            best = Some((pd, name, rt_ok, score));
        }
    }

    best.map(|(pd, n, rt, _)| (pd, n, rt))
        .ok_or(RtxError::NoSuitableGpu)
}