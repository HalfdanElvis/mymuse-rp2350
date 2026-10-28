//! Hardware abstraction traits.
//!
//! Core (`mymuse-core`) is platform-agnostic. These traits are the seam
//! between core and the RP2350. Stub impls live in `stubs.rs`; real impls
//! replace them once the board arrives.

use alloc::string::String;
use alloc::vec::Vec;

use mymuse_core::app::ArtContext;
use mymuse_core::app::AudioState;
use mymuse_core::app::InputAction;
use mymuse_core::app::PlaybackIntent;
use mymuse_core::draw::DrawCmd;
use mymuse_core::library::AlbumArt;

/// Wall clock in milliseconds since some fixed epoch.
/// Used for scroll animations, hold-to-seek, and slideshow timing.
pub trait TimeSource {
    fn now_ms(&mut self) -> u64;
}

/// Non-blocking poll for user input. Returns `None` if no event
/// occurred this tick.
pub trait InputSource {
    fn poll(&mut self) -> Option<InputAction>;
}

/// Audio device. Executes intents produced by core and reports
/// current state every tick.
pub trait AudioOutput {
    fn execute(&mut self, intent: PlaybackIntent);
    fn state(&self) -> AudioState;
}

/// Framebuffer / display. Consumes `DrawCmd` lists and blits the
/// referenced album art. Also provides text metrics, since font
/// loading belongs here (not in core).
pub trait Display {
    fn present(&mut self, cmds: &[DrawCmd], art: &ArtContext<'_>);
    fn measure_text(&self, text: &str, size: u16) -> f32;
}

/// File system. In production this is `embedded-sdmmc`; in tests it
/// can be a `Vec`-backed mock.
#[allow(dead_code)]
pub trait Storage {
    /// Enumerate all files with one of the given extensions.
    /// Returns `(path, size_in_bytes)`.
    fn list_files(&mut self, extensions: &[&str]) -> Result<Vec<(String, u32)>, ()>;

    /// Read up to `max_bytes` from the start of the file.
    fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>, ()>;

    /// Read `len` bytes starting at `offset`.
    fn read_at(&mut self, path: &str, offset: u32, len: usize) -> Result<Vec<u8>, ()>;

    /// Overwrite (or create) a file with the given bytes.
    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), ()>;

    fn exists(&mut self, path: &str) -> bool;
    fn file_size(&mut self, path: &str) -> Option<u32>;
}

/// JPEG/PNG decoder for album art. Caps output at ~128×128 RGBA
/// to keep memory bounded.
pub trait ArtDecoder {
    fn decode(&mut self, bytes: &[u8]) -> Result<AlbumArt, ()>;
}
