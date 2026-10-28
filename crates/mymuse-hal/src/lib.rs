#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use mymuse_core::app::ArtContext;
use mymuse_core::app::AudioState;
use mymuse_core::app::InputAction;
use mymuse_core::app::PlaybackIntent;
use mymuse_core::draw::DrawCmd;
use mymuse_core::library::AlbumArt;

pub trait TimeSource {
    fn now_ms(&mut self) -> u64;
}

pub trait InputSource {
    fn poll(&mut self) -> Option<InputAction>;
}

pub trait AudioOutput {
    fn execute(&mut self, intent: PlaybackIntent);
    fn state(&self) -> AudioState;
}

pub trait Display {
    fn present(&mut self, cmds: &[DrawCmd], art: &ArtContext<'_>);
    fn measure_text(&self, text: &str, size: u16) -> f32;
}

#[allow(dead_code)]
pub trait Storage {
    fn list_files(&mut self, extensions: &[&str]) -> Result<Vec<(String, u32)>, ()>;
    fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>, ()>;
    fn read_at(&mut self, path: &str, offset: u32, len: usize) -> Result<Vec<u8>, ()>;
    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), ()>;
    fn exists(&mut self, path: &str) -> bool;
    fn file_size(&mut self, path: &str) -> Option<u32>;
}

pub trait ArtDecoder {
    fn decode(&mut self, bytes: &[u8]) -> Result<AlbumArt, ()>;
}
