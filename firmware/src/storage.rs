use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::ops::ControlFlow;

use mymuse_hal::Storage;

use embedded_io::Write;
use embedded_sdmmc::{
    BlockDevice, Directory, Error as SdError, Mode, TimeSource, Timestamp, VolumeIdx, VolumeManager,
};

pub struct SdStorage<D, T>
where
    D: BlockDevice,
    T: TimeSource,
{
    volume_mgr: VolumeManager<D, T, 4, 4, 1>,
}

impl<D, T> SdStorage<D, T>
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

impl<D, T> Storage for SdStorage<D, T>
where
    D: BlockDevice,
    T: TimeSource,
{
    fn list_files(&mut self, extensions: &[&str]) -> Result<Vec<(String, u32)>, ()> {
        let mut out = Vec::new();
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root_dir = volume.open_root_dir().map_err(|_| ())?;

        walk_dir(&root_dir, "", extensions, &mut out).map_err(|_| ())?;
        out.sort();
        Ok(out)
    }

    fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>, ()> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root_dir = volume.open_root_dir().map_err(|_| ())?;
        let file = root_dir
            .open_file_in_dir(path, Mode::ReadOnly)
            .map_err(|_| ())?;

        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            let n = file.read(&mut chunk).map_err(|_| ())?;
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
        let file = root_dir
            .open_file_in_dir(path, Mode::ReadOnly)
            .map_err(|_| ())?;

        file.seek_from_start(offset).map_err(|_| ())?;

        let mut buf = vec![0u8; len];
        let mut read_bytes = 0;
        while read_bytes < len {
            let n = file.read(&mut buf[read_bytes..]).map_err(|_| ())?;
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

        let mut file = root_dir
            .open_file_in_dir(path, Mode::ReadWriteCreateOrTruncate)
            .map_err(|_| ())?;

        file.write_all(data).map_err(|_| ())?;
        file.flush().map_err(|_| ())?;
        Ok(())
    }

    fn exists(&mut self, path: &str) -> bool {
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            return false;
        };
        let Ok(root_dir) = volume.open_root_dir() else {
            return false;
        };
        root_dir.open_file_in_dir(path, Mode::ReadOnly).is_ok()
    }

    fn file_size(&mut self, path: &str) -> Option<u32> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).ok()?;
        let root_dir = volume.open_root_dir().ok()?;
        let file = root_dir.open_file_in_dir(path, Mode::ReadOnly).ok()?;
        Some(file.length())
    }
}

fn walk_dir<D, T>(
    dir: &Directory<'_, D, T, 4, 4, 1>,
    current_path: &str,
    exts: &[&str],
    out: &mut Vec<(String, u32)>,
) -> Result<(), SdError<D::Error>>
where
    D: BlockDevice,
    T: TimeSource,
{
    dir.iterate_dir(|entry| {
        if entry.attributes.is_directory() {
            let name = entry.name.to_string();
            if name == "." || name == ".." {
                return ControlFlow::Continue(());
            }
            let subdir_path = if current_path.is_empty() {
                name
            } else {
                format!("{}/{}", current_path, name)
            };

            // Pass entry.name directly (it implements ToShortFileName)
            if let Ok(subdir) = dir.open_dir(entry.name) {
                let _ = walk_dir(&subdir, &subdir_path, exts, out);
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
        ControlFlow::Continue(())
    })
}

// Add this at the bottom of storage.rs to satisfy the TimeSource requirement

pub struct DummyTimeSource;

impl TimeSource for DummyTimeSource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 0,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}
