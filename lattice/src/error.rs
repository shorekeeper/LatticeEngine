use std::fmt;

#[derive(Debug)]
pub enum RtxError {
    Vulkan(ash::vk::Result),
    Loading(ash::LoadingError),
    Init(String),
    NoSuitableGpu,
    Interop(String),
}

impl fmt::Display for RtxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vulkan(e)  => write!(f, "Vulkan error: {e:?}"),
            Self::Loading(e) => write!(f, "Vulkan loader error: {e:?}"),
            Self::Init(msg)  => write!(f, "Init failed: {msg}"),
            Self::NoSuitableGpu => write!(f, "No suitable GPU found"),
            Self::Interop(msg)  => write!(f, "Interop error: {msg}"),
        }
    }
}

impl std::error::Error for RtxError {}

impl From<ash::vk::Result> for RtxError {
    fn from(e: ash::vk::Result) -> Self { Self::Vulkan(e) }
}
impl From<ash::LoadingError> for RtxError {
    fn from(e: ash::LoadingError) -> Self { Self::Loading(e) }
}

pub type Result<T> = std::result::Result<T, RtxError>;