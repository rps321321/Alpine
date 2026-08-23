use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::path::Path;

fn restore_icon(path: &Path, encoded: &str) {
    let bytes = STANDARD
        .decode(encoded.trim())
        .expect("desktop icon source must be valid base64");

    if std::fs::read(path).is_ok_and(|current| current == bytes) {
        return;
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("desktop icon directory must be writable");
    }
    std::fs::write(path, bytes).expect("desktop icon must be materialized for Tauri");
}

fn main() {
    restore_icon(
        Path::new("icons/icon.png"),
        include_str!("assets/icon.png.base64"),
    );
    restore_icon(
        Path::new("icons/icon.ico"),
        include_str!("assets/icon.ico.base64"),
    );
    tauri_build::build()
}
