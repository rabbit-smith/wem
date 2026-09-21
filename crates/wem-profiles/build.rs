use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("profile directory reads") {
        let entry = entry.expect("profile directory entry reads");
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else if path.is_file() {
            assert!(
                path.strip_prefix(root).is_ok(),
                "profile resource remains under its root"
            );
            files.push(path);
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let profiles_dir = manifest_dir.join("../../src/wwise_wem/data/profiles");
    let index = profiles_dir.join("index.json");
    assert!(
        index.is_file(),
        "profile index must exist at {}",
        index.display()
    );

    let mut files = Vec::new();
    collect_files(&profiles_dir, &profiles_dir, &mut files);
    files.sort();

    let mut source = String::from("pub static EMBEDDED_INDEX: &[u8] = include_bytes!(");
    source.push_str(&format!("{:?}", index));
    source.push_str(");\n");
    source.push_str("pub static EMBEDDED_FILES: &[(&str, &[u8])] = &[\n");
    for path in files {
        if path == index {
            continue;
        }
        let logical = path
            .strip_prefix(&profiles_dir)
            .expect("profile resource has logical path")
            .to_string_lossy()
            .replace('\\', "/");
        source.push_str(&format!("    ({logical:?}, include_bytes!({:?})),\n", path));
        println!("cargo:rerun-if-changed={}", path.display());
    }
    source.push_str("];\n");
    println!("cargo:rerun-if-changed={}", index.display());

    let output = PathBuf::from(env::var("OUT_DIR").expect("out dir")).join("embedded_profiles.rs");
    fs::write(output, source).expect("embedded profile source writes");
}
