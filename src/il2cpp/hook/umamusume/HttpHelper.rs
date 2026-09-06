//! Hook `Gallop.HttpHelper::DecompressResponse(byte[]) -> byte[]`。
//!
//! 這是所有 API response 的 LZ4 解壓 choke point（`HttpManager.DecompressFunc`）。其**回傳值**
//! ＝已解密（Coneshell native）＋已解壓的 msgpack 明文。我們讀回傳的 byte[] 交給
//! [`api_packet::capture_response`] 解碼＋落檔。單一 static 方法涵蓋所有 endpoint。
//!
//! response 端同時餵給因子卡片與「練習賽擷取」，另有全量 API 擷取（皆在 `api_packet`）。

use crate::{
    core::api_packet,
    il2cpp::{symbols::{get_method_addr, Array}, types::*},
};

type DecompressResponseFn = extern "C" fn(response_data: *mut Il2CppArray) -> *mut Il2CppArray;
extern "C" fn DecompressResponse(response_data: *mut Il2CppArray) -> *mut Il2CppArray {
    let result = get_orig_fn!(DecompressResponse, DecompressResponseFn)(response_data);
    if !result.is_null() {
        let arr: Array<u8> = Array::from(result);
        let bytes: &[u8] = unsafe { arr.as_slice() };
        api_packet::capture_response(bytes);
    }
    result
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, HttpHelper);

    let DecompressResponse_addr = get_method_addr(HttpHelper, c"DecompressResponse", 1);
    new_hook!(DecompressResponse_addr, DecompressResponse);
}
