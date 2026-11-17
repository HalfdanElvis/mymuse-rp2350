//! Stub implementations of hardware traits used by the firmware.

use alloc::string::String;
use alloc::vec::Vec;

use embedded_hal::digital::InputPin;

use mymuse_core::app::{ArtContext, AudioState, InputAction, PlaybackIntent};
use mymuse_core::draw::DrawCmd;
use mymuse_core::library::AlbumArt;

use mymuse_hal::{ArtDecoder, AudioOutput, Display, InputSource, Storage, TimeSource};

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
        self.now_ms = self.now_ms.saturating_add(10);
        self.now_ms
    }
}

// ---------------------------------------------------------------------------
// Debounced, edge-triggered D-pad input
// ---------------------------------------------------------------------------

const DEBOUNCE_POLLS: u8 = 3;

/// A button must be observed in the same state for several polls before its
/// stable state changes. `update` returns true only on a new press edge.
#[derive(Default)]
struct ButtonState {
    stable_pressed: bool,
    candidate_pressed: bool,
    candidate_polls: u8,
}

impl ButtonState {
    fn update(&mut self, pressed: bool) -> bool {
        if pressed == self.candidate_pressed {
            self.candidate_polls = self.candidate_polls.saturating_add(1);
        } else {
            self.candidate_pressed = pressed;
            self.candidate_polls = 1;
        }

        if self.candidate_polls >= DEBOUNCE_POLLS && self.stable_pressed != self.candidate_pressed {
            self.stable_pressed = self.candidate_pressed;
            return self.stable_pressed;
        }

        false
    }
}

pub struct StubInput<U, D, L, R, M> {
    up: U,
    down: D,
    left: L,
    right: R,
    mid: M,

    up_state: ButtonState,
    down_state: ButtonState,
    left_state: ButtonState,
    right_state: ButtonState,
    mid_state: ButtonState,
}

impl<U, D, L, R, M> StubInput<U, D, L, R, M>
where
    U: InputPin,
    D: InputPin,
    L: InputPin,
    R: InputPin,
    M: InputPin,
{
    pub fn new(up: U, down: D, left: L, right: R, mid: M) -> Self {
        Self {
            up,
            down,
            left,
            right,
            mid,
            up_state: ButtonState::default(),
            down_state: ButtonState::default(),
            left_state: ButtonState::default(),
            right_state: ButtonState::default(),
            mid_state: ButtonState::default(),
        }
    }
}

impl<U, D, L, R, M> InputSource for StubInput<U, D, L, R, M>
where
    U: InputPin,
    D: InputPin,
    L: InputPin,
    R: InputPin,
    M: InputPin,
{
    fn poll(&mut self) -> Option<InputAction> {
        // Buttons are wired active-low with pull-up inputs.
        let up_pressed = self.up.is_low().unwrap_or(false);
        let down_pressed = self.down.is_low().unwrap_or(false);
        let left_pressed = self.left.is_low().unwrap_or(false);
        let right_pressed = self.right.is_low().unwrap_or(false);
        let mid_pressed = self.mid.is_low().unwrap_or(false);

        // Sample every button on every poll so release edges are also seen.
        let up_edge = self.up_state.update(up_pressed);
        let down_edge = self.down_state.update(down_pressed);
        let left_edge = self.left_state.update(left_pressed);
        let right_edge = self.right_state.update(right_pressed);
        let mid_edge = self.mid_state.update(mid_pressed);

        // One event per physical press. Holding a button does not repeatedly
        // move the selection to the beginning or end of the list.
        if up_edge {
            Some(InputAction::Up)
        } else if down_edge {
            Some(InputAction::Down)
        } else if left_edge {
            Some(InputAction::Back)
        } else if right_edge {
            Some(InputAction::Enter)
        } else if mid_edge {
            Some(InputAction::TogglePause)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

pub struct StubAudio;

impl AudioOutput for StubAudio {
    fn execute(&mut self, _intent: PlaybackIntent) {}

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
    fn present(&mut self, _cmds: &[DrawCmd], _art: &ArtContext<'_>) {
        self.frame_count = self.frame_count.saturating_add(1);
    }

    fn measure_text(&self, text: &str, size: u16) -> f32 {
        text.chars().count() as f32 * (size as f32 * 0.5)
    }
}

// ---------------------------------------------------------------------------
// Storage stub
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
// Art decoder stub
// ---------------------------------------------------------------------------

pub struct StubDecoder;

impl ArtDecoder for StubDecoder {
    fn decode(&mut self, _bytes: &[u8]) -> Result<AlbumArt, ()> {
        Err(())
    }
}
