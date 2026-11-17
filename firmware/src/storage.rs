use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::ops::ControlFlow;

use embedded_io::{Read, Write};
use embedded_sdmmc::{
    BlockDevice, DirEntry, Directory, Error as SdError, File, LfnBuffer, Mode, TimeSource,
    VolumeIdx, VolumeManager,
};

use mymuse_hal::Storage;

const MAX_OPEN_DIRS: usize = 4;
const MAX_OPEN_FILES: usize = 4;
const MAX_OPEN_VOLUMES: usize = 1;

// Large enough for typical FAT long filenames.
const LFN_BUFFER_SIZE: usize = 256;

pub struct SdStorage<D, T>
where
    D: BlockDevice,
    T: TimeSource,
{
    volume_mgr: VolumeManager<D, T, MAX_OPEN_DIRS, MAX_OPEN_FILES, MAX_OPEN_VOLUMES>,
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
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root = volume.open_root_dir().map_err(|_| ())?;

        let mut files = Vec::new();

        walk_dir(&root, "", extensions, &mut files).map_err(|_| ())?;

        files.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(files)
    }

    fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>, ()> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root = volume.open_root_dir().map_err(|_| ())?;

        let parts = split_path(path);
        let (parent_parts, filename) = parts.split_at(parts.len().checked_sub(1).ok_or(())?);

        let file = if parent_parts.is_empty() {
            open_read_file(&root, filename[0]).map_err(|_| ())?
        } else {
            let parent = open_dir_path(&root, parent_parts).map_err(|_| ())?;
            open_read_file(&parent, filename[0]).map_err(|_| ())?
        };

        let mut output = Vec::new();
        let mut chunk = [0u8; 512];

        while output.len() < max_bytes {
            let count = file.read(&mut chunk).map_err(|_| ())?;

            if count == 0 {
                break;
            }

            let remaining = max_bytes - output.len();
            let take = core::cmp::min(count, remaining);

            output.extend_from_slice(&chunk[..take]);
        }

        Ok(output)
    }

    fn read_at(&mut self, path: &str, offset: u32, len: usize) -> Result<Vec<u8>, ()> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root = volume.open_root_dir().map_err(|_| ())?;

        let parts = split_path(path);
        let (parent_parts, filename) = parts.split_at(parts.len().checked_sub(1).ok_or(())?);

        let mut file = if parent_parts.is_empty() {
            open_read_file(&root, filename[0]).map_err(|_| ())?
        } else {
            let parent = open_dir_path(&root, parent_parts).map_err(|_| ())?;
            open_read_file(&parent, filename[0]).map_err(|_| ())?
        };

        file.seek_from_start(offset).map_err(|_| ())?;

        let mut output = vec![0u8; len];
        let mut bytes_read = 0;

        while bytes_read < len {
            let count = file.read(&mut output[bytes_read..]).map_err(|_| ())?;

            if count == 0 {
                break;
            }

            bytes_read += count;
        }

        output.truncate(bytes_read);
        Ok(output)
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), ()> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
        let root = volume.open_root_dir().map_err(|_| ())?;

        let parts = split_path(path);
        let (parent_parts, filename) = parts.split_at(parts.len().checked_sub(1).ok_or(())?);

        let mut file = if parent_parts.is_empty() {
            root.open_file_in_dir(filename[0], Mode::ReadWriteCreateOrTruncate)
                .map_err(|_| ())?
        } else {
            let parent = open_dir_path(&root, parent_parts).map_err(|_| ())?;
            parent
                .open_file_in_dir(filename[0], Mode::ReadWriteCreateOrTruncate)
                .map_err(|_| ())?
        };

        file.write_all(data).map_err(|_| ())?;
        file.flush().map_err(|_| ())?;

        Ok(())
    }

    fn exists(&mut self, path: &str) -> bool {
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            return false;
        };

        let Ok(root) = volume.open_root_dir() else {
            return false;
        };

        let parts = split_path(path);

        let Some((&filename, parent_parts)) = parts.split_last() else {
            return false;
        };

        if parent_parts.is_empty() {
            open_read_file(&root, filename).is_ok()
        } else {
            let Ok(parent) = open_dir_path(&root, parent_parts) else {
                return false;
            };

            open_read_file(&parent, filename).is_ok()
        }
    }

    fn file_size(&mut self, path: &str) -> Option<u32> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).ok()?;
        let root = volume.open_root_dir().ok()?;

        let parts = split_path(path);
        let (parent_parts, filename) = parts.split_at(parts.len().checked_sub(1)?);

        let file = if parent_parts.is_empty() {
            open_read_file(&root, filename[0]).ok()?
        } else {
            let parent = open_dir_path(&root, parent_parts).ok()?;
            open_read_file(&parent, filename[0]).ok()?
        };

        Some(file.length())
    }
}

// -----------------------------------------------------------------------------
// Directory scanning
// -----------------------------------------------------------------------------

