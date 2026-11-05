// A2U（AnimateToUnity）Flash 播放器
#[cfg(target_os = "windows")]
mod AnRootManager;

pub fn init() {
    get_assembly_image_or_return!(image, "Plugins.dll");

    #[cfg(target_os = "windows")]
    AnRootManager::init(image);
    #[cfg(not(target_os = "windows"))]
    let _ = image;
}
