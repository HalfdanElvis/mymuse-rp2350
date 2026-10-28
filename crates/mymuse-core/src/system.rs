//! Hardware state (battery, bluetooth, headphones) abstraction.
//! Firmware provides an impl; core never reads GPIOs directly.

pub trait SystemInfo {
    /// 0.0 ..= 1.0
    fn battery(&self) -> f32;
    fn bluetooth(&self) -> bool;
    fn headphones(&self) -> bool;
}

pub struct MockSystemInfo {
    pub battery: f32,
    pub bluetooth: bool,
    pub headphones: bool,
}

impl MockSystemInfo {
    pub fn new() -> Self {
        Self { battery: 0.8, bluetooth: false, headphones: false }
    }
}

impl Default for MockSystemInfo {
    fn default() -> Self { Self::new() }
}

impl SystemInfo for MockSystemInfo {
    fn battery(&self) -> f32 { self.battery }
    fn bluetooth(&self) -> bool { self.bluetooth }
    fn headphones(&self) -> bool { self.headphones }
}
