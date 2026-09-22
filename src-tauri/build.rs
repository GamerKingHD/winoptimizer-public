use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(mobile)");
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let frontend_dist = manifest_dir.join("../.build/frontend");
    let tauri_conf = manifest_dir.join("tauri.conf.json");
    let capabilities = manifest_dir.join("capabilities");
    let embed_hash = hash_embed_inputs(&frontend_dist, &tauri_conf, &capabilities);

    println!("cargo:rerun-if-changed={}", frontend_dist.display());
    println!("cargo:rerun-if-changed={}", tauri_conf.display());
    println!("cargo:rerun-if-changed={}", capabilities.display());

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let stamp = out_dir.join("frontend-embed.sha256");
    let previous = fs::read_to_string(&stamp).ok();
    if previous.as_deref() == Some(embed_hash.as_str()) {
        return;
    }

    tauri_build::build();
    if let Ok(mut file) = fs::File::create(&stamp) {
        let _ = file.write_all(embed_hash.as_bytes());
    }
}

fn hash_embed_inputs(frontend_dist: &Path, tauri_conf: &Path, capabilities: &Path) -> String {
    let mut files = BTreeMap::new();
    collect_files(frontend_dist, &mut files);
    collect_files(tauri_conf, &mut files);
    collect_files(capabilities, &mut files);
    let mut hasher = DefaultHasher::new();
    for (path, bytes) in files {
        path.hash(&mut hasher);
        bytes.len().hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn collect_files(root: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    if !root.exists() {
        return;
    }
    if root.is_file() {
        if let Ok(bytes) = fs::read(root) {
            files.insert(root.to_string_lossy().replace('\\', "/"), bytes);
        }
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        collect_files(&entry.path(), files);
    }
}
