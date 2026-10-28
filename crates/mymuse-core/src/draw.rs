//! Draw commands emitted by the UI logic.
//!
//! Firmware executes these against a framebuffer. Core never touches pixels.
//! Coordinates are in logical pixels; the framebuffer's physical size may
//! differ (scaling handled on the firmware side).

use alloc::string::String;

/// A single drawing instruction. Produced by `draw_screen()`, consumed by
/// the firmware's renderer.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCmd {
    /// Filled rectangle. `radius == 0` means square corners.
    Rect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: u32, // 0xAARRGGBB
        radius: u32,
    },

    /// Vertical linear gradient (top → bottom).
    GradientRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color_start: u32,
        color_end: u32,
    },

    /// Text string. Firmware owns the font.
    /// `clip_left`/`clip_right` bound horizontal drawing (useful for scrolling).
    Text {
        x: i32,
        y: i32,
        text: String,
        color: u32,
        size: u16, // font size in logical px
        clip_left: i32,
        clip_right: i32,
    },

    /// Filled triangle pointing right (the play/pause glyph).
    Triangle {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u32,
        color: u32,
    },

    /// The "previous" album art during a crossfade. Firmware knows what art
    /// this refers to (it holds the frame state); it just draws here with the
    /// given fade alpha (0 = invisible, 255 = fully opaque).
    AlbumArtPrev {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        alpha: u8,
    },

    /// Album art with crossfade progress. Firmware holds both previous and
    /// current art; it interpolates between them per pixel.
    AlbumArt {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        progress: f32, // 0.0 = fully previous, 1.0 = fully current
    },

    /// The small thumbnail next to the now-playing text.
    NowArtThumb { x: i32, y: i32, w: u32, h: u32 },
}

/// Convenience constructors — they make `draw_screen` readable.
impl DrawCmd {
    pub fn rect(x: i32, y: i32, w: u32, h: u32, color: u32) -> Self {
        Self::Rect {
            x,
            y,
            w,
            h,
            color,
            radius: 0,
        }
    }

    pub fn rect_rounded(x: i32, y: i32, w: u32, h: u32, color: u32, radius: u32) -> Self {
        Self::Rect {
            x,
            y,
            w,
            h,
            color,
            radius,
        }
    }

    pub fn text(
        x: i32,
        y: i32,
        text: impl Into<String>,
        color: u32,
        size: u16,
        clip_left: i32,
        clip_right: i32,
    ) -> Self {
        Self::Text {
            x,
            y,
            text: text.into(),
            color,
            size,
            clip_left,
            clip_right,
        }
    }
}
