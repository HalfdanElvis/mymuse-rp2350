//! SD-card storage. Scaffold for the real `embedded-sdmmc` implementation.
//!
//! Once the board arrives:
//!   1. Verify the SPI pins for your specific RP2350 board (varies by vendor).
//!   2. Fill in the SPI bus, CS pin, and delay in `SdStorage::new`.
//!   3. Replace `todo!()` in `list_files` / `read_file` / `read_at` / `write_file`
//!      with the `embedded_sdmmc` calls shown in the comments.
//!
//! Everything else in the firmware — `main.rs`, `stubs.rs`, `traits.rs` — is
//! already structured to slot this in without modification.

use alloc::string::String;
use alloc::vec::Vec;

use mymuse_hal::Storage;

#[allow(dead_code)]
pub struct SdStorage {
    // TODO(board): hold SPI peripheral, CS pin, delay timer, and (if you
    // want persistent mounts) the VolumeManager. See embedded-sdmmc's
    // `examples/readme_test.rs` for the exact types.
    _priv: (),
}

#[allow(dead_code)]
impl SdStorage {
    pub fn new() -> Self {
        // TODO(board):  wire up SPI at ~12 MHz, CS on the pin your
        // board exposes (check the schematic — for Waveshare RP2350B
        // modules it's usually GP17 or GP22).
        Self { _priv: () }
    }
}

impl Storage for SdStorage {
    fn list_files(&mut self, _extensions: &[&str]) -> Result<Vec<(String, u32)>, ()> {
        // TODO(board): using embedded_sdmmc:
        //   let sdcard = SdCard::new(spi_device, delay);
        //   let mut volume_mgr = VolumeManager::new(sdcard, MyTimesource);
        //   let volume = volume_mgr.open_volume(VolumeIdx(0))?;
        //   let root = volume.open_root_dir()?;
        //   root.iterate_dir(|entry| { ... recurse via open_dir ... })?;
        todo!("implement with embedded-sdmmc once board is in hand")
    }

    fn read_file(&mut self, _path: &str, _max_bytes: usize) -> Result<Vec<u8>, ()> {
        todo!("open file, read up to max_bytes, return Vec<u8>")
    }

    fn read_at(&mut self, _path: &str, _offset: u32, _len: usize) -> Result<Vec<u8>, ()> {
        todo!("seek + read for cache I/O")
    }

    fn write_file(&mut self, _path: &str, _data: &[u8]) -> Result<(), ()> {
        todo!("used when writing /mymuse/art.cache")
    }

    fn exists(&mut self, _path: &str) -> bool {
        false
    }

    fn file_size(&mut self, _path: &str) -> Option<u32> {
        None
    }
}
