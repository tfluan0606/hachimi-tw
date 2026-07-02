// 一次性 il2cpp 全類別/方法 dump（用於偵測分析）。
// 遊戲的 global-metadata.dat 字串表在硬碟上被加密（class/method 名讀不到），
// 但 runtime 會解密到記憶體；我們的 DLL 已在進程內，直接用 il2cpp domain API 枚舉即可拿到真名。
// 觸發：在資料目錄放一個名為 `dump_il2cpp` 的空檔，下次啟動就會 dump 到 `il2cpp_dump.txt`。
use std::{
    ffi::CStr,
    fs::File,
    io::{BufWriter, Write},
    os::raw::{c_char, c_void},
    path::Path,
    ptr::null_mut,
};

use super::api::*;
use super::types::*;

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    CStr::from_ptr(p).to_string_lossy().into_owned()
}

/// 枚舉整個 il2cpp domain，把 `Namespace.Class::Method` 逐行寫出。
/// 回傳 (class 數, method 數)。
pub fn dump_all(out_path: &Path) -> std::io::Result<(usize, usize)> {
    let file = File::create(out_path)?;
    let mut w = BufWriter::new(file);
    let mut class_total = 0usize;
    let mut method_total = 0usize;

    unsafe {
        let domain = il2cpp_domain_get();
        if domain.is_null() {
            return Ok((0, 0));
        }
        // 從我們自己的 native thread 呼叫 il2cpp API，需先 attach 成 managed thread。
        il2cpp_thread_attach(domain);

        let mut asm_count: usize = 0;
        let assemblies = il2cpp_domain_get_assemblies(domain, &mut asm_count);

        for i in 0..asm_count {
            let assembly = *assemblies.add(i);
            if assembly.is_null() {
                continue;
            }
            let image = il2cpp_assembly_get_image(assembly);
            if image.is_null() {
                continue;
            }
            let ccount = il2cpp_image_get_class_count(image);
            for ci in 0..ccount {
                let klass = il2cpp_image_get_class(image, ci) as *mut Il2CppClass;
                if klass.is_null() {
                    continue;
                }
                let ns = cstr(il2cpp_class_get_namespace(klass));
                let name = cstr(il2cpp_class_get_name(klass));
                let full = if ns.is_empty() { name } else { format!("{ns}.{name}") };
                class_total += 1;

                let mut iter: *mut c_void = null_mut();
                loop {
                    let m = il2cpp_class_get_methods(klass, &mut iter);
                    if m.is_null() {
                        break;
                    }
                    let mname = cstr(il2cpp_method_get_name(m));
                    writeln!(w, "{full}::{mname}")?;
                    method_total += 1;
                }
            }
        }
    }

    w.flush()?;
    Ok((class_total, method_total))
}
