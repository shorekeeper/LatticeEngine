//! JNI entry points for the RTX native library.

mod error;
mod interop;
mod pipeline;
mod renderer;
mod vulkan_context;
mod modular;

use jni::objects::JClass;
use jni::sys::*;
use jni::JNIEnv;
use log::{error, info};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use renderer::Renderer;

// 
// global state
// 

static RENDERER: Mutex<Option<Renderer>> = Mutex::new(None);
static GPU_NAME: OnceLock<String> = OnceLock::new();
static RT_FLAG: AtomicBool = AtomicBool::new(false);

// 
// helpers
// 

fn return_long_array(env: &mut JNIEnv, vals: &[i64]) -> jlongArray {
    match env.new_long_array(vals.len() as i32) {
        Ok(arr) => {
            let _ = env.set_long_array_region(&arr, 0, vals);
            arr.into_raw()
        }
        Err(_) => std::ptr::null_mut(),
    }
}

// 
// JNI exports
// 

/// Initialize Vulkan context, create shared images, return handles.
///
/// Returns `long[4]`: `[in_handle, in_alloc_size, out_handle, out_alloc_size]`
/// or `null` on failure.
#[no_mangle]
pub extern "system" fn Java_org_synergyst_latticeeng_NativeBridge_init(
    mut env: JNIEnv,
    _class: JClass,
    width: jint,
    height: jint,
) -> jlongArray {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    info!("RTX Native init ({width}×{height})");

    let renderer = match Renderer::new(width as u32, height as u32) {
        Ok(r) => r,
        Err(e) => {
            error!("Init failed: {e}");
            return std::ptr::null_mut();
        }
    };

    GPU_NAME.get_or_init(|| renderer.gpu_name().to_string());
    RT_FLAG.store(renderer.rt_supported(), Ordering::Relaxed);

    let handles = renderer.get_handles();
    info!(
        "RTX ready — GPU: {}, RT: {}",
        GPU_NAME.get().unwrap(),
        RT_FLAG.load(Ordering::Relaxed)
    );

    if let Ok(mut lock) = RENDERER.lock() {
        *lock = Some(renderer);
    }

    return_long_array(&mut env, &handles)
}

/// Run one compute pass (GL frame already in the input image).
#[no_mangle]
pub extern "system" fn Java_org_synergyst_latticeeng_NativeBridge_processFrame(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let Ok(mut lock) = RENDERER.lock() else { return 0 };
    let Some(r) = lock.as_mut() else { return 0 };
    match r.process_frame() {
        Ok(()) => 1,
        Err(e) => {
            error!("processFrame: {e}");
            0
        }
    }
}

/// Resize shared images; returns new handles (same layout as init) or null.
#[no_mangle]
pub extern "system" fn Java_org_synergyst_latticeeng_NativeBridge_resize(
    mut env: JNIEnv,
    _class: JClass,
    width: jint,
    height: jint,
) -> jlongArray {
    let Ok(mut lock) = RENDERER.lock() else { return std::ptr::null_mut() };
    let Some(r) = lock.as_mut() else { return std::ptr::null_mut() };
    match r.resize(width as u32, height as u32) {
        Ok(h) => return_long_array(&mut env, &h),
        Err(e) => {
            error!("resize: {e}");
            std::ptr::null_mut()
        }
    }
}

/// Tear down everything.
#[no_mangle]
pub extern "system" fn Java_org_synergyst_latticeeng_NativeBridge_shutdown(
    _env: JNIEnv,
    _class: JClass,
) {
    info!("RTX Native shutdown");
    if let Ok(mut lock) = RENDERER.lock() {
        lock.take(); // drops Renderer
    }
}

/// GPU name string (available even after failed init → "Unknown").
#[no_mangle]
pub extern "system" fn Java_org_synergyst_latticeeng_NativeBridge_getGpuName(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let name = GPU_NAME.get().map(|s| s.as_str()).unwrap_or("Unknown");
    env.new_string(name)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Whether the device was created with RT extensions enabled.
#[no_mangle]
pub extern "system" fn Java_org_synergyst_latticeeng_NativeBridge_isRtSupported(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    if RT_FLAG.load(Ordering::Relaxed) { 1 } else { 0 }
}