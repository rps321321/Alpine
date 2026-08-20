use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct InterprocessLock {
    file: File,
}

impl InterprocessLock {
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create lock directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let started = Instant::now();
        loop {
            let last_error = match OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)
            {
                Ok(file) => match FileExt::try_lock_exclusive(&file) {
                    Ok(()) => return Ok(Self { file }),
                    Err(error) => error,
                },
                Err(error) => error,
            };
            if started.elapsed() >= timeout {
                return Err(format!(
                    "timed out waiting for interprocess lock {}: {}",
                    path.display(),
                    last_error
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for InterprocessLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn lock_is_exclusive_and_recovers_after_release() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capacity.lease");
        let owner = InterprocessLock::acquire(&path, Duration::from_secs(1)).unwrap();
        assert!(
            InterprocessLock::acquire(&path, Duration::from_millis(50))
                .unwrap_err()
                .contains("timed out")
        );
        drop(owner);
        InterprocessLock::acquire(&path, Duration::from_secs(1)).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn lock_interoperates_with_the_migrating_powershell_owner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capacity.lease");
        let rendered = path.to_string_lossy().replace('\'', "''");
        let owner = InterprocessLock::acquire(&path, Duration::from_secs(1)).unwrap();
        let blocked = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "$ErrorActionPreference='Stop'; try {{ $h=[IO.File]::Open('{rendered}',[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None); $h.Dispose(); exit 1 }} catch [IO.IOException] {{ exit 0 }}"
                ),
            ])
            .status()
            .unwrap();
        assert!(blocked.success());
        drop(owner);

        let available = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "$h=[IO.File]::Open('{rendered}',[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None); $h.Dispose()"
                ),
            ])
            .status()
            .unwrap();
        assert!(available.success());
    }
}
