//! Stub implementations of every hardware trait.
//!
//! These compile and run today (no board required to verify the loop).
//! As hardware is integrated, each stub is replaced by a real impl —
//! `main.rs` stays unchanged.

use alloc::string::String;
use alloc::vec::Vec;

use defmt::info;

use mymuse_core::app::ArtContext;
use mymuse_core::app::AudioState;
use mymuse_core::app::InputAction;
use mymuse_core::app::PlaybackIntent;
use mymuse_core::draw::DrawCmd;
use mymuse_core::library::AlbumArt;

use mymuse_hal::ArtDecoder;
use mymuse_hal::AudioOutput;
use mymuse_hal::Display;
use mymuse_hal::InputSource;
use mymuse_hal::Storage;
use mymuse_hal::TimeSource;

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

pub struct StubTime {
    now_ms: u64,
}

impl StubTime {
    pub fn new() -> Self {
        Self { now_ms: 0 }
    }
}

impl TimeSource for StubTime {
    fn now_ms(&mut self) -> u64 {
        // Advance by one frame per call. Since the main loop calls this
        // once per iteration, this simulates a 30 fps wall clock.
        self.now_ms += 33;
        self.now_ms
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

pub struct StubInput;

impl InputSource for StubInput {
    fn poll(&mut self) -> Option<InputAction> {
        None
    }
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

pub struct StubAudio;

impl AudioOutput for StubAudio {
    fn execute(&mut self, intent: PlaybackIntent) {
        match intent {
            PlaybackIntent::Play { .. } => info!("audio: play"),
            PlaybackIntent::Pause => info!("audio: pause"),
            PlaybackIntent::Resume => info!("audio: resume"),
            PlaybackIntent::SeekTo(_) => info!("audio: seek"),
        }
    }

    fn state(&self) -> AudioState {
        AudioState::idle()
    }
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

pub struct StubDisplay {
    frame_count: u32,
}

impl StubDisplay {
    pub fn new() -> Self {
        Self { frame_count: 0 }
    }
}

impl Display for StubDisplay {
    fn present(&mut self, cmds: &[DrawCmd], _art: &ArtContext<'_>) {
        self.frame_count += 1;
        // Log every 30th frame (~once per second at 30 fps).
        if self.frame_count % 30 == 0 {
            info!("display: frame {} ({} cmds)", self.frame_count, cmds.len());
        }
    }

    fn measure_text(&self, text: &str, size: u16) -> f32 {
        // Rough approximation: half the font size per character.
        text.chars().count() as f32 * (size as f32 * 0.5)
    }
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

pub struct StubStorage;

impl Storage for StubStorage {
    fn list_files(&mut self, _extensions: &[&str]) -> Result<Vec<(String, u32)>, ()> {
        Ok(Vec::new())
    }
    fn read_file(&mut self, _path: &str, _max_bytes: usize) -> Result<Vec<u8>, ()> {
        Err(())
    }
    fn read_at(&mut self, _path: &str, _offset: u32, _len: usize) -> Result<Vec<u8>, ()> {
        Err(())
    }
    fn write_file(&mut self, _path: &str, _data: &[u8]) -> Result<(), ()> {
        Err(())
    }
    fn exists(&mut self, _path: &str) -> bool {
        false
    }
    fn file_size(&mut self, _path: &str) -> Option<u32> {
        None
    }
}

// ---------------------------------------------------------------------------
// Art decoder
// ---------------------------------------------------------------------------

pub struct StubDecoder;

impl ArtDecoder for StubDecoder {
    fn decode(&mut self, _bytes: &[u8]) -> Result<AlbumArt, ()> {
        Err(())
    }
}
