use std::sync::Mutex;

use fnv::FnvHashMap;
use once_cell::sync::Lazy;

use crate::{core::{ext::Utf16StringExt, hachimi::AssetMetadata}, il2cpp::{
    api::il2cpp_resolve_icall, ext::Il2CppStringExt, symbols::GCHandle, types::*
}};

pub const ASSET_PATH_PREFIX: &str = "assets/_gallopresources/bundle/resources/";

pub struct RequestInfo {
    pub name_handle: GCHandle,
    pub bundle: usize // *mut Il2CppObject (this)
}
impl RequestInfo {
    pub fn name(&self) -> *mut Il2CppString {
        self.name_handle.target() as _
    }
}
pub static REQUEST_INFOS: Lazy<Mutex<FnvHashMap<usize, RequestInfo>>> = Lazy::new(|| Mutex::default());

static BUNDLE_PATHS: Lazy<Mutex<FnvHashMap<usize, GCHandle>>> = Lazy::new(|| Mutex::default());
pub fn get_bundle_path(this: *mut Il2CppObject) -> Option<*mut Il2CppString> {
    Some(BUNDLE_PATHS.lock().unwrap().get(&(this as usize))?.target() as _)
}

pub fn check_asset_bundle_name(this: *mut Il2CppObject, metadata: &AssetMetadata) -> bool {
    if let Some(meta_bundle_name) = &metadata.bundle_name {
        if let Some(bundle_path) = get_bundle_path(this) {
            let bundle_name = unsafe { (*bundle_path).as_utf16str().path_filename() };
            if !bundle_name.str_eq(&meta_bundle_name) {
                warn!("Expected bundle {}, got {}", meta_bundle_name, bundle_name);
                return false;
            }
        }
    }

    true
}

type LoadAssetFn = extern "C" fn(this: *mut Il2CppObject, name: *mut Il2CppString, type_: *mut Il2CppObject) -> *mut Il2CppObject;
extern "C" fn LoadAsset_Internal(this: *mut Il2CppObject, name: *mut Il2CppString, type_: *mut Il2CppObject) -> *mut Il2CppObject {
    let asset = get_orig_fn!(LoadAsset_Internal, LoadAssetFn)(this, name, type_);
    on_LoadAsset(this, asset, name);
    asset
}

pub fn LoadAsset_Internal_orig(this: *mut Il2CppObject, name: *mut Il2CppString, type_: *mut Il2CppObject) -> *mut Il2CppObject {
    get_orig_fn!(LoadAsset_Internal, LoadAssetFn)(this, name, type_)
}

type LoadAssetAsyncFn = extern "C" fn(this: *mut Il2CppObject, name: *mut Il2CppString, type_: *mut Il2CppObject) -> *mut Il2CppObject;
extern "C" fn LoadAssetAsync_Internal(this: *mut Il2CppObject, name: *mut Il2CppString, type_: *mut Il2CppObject) -> *mut Il2CppObject {
    let request = get_orig_fn!(LoadAssetAsync_Internal, LoadAssetAsyncFn)(this, name, type_);
    let info = RequestInfo {
        name_handle: GCHandle::new(name as _, false), // is name even guaranteed to survive in memory..?
        bundle: this as usize
    };
    REQUEST_INFOS.lock().unwrap().insert(request as usize, info);
    request
}

// 資產替換（在地化）已移除。這裡保留成 no-op，只為了讓字體載入器仍能透過
// LoadAsset_Internal_orig / LoadFromFile_Internal_orig 拿到原始函式（需要 hook 有裝）。
pub fn on_LoadAsset(_bundle: *mut Il2CppObject, _asset: *mut Il2CppObject, _name: *mut Il2CppString) {
}

type LoadFromFileInternalFn = extern "C" fn(path: *mut Il2CppString, crc: u32, offset: u64) -> *mut Il2CppObject;
extern "C" fn LoadFromFile_Internal(path: *mut Il2CppString, crc: u32, offset: u64) -> *mut Il2CppObject {
    let bundle = get_orig_fn!(LoadFromFile_Internal, LoadFromFileInternalFn)(path, crc, offset);
    if !bundle.is_null() {
        BUNDLE_PATHS.lock().unwrap().insert(bundle as usize, GCHandle::new(path as _, false));
    }
    bundle
}

pub fn LoadFromFile_Internal_orig(path: *mut Il2CppString, crc: u32, offset: u64) -> *mut Il2CppObject {
    get_orig_fn!(LoadFromFile_Internal, LoadFromFileInternalFn)(path, crc, offset)
}

type UnloadFn = extern "C" fn(this: *mut Il2CppObject, unload_all_loaded_objects: bool);
extern "C" fn Unload(this: *mut Il2CppObject, unload_all_loaded_objects: bool) {
    BUNDLE_PATHS.lock().unwrap().remove(&(this as usize));
    get_orig_fn!(Unload, UnloadFn)(this, unload_all_loaded_objects);
}

pub fn init(_UnityEngine_AssetBundleModule: *const Il2CppImage) {
    //get_class_or_return!(UnityEngine_AssetBundleModule, UnityEngine, AssetBundle);

    let LoadAsset_Internal_addr = il2cpp_resolve_icall(
        c"UnityEngine.AssetBundle::LoadAsset_Internal(System.String,System.Type)".as_ptr()
    );
    let LoadAssetAsync_Internal_addr = il2cpp_resolve_icall(
        c"UnityEngine.AssetBundle::LoadAssetAsync_Internal(System.String,System.Type)".as_ptr()
    );
    let LoadFromFile_Internal_addr = il2cpp_resolve_icall(
        c"UnityEngine.AssetBundle::LoadFromFile_Internal(System.String,System.UInt32,System.UInt64)".as_ptr()
    );
    let Unload_addr = il2cpp_resolve_icall(
        c"UnityEngine.AssetBundle::Unload(System.Boolean)".as_ptr()
    );

    new_hook!(LoadAsset_Internal_addr, LoadAsset_Internal);
    new_hook!(LoadAssetAsync_Internal_addr, LoadAssetAsync_Internal);
    new_hook!(LoadFromFile_Internal_addr, LoadFromFile_Internal);
    new_hook!(Unload_addr, Unload);
}