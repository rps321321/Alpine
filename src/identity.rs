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

fn hex_digest(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}