fn walk_dir<D, T>(
    dir: &Directory<'_, D, T, MAX_OPEN_DIRS, MAX_OPEN_FILES, MAX_OPEN_VOLUMES>,
    current_path: &str,
    extensions: &[&str],
    output: &mut Vec<(String, u32)>,
) -> Result<(), SdError<D::Error>>
where
    D: BlockDevice,
    T: TimeSource,
{
    // Do not call any SD-card methods inside the iterate callback.
    // The VolumeManager is locked while this callback is running.
    let mut entries: Vec<(DirEntry, Option<String>)> = Vec::new();

    let mut lfn_storage = [0u8; LFN_BUFFER_SIZE];
    let mut lfn_buffer = LfnBuffer::new(&mut lfn_storage);

    dir.iterate_dir_lfn(&mut lfn_buffer, |entry, long_name| {
        entries.push((entry.clone(), long_name.map(ToString::to_string)));
        ControlFlow::Continue(())
    })?;

    for (entry, long_name) in entries {
        let short_name = entry.name.to_string();
        let name = long_name.unwrap_or_else(|| short_name.clone());

        if name == "." || name == ".." {
            continue;
        }

        if name.starts_with('.') {
            continue;
        }

        if name.eq_ignore_ascii_case("System Volume Information")
            || name.eq_ignore_ascii_case("SYSTEM~1")
            || name.eq_ignore_ascii_case("LOST.DIR")
        {
            continue;
        }

        let full_path = if current_path.is_empty() {
            name.clone()
        } else {
            alloc::format!("{}/{}", current_path, name)
        };

        if entry.attributes.is_directory() {
            // Use the short name to open the directory. This works even
            // when the directory's displayed name is a long filename.
            let subdir = dir.open_dir(entry.name)?;

            walk_dir(&subdir, &full_path, extensions, output)?;
        } else {
            let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();

            let matches = extensions
                .iter()
                .any(|ext| ext.trim_start_matches('.').eq_ignore_ascii_case(&extension));

            if matches {
                output.push((full_path, entry.size));
            }
        }
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// Path handling
// -----------------------------------------------------------------------------

fn split_path(path: &str) -> Vec<&str> {
    path.trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect()
}

// Find the short FAT name for a path component.
// The returned short name can safely be passed to open_dir().
fn find_short_name<D, T>(
    dir: &Directory<'_, D, T, MAX_OPEN_DIRS, MAX_OPEN_FILES, MAX_OPEN_VOLUMES>,
    requested_name: &str,
) -> Result<embedded_sdmmc::ShortFileName, SdError<D::Error>>
where
    D: BlockDevice,
    T: TimeSource,
{
    let mut lfn_storage = [0u8; LFN_BUFFER_SIZE];
    let mut lfn_buffer = LfnBuffer::new(&mut lfn_storage);

    let mut found = None;

    dir.iterate_dir_lfn(&mut lfn_buffer, |entry, long_name| {
        let short_name = entry.name.to_string();

        let matches_short = short_name.eq_ignore_ascii_case(requested_name);
        let matches_long = long_name
            .map(|name| name.eq_ignore_ascii_case(requested_name))
            .unwrap_or(false);

        if matches_short || matches_long {
            found = Some(entry.name.clone());
            return ControlFlow::Break(());
        }

        ControlFlow::Continue(())
    })?;

    found.ok_or(SdError::NotFound)
}

// Open a nested directory path by resolving each component to its
// short FAT filename before opening it.
fn open_dir_path<'a, D, T>(
    dir: &Directory<'a, D, T, MAX_OPEN_DIRS, MAX_OPEN_FILES, MAX_OPEN_VOLUMES>,
    components: &[&str],
) -> Result<Directory<'a, D, T, MAX_OPEN_DIRS, MAX_OPEN_FILES, MAX_OPEN_VOLUMES>, SdError<D::Error>>
where
    D: BlockDevice,
    T: TimeSource,
{
    if components.is_empty() {
        return Err(SdError::NotFound);
    }

    let short_name = find_short_name(dir, components[0])?;
    let child = dir.open_dir(short_name)?;

    if components.len() == 1 {
        return Ok(child);
    }

    open_dir_path(&child, &components[1..])
}

// Open a file by its long name first, then try its short FAT name.
// The second attempt is useful for files without an LFN entry.
fn open_read_file<'a, D, T>(
    dir: &Directory<'a, D, T, MAX_OPEN_DIRS, MAX_OPEN_FILES, MAX_OPEN_VOLUMES>,
    filename: &str,
) -> Result<File<'a, D, T, MAX_OPEN_DIRS, MAX_OPEN_FILES, MAX_OPEN_VOLUMES>, SdError<D::Error>>
where
    D: BlockDevice,
    T: TimeSource,
{
    match dir.open_long_name_file_in_dir(filename, Mode::ReadOnly) {
        Ok(file) => Ok(file),
        Err(_) => dir.open_file_in_dir(filename, Mode::ReadOnly),
    }
}

// -----------------------------------------------------------------------------
// Dummy FAT timestamp source
// -----------------------------------------------------------------------------

use embedded_sdmmc::Timestamp;

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
