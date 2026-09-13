//! 發版用小工具：對一或多個檔案算 blake3，輸出自動更新器要對照的 `blake3.json`。
//!
//! 用法：
//!   cargo run -p blake3json --release -- <檔案> [更多檔案...] [-o 輸出路徑]
//!
//! 例（在 repo 根目錄，對要上傳當 release asset 的 version.dll）：
//!   cargo run -p blake3json --release -- dist\version.dll
//!
//! 產生的 blake3.json 長這樣（key 是檔名，updater 只會看 `version.dll` 這一筆）：
//!   {
//!     "version.dll": "e3b0c44298fc1c149afbf4c8996fb924..."
//!   }
//!
//! 把這個 blake3.json 跟 version.dll 一起附到 GitHub release 即可。

use std::{fs, path::Path, process::ExitCode};

fn main() -> ExitCode {
    let mut inputs: Vec<String> = Vec::new();
    let mut out_path = String::from("blake3.json");

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--output" => {
                let Some(v) = args.next() else {
                    eprintln!("error: {arg} needs a path");
                    return ExitCode::FAILURE;
                };
                out_path = v;
            }
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            _ => inputs.push(arg),
        }
    }

    if inputs.is_empty() {
        print_usage();
        return ExitCode::FAILURE;
    }

    // 自己拼 JSON，省一個相依。key 用檔名（basename），value 是 blake3 十六進位。
    let mut entries: Vec<(String, String)> = Vec::with_capacity(inputs.len());
    for input in &inputs {
        let path = Path::new(input);
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            eprintln!("error: bad file name: {input}");
            return ExitCode::FAILURE;
        };
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: can't read {input}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let hash = blake3::hash(&bytes).to_hex().to_string();
        println!("{name}  {hash}");
        entries.push((name.to_string(), hash));
    }

    let mut json = String::from("{\n");
    for (i, (name, hash)) in entries.iter().enumerate() {
        let comma = if i + 1 < entries.len() { "," } else { "" };
        json.push_str(&format!("  {}: {}{}\n", json_string(name), json_string(hash), comma));
    }
    json.push_str("}\n");

    if let Err(e) = fs::write(&out_path, json) {
        eprintln!("error: can't write {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    println!("wrote {out_path}");
    ExitCode::SUCCESS
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn print_usage() {
    eprintln!("usage: cargo run -p blake3json --release -- <file> [more files...] [-o blake3.json]");
}
