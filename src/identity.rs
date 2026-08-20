use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex_digest(&Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("failed to read identity file {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 8 * 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash identity file {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex_digest(digest.finalize().as_slice()))
}

pub fn tree_sha256(root: &Path, paths: &[PathBuf]) -> Result<String, String> {
    let mut relative_paths = paths
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(root).map_err(|_| {
                format!(
                    "identity file {} is outside root {}",
                    path.display(),
                    root.display()
                )
            })?;
            let relative = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            Ok((relative, path))
        })
        .collect::<Result<Vec<_>, String>>()?;
    relative_paths.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (relative, path) in relative_paths {
        let name = relative.as_bytes();
        let length = u32::try_from(name.len())
            .map_err(|_| format!("identity path is too long: {}", path.display()))?;
        digest.update(length.to_be_bytes());
        digest.update(name);
        let mut file = std::fs::File::open(path)
            .map_err(|error| format!("failed to read identity file {}: {error}", path.display()))?;
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|error| {
                format!("failed to hash identity file {}: {error}", path.display())
            })?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
    }
    Ok(hex_digest(digest.finalize().as_slice()))
}

pub fn runtime_bundle_sha256(server: &Path) -> Result<String, String> {
    let server = std::fs::canonicalize(server).map_err(|error| {
        format!(
            "failed to resolve runtime executable {}: {error}",
            server.display()
        )
    })?;
    if !server.is_file() {
        return Err(format!(
            "runtime executable is not a file: {}",
            server.display()
        ));
    }
    let root = server
        .parent()
        .ok_or_else(|| format!("runtime executable has no parent: {}", server.display()))?;
    let mut files = vec![server.clone()];
    for entry in std::fs::read_dir(root).map_err(|error| {
        format!(
            "failed to enumerate runtime bundle {}: {error}",
            root.display()
        )
    })? {
        let entry =
            entry.map_err(|error| format!("failed to enumerate runtime bundle entry: {error}"))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| format!("failed to inspect runtime bundle entry: {error}"))?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let extension = path
            .extension()
            .map(|value| value.to_string_lossy().to_ascii_lowercase());
        if name == "build-manifest.json"
            || matches!(extension.as_deref(), Some("dll" | "so" | "dylib"))
            || name.contains(".so.")
        {
            files.push(path);
        }
    }
    tree_sha256(root, &files)
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_bundle_tracks_server_and_shared_libraries_only() {
        let directory = tempfile::tempdir().unwrap();
        let server = directory.path().join("llama-server.exe");
        let library = directory.path().join("llama-server-impl.dll");
        let unrelated = directory.path().join("llama-cli.exe");
        let notes = directory.path().join("README.txt");
        std::fs::write(&server, b"server-v1").unwrap();
        std::fs::write(&library, b"library-v1").unwrap();
        std::fs::write(&unrelated, b"cli-v1").unwrap();
        std::fs::write(&notes, b"notes-v1").unwrap();

        let initial = runtime_bundle_sha256(&server).unwrap();
        std::fs::write(&library, b"library-v2").unwrap();
        let library_changed = runtime_bundle_sha256(&server).unwrap();
        assert_ne!(initial, library_changed);

        std::fs::write(&unrelated, b"cli-v2").unwrap();
        std::fs::write(&notes, b"notes-v2").unwrap();
        assert_eq!(library_changed, runtime_bundle_sha256(&server).unwrap());

        std::fs::write(&server, b"server-v2").unwrap();
        assert_ne!(library_changed, runtime_bundle_sha256(&server).unwrap());
    }
}
