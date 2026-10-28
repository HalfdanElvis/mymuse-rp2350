//! SD-card storage implementation using `embedded-sdmmc`.
//!
//! When the board arrives, you will:
//! 1. Initialize the SPI bus at ~12 MHz.
//! 2. Create the `SdCard` with the SPI device and a `Delay`.
//! 3. Pass it into `SdStorage::new(...)`.
//!
//! The `Storage` trait requires `list_files`, which does a full recursive
//! directory walk. Because `embedded-sdmmc`'s directory handles are tightly
//! scoped (you can't hold a reference to a parent while recursing), we use
//! a stack-based approach that opens and closes subdirectories as it goes.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use mymuse_hal::Storage;

use embedded_sdmmc::{
    BlockDevice, Directory, Error as SdError, Mode, TimeSource, VolumeIdx, VolumeManager,
};

pub struct SdStorage<D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize>
where
    D: BlockDevice,
    T: TimeSource,
{
    volume_mgr: VolumeManager<D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
}

impl<D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize>
    SdStorage<D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>
where
    D: BlockDevice,
    T: TimeSource,
{
    pub fn new(block_device: D, time_source: T) -> Self {
        Self {
            volume_mgr: VolumeManager::new(block_device, time_source),
        }
    }
}

impl<D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize> Storage
    for SdStorage<D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>
where
    D: BlockDevice,
    T: TimeSource,
{
    fn list_files(&mut self, extensions: &[&str]) -> Result<Vec<(String, u32)>, ()> {
        let mut out = Vec::new();
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root_dir = volume.open_root_dir().map_err(|_| ())?;
        
        walk_dir(&mut self.volume_mgr, &root_dir, "", extensions, &mut out).map_err(|_| ())?;
        out.sort();
        Ok(out)
    }

    fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>, ()> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root_dir = volume.open_root_dir().map_err(|_| ())?;
        let mut file = self
            .volume_mgr
            .open_file_in_dir(root_dir, path, Mode::ReadOnly)
            .map_err(|_| ())?;

        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            let n = self.volume_mgr.read(&mut file, &mut chunk).map_err(|_| ())?;
            if n == 0 {
                break;
            }
            let take = core::cmp::min(n, max_bytes.saturating_sub(buf.len()));
            buf.extend_from_slice(&chunk[..take]);
            if buf.len() >= max_bytes {
                break;
            }
        }
        Ok(buf)
    }

    fn read_at(&mut self, path: &str, offset: u32, len: usize) -> Result<Vec<u8>, ()> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root_dir = volume.open_root_dir().map_err(|_| ())?;
        let mut file = self
            .volume_mgr
            .open_file_in_dir(root_dir, path, Mode::ReadOnly)
            .map_err(|_| ())?;

        self.volume_mgr
            .seek(&mut file, embedded_sdmmc::SeekFrom::Start(offset))
            .map_err(|_| ())?;

        let mut buf = vec![0u8; len];
        let mut read_bytes = 0;
        while read_bytes < len {
            let n = self
                .volume_mgr
                .read(&mut file, &mut buf[read_bytes..])
                .map_err(|_| ())?;
            if n == 0 {
                break;
            }
            read_bytes += n;
        }
        buf.truncate(read_bytes);
        Ok(buf)
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), ()> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root_dir = volume.open_root_dir().map_err(|_| ())?;
        
        // Note: embedded-sdmmc's Mode::ReadWrite creates or overwrites depending on config
        let file = self
            .volume_mgr
            .open_file_in_dir(root_dir, path, Mode::ReadWriteCreateOrTruncate)
            .map_err(|_| ())?;

        self.volume_mgr.write(&mut file, data).map_err(|_| ())?;
        Ok(())
    }

    fn exists(&mut self, path: &str) -> bool {
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            return false;
        };
        let Ok(root_dir) = volume.open_root_dir() else {
            return false;
        };
        self.volume_mgr
            .open_file_in_dir(root_dir, path, Mode::ReadOnly)
            .is_ok()
    }

    fn file_size(&mut self, path: &str) -> Option<u32> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).ok()?;
        let root_dir = volume.open_root_dir().ok()?;
        let file = self
            .volume_mgr
            .open_file_in_dir(root_dir, path, Mode::ReadOnly)
            .ok()?;
        Some(file.length())
    }
}

/// Recursive directory walker.
fn walk_dir<D, T, const MAX_DIRS: usize, const MAX_FILES: usize, const MAX_VOLUMES: usize>(
    volume_mgr: &mut VolumeManager<D, T, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    dir: &Directory,
    current_path: &str,
    exts: &[&str],
    out: &mut Vec<(String, u32)>,
) -> Result<(), SdError<D::Error>>
where
    D: BlockDevice,
    T: TimeSource,
{
    volume_mgr.iterate_dir(dir, |entry| {
        if entry.attributes.contains(embedded_sdmmc::Attributes::DIRECTORY) {
            if entry.name.to_string() == "." || entry.name.to_string() == ".." {
                return;
            }
            let subdir_path = if current_path.is_empty() {
                entry.name.to_string()
            } else {
                format!("{}/{}", current_path, entry.name)
            };

            // Open subdirectory, recurse, then it gets dropped
            if let Ok(subdir) = volume_mgr.open_dir(dir, &entry.name.to_string()) {
                let _ = walk_dir(volume_mgr, &subdir, &subdir_path, exts, out);
            }
        } else {
            let name = entry.name.to_string();
            let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
            if exts.contains(&ext.as_str()) {
                let file_path = if current_path.is_empty() {
                    name
                } else {
                    format!("{}/{}", current_path, name)
                };
                out.push((file_path, entry.size));
            }
        }
    })
}
