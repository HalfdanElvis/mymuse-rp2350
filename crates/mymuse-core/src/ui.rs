//! Pure UI logic: state → list of draw commands.
//!
//! This is the direct port of the original `ui.lua::draw_screen`.
//! It contains **no rendering** — the firmware executes the commands.
//! Text measurement is injected so the font library stays in firmware.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::draw::DrawCmd;
use crate::theme::ThemeColors;

// ============================================================================
// Layout constants (ported 1:1 from ui.lua CONFIG)
// ============================================================================

const SCREEN_W: i32 = 320;
const SCREEN_H: i32 = 240;
const ART_PANEL_W: i32 = 144;
const LIST_PAD_X: i32 = 6;
const HEADER_PAD_X: i32 = 10;
const ITEM_FONT: u16 = 10;
const HEADER_FONT: u16 = 12;
const NOWPLAY_TITLE_FONT: u16 = 10;
const NOWPLAY_ARTIST_FONT: u16 = 8;
const LIST_SCROLL_GAP: f32 = 30.0;
const NOWPLAY_SCROLL_GAP: f32 = 20.0;
const HEADER_SCROLL_GAP: f32 = 30.0;
const ARTIST_SCROLL_GAP: f32 = 20.0;
const VISIBLE_ROWS: i32 = 7;

const LIST_W: i32 = SCREEN_W - ART_PANEL_W; // 176
// const LIST_ITEM_MAX_WIDTH: f32 = (LIST_W - 2 * LIST_PAD_X) as f32; // 164
const NOWPLAY_MAX_WIDTH: f32 = 240.0;
const HEADER_MAX_WIDTH: f32 = (SCREEN_W - 2 * HEADER_PAD_X) as f32; // 300
const NOWPLAY_ARTIST_MAX_WIDTH: f32 = 200.0;

// Inner draw_screen constants
const STATUS_H: i32 = 16;
const HEADER_H: i32 = 22;
const LIST_GAP: i32 = 4;
const NOWPLAY_H: i32 = 44;
const STATUS_PAD_X: i32 = 6;
const STATUS_FONT: u16 = 9;
const STATUS_SLOT_W: i32 = 16;
const BAT_W: i32 = 16;
const BAT_H: i32 = 8;
const ART_SIZE: i32 = 128;
const ART_PAD: i32 = 8;
const ITEM_VPAD: i32 = 4;
const NOWPLAY_PAD: i32 = 6;
const NOWPLAY_ART: i32 = 32;
const PROGRESS_H: i32 = 4;
const FADE_HEIGHT: i32 = 12;

// Scroll math constants
const SCROLL_START_TIME: f32 = 1.0;
const SCROLL_SPEED: f32 = 30.0;
const MIN_LOOP_DISTANCE: f32 = 20.0;

// ============================================================================
// Inputs
// ============================================================================

pub struct NowPlayingView<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub elapsed: f32,
    pub duration: f32,
}

pub struct UiFrame<'a> {
    pub items: &'a [String],
    pub selected_index: usize, // 0-based
    pub scroll_offset: usize,  // 0-based
    pub title: &'a str,
    pub clock: &'a str,
    pub battery: f32,
    pub bluetooth: bool,
    pub headphones: bool,
    pub now: Option<NowPlayingView<'a>>,
    pub has_now_art: bool,
    pub has_pane_art: bool,
    pub has_pane_art_prev: bool,
    pub list_scroll_elapsed: f32,
    pub now_scroll_elapsed: f32,
    pub header_scroll_elapsed: f32,
    pub artist_scroll_elapsed: f32,
    pub is_reversing: bool,
    pub is_paused: bool,
    pub is_settings: bool,
    pub option_values: Option<&'a [String]>,
    pub line_height_ratio: f32,
    pub theme: &'static ThemeColors,
    pub pane_art_progress: f32,
}

// ============================================================================
// Helpers
// ============================================================================

fn fmt_time(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Ported 1:1 from the Lua `scroll_offset`.
fn scroll_offset<F: Fn(&str, u16) -> f32>(
    text: &str,
    font_size: u16,
    max_width: f32,
    gap: f32,
    elapsed: f32,
    pingpong: bool,
    measure: &F,
) -> (f32, f32) {
    if elapsed <= SCROLL_START_TIME {
        return (0.0, 0.0);
    }
    let scroll_time = elapsed - SCROLL_START_TIME;
    let text_w = measure(text, font_size);
    if text_w <= max_width {
        return (0.0, text_w);
    }
    let loop_dist = if pingpong {
        text_w - max_width
    } else {
        text_w + gap
    };
    let pause_mult = if pingpong && loop_dist < MIN_LOOP_DISTANCE {
        1.0 + (1.0 - loop_dist / MIN_LOOP_DISTANCE) * 2.0
    } else {
        1.0
    };
    let dynamic_pause = SCROLL_START_TIME * pause_mult;
    let offset = if pingpong {
        let travel_time = loop_dist / SCROLL_SPEED;
        let cycle = (travel_time + dynamic_pause) * 2.0;
        let t = scroll_time % cycle;
        if t < travel_time {
            t * SCROLL_SPEED
        } else if t < travel_time + dynamic_pause {
            loop_dist
        } else if t < travel_time * 2.0 + dynamic_pause {
            loop_dist - (t - travel_time - dynamic_pause) * SCROLL_SPEED
        } else {
            0.0
        }
    } else {
        (scroll_time * SCROLL_SPEED) % loop_dist
    };
    (offset, text_w)
}

// ============================================================================
// Main entry point
// ============================================================================

pub fn draw_screen<F: Fn(&str, u16) -> f32>(state: &UiFrame, measure: &F) -> Vec<DrawCmd> {
    let mut cmds = Vec::with_capacity(64);
    let theme = state.theme;

    let content_top = STATUS_H + HEADER_H + LIST_GAP;
    let content_bottom = SCREEN_H - NOWPLAY_H;
    let content_h = content_bottom - content_top;
    let item_line_h = ITEM_FONT as f32 * state.line_height_ratio;
    let item_h = (item_line_h as i32) + ITEM_VPAD * 2;

    let selected = state.selected_index;

    // 0. Background
    cmds.push(DrawCmd::rect(
        0,
        0,
        SCREEN_W as u32,
        SCREEN_H as u32,
        theme.bg,
    ));

    // === 1. Status bar ===
    cmds.push(DrawCmd::rect(
        0,
        0,
        SCREEN_W as u32,
        STATUS_H as u32,
        theme.status_bg,
    ));
    let status_text_y = (STATUS_H - STATUS_FONT as i32) / 2;
    cmds.push(DrawCmd::text(
        STATUS_PAD_X,
        status_text_y,
        state.clock,
        theme.status_text,
        STATUS_FONT,
        0,
        SCREEN_W,
    ));

    // Battery icon (right side)
    let right = SCREEN_W - STATUS_PAD_X;
    let bat_x = right - BAT_W - 2;
    let bat_y = (STATUS_H - BAT_H) / 2;
    cmds.push(DrawCmd::rect(
        bat_x,
        bat_y,
        BAT_W as u32,
        BAT_H as u32,
        theme.status_text,
    ));
    cmds.push(DrawCmd::rect(
        bat_x + 1,
        bat_y + 1,
        (BAT_W - 2) as u32,
        (BAT_H - 2) as u32,
        theme.status_bg,
    ));
    let pct = state.battery.clamp(0.0, 1.0);
    let fill_w = ((BAT_W - 2) as f32 * pct) as i32;
    if fill_w > 0 {
        cmds.push(DrawCmd::rect(
            bat_x + 1,
            bat_y + 1,
            fill_w as u32,
            (BAT_H - 2) as u32,
            theme.battery,
        ));
    }
    // Battery nub
    cmds.push(DrawCmd::rect(
        bat_x + BAT_W,
        bat_y + 2,
        2,
        (BAT_H - 4) as u32,
        theme.status_text,
    ));

    // Battery percentage
    let pct_str = format!("{}%", (pct * 100.0 + 0.5) as i32);
    let pct_w = measure(&pct_str, STATUS_FONT);
    let pct_x = bat_x - 4 - pct_w as i32;
    cmds.push(DrawCmd::text(
        pct_x,
        status_text_y,
        &pct_str,
        theme.status_text,
        STATUS_FONT,
        0,
        SCREEN_W,
    ));

    // BT / HP indicators
    let bt_x = pct_x - 2 - STATUS_SLOT_W;
    let hp_x = bt_x - STATUS_SLOT_W;
    let bt_color = if state.bluetooth {
        theme.status_text
    } else {
        theme.status_off
    };
    let hp_color = if state.headphones {
        theme.status_text
    } else {
        theme.status_off
    };
    cmds.push(DrawCmd::text(
        bt_x,
        status_text_y,
        "BT",
        bt_color,
        STATUS_FONT,
        0,
        SCREEN_W,
    ));
    cmds.push(DrawCmd::text(
        hp_x,
        status_text_y,
        "HP",
        hp_color,
        STATUS_FONT,
        0,
        SCREEN_W,
    ));

    // === 2. Header ===
    let header_y = STATUS_H;
    cmds.push(DrawCmd::GradientRect {
        x: 0,
        y: header_y,
        w: SCREEN_W as u32,
        h: HEADER_H as u32,
        color_start: theme.header_top,
        color_end: theme.header_bot,
    });
    cmds.push(DrawCmd::rect(
        0,
        header_y,
        SCREEN_W as u32,
        1,
        theme.header_hilite,
    ));
    cmds.push(DrawCmd::rect(
        0,
        header_y + HEADER_H - 1,
        SCREEN_W as u32,
        1,
        theme.header_shadow,
    ));

    let header_text_y = header_y + (HEADER_H - HEADER_FONT as i32) / 2;
    let (hdr_scroll_x, hdr_text_w) = scroll_offset(
        state.title,
        HEADER_FONT,
        HEADER_MAX_WIDTH,
        HEADER_SCROLL_GAP,
        state.header_scroll_elapsed,
        state.is_reversing,
        measure,
    );
    let hdr_draw_x = HEADER_PAD_X - hdr_scroll_x as i32;
    let hdr_clip_left = HEADER_PAD_X;
    let hdr_clip_right = HEADER_PAD_X + HEADER_MAX_WIDTH as i32;
    cmds.push(DrawCmd::text(
        hdr_draw_x,
        header_text_y,
        state.title,
        theme.header_text,
        HEADER_FONT,
        hdr_clip_left,
        hdr_clip_right,
    ));
    if hdr_scroll_x > 0.0 && !state.is_reversing {
        let wrap_x = hdr_draw_x + hdr_text_w as i32 + HEADER_SCROLL_GAP as i32;
        cmds.push(DrawCmd::text(
            wrap_x,
            header_text_y,
            state.title,
            theme.header_text,
            HEADER_FONT,
            hdr_clip_left,
            hdr_clip_right,
        ));
    }

    // === 3. Album-art panel (hidden on settings) ===
    if !state.is_settings {
        let art_x = SCREEN_W - ART_PANEL_W + ART_PAD;
        let art_y = content_top + (content_h - ART_SIZE) / 2;

        cmds.push(DrawCmd::rect(
            art_x - 2,
            art_y - 2,
            (ART_SIZE + 4) as u32,
            (ART_SIZE + 4) as u32,
            theme.art_frame,
        ));

        if state.has_pane_art_prev {
            cmds.push(DrawCmd::AlbumArtPrev {
                x: art_x,
                y: art_y,
                w: ART_SIZE as u32,
                h: ART_SIZE as u32,
                alpha: 255,
            });
        }
        if state.has_pane_art {
            cmds.push(DrawCmd::AlbumArt {
                x: art_x,
                y: art_y,
                w: ART_SIZE as u32,
                h: ART_SIZE as u32,
                progress: state.pane_art_progress, //1.0
            });
        } else if !state.has_pane_art_prev {
            // Placeholder colored square based on selection index.
            let colors = theme.placeholder_colors;
            let color = colors[selected % colors.len()];
            cmds.push(DrawCmd::rect(
                art_x,
                art_y,
                ART_SIZE as u32,
                ART_SIZE as u32,
                color,
            ));
        }
    }

    // === 4. List ===
    let n = state.items.len() as i32;
    let first = state.scroll_offset as i32 + 1;
    let last = (first + VISIBLE_ROWS).min(n);

    let list_x = 0;
    let list_w = if state.is_settings { SCREEN_W } else { LIST_W };
    let list_item_max = (list_w - 2 * LIST_PAD_X) as f32;

    for i in first..=last {
        let idx_usize = (i - 1) as usize;
        if idx_usize >= state.items.len() {
            break;
        }
        let item = &state.items[idx_usize];
        let y = content_top + (i - first) * item_h;
        let selected_here = idx_usize == selected;

        if selected_here {
            cmds.push(DrawCmd::rect(
                list_x,
                y,
                list_w as u32,
                item_h as u32,
                theme.selected,
            ));
        }

        let text_y = y + ITEM_VPAD;

        if state.is_settings {
            // label-left, `<value>`-right
            let val = format!(
                "<{}>",
                state
                    .option_values
                    .and_then(|v| v.get(idx_usize))
                    .map(|s| s.as_str())
                    .unwrap_or("")
            );
            let val_w = measure(&val, ITEM_FONT) as i32;
            let val_x = list_x + list_w - LIST_PAD_X - val_w;
            let label_clip_left = list_x + LIST_PAD_X;
            let label_clip_right = val_x - 4;

            cmds.push(DrawCmd::text(
                label_clip_left,
                text_y,
                item,
                theme.item_text,
                ITEM_FONT,
                label_clip_left,
                label_clip_right,
            ));
            cmds.push(DrawCmd::text(
                val_x,
                text_y,
                &val,
                theme.nowplay_dim,
                ITEM_FONT,
                0,
                SCREEN_W,
            ));
        } else {
            let (scroll_x, text_w) = if selected_here {
                scroll_offset(
                    item,
                    ITEM_FONT,
                    list_item_max,
                    LIST_SCROLL_GAP,
                    state.list_scroll_elapsed,
                    state.is_reversing,
                    measure,
                )
            } else {
                (0.0, 0.0)
            };
            let draw_x = list_x + LIST_PAD_X - scroll_x as i32;
            let clip_left = list_x + LIST_PAD_X;
            let clip_right = clip_left + list_item_max as i32;

            cmds.push(DrawCmd::text(
                draw_x,
                text_y,
                item,
                theme.item_text,
                ITEM_FONT,
                clip_left,
                clip_right,
            ));
            if scroll_x > 0.0 && !state.is_reversing {
                let wrap_x = draw_x + text_w as i32 + LIST_SCROLL_GAP as i32;
                cmds.push(DrawCmd::text(
                    wrap_x,
                    text_y,
                    item,
                    theme.item_text,
                    ITEM_FONT,
                    clip_left,
                    clip_right,
                ));
            }
        }
    }

    // === 5. Now-playing bar ===
    let np_y = content_bottom;

    // Fade gradient above the bar
    cmds.push(DrawCmd::GradientRect {
        x: 0,
        y: np_y - FADE_HEIGHT,
        w: SCREEN_W as u32,
        h: FADE_HEIGHT as u32,
        color_start: theme.nowplay_bg & 0x00FFFFFF, // transparent
        color_end: theme.nowplay_bg,
    });

    cmds.push(DrawCmd::rect(
        0,
        np_y,
        SCREEN_W as u32,
        NOWPLAY_H as u32,
        theme.nowplay_bg,
    ));

    if let Some(now) = &state.now {
        let th_x = NOWPLAY_PAD;
        let th_y = np_y + (NOWPLAY_H - PROGRESS_H - NOWPLAY_ART) / 2;

        if state.has_now_art {
            cmds.push(DrawCmd::NowArtThumb {
                x: th_x,
                y: th_y,
                w: NOWPLAY_ART as u32,
                h: NOWPLAY_ART as u32,
            });
        } else {
            cmds.push(DrawCmd::rect(
                th_x,
                th_y,
                NOWPLAY_ART as u32,
                NOWPLAY_ART as u32,
                theme.art_frame,
            ));
        }

        let text_x = th_x + NOWPLAY_ART + NOWPLAY_PAD;
        let title_y = np_y + NOWPLAY_PAD;

        // Title (possibly scrolling)
        let (now_scroll_x, now_text_w) = scroll_offset(
            now.title,
            NOWPLAY_TITLE_FONT,
            NOWPLAY_MAX_WIDTH,
            NOWPLAY_SCROLL_GAP,
            state.now_scroll_elapsed,
            state.is_reversing,
            measure,
        );
        let draw_title_x = text_x - now_scroll_x as i32;
        let title_clip_left = text_x;
        let title_clip_right = text_x + NOWPLAY_MAX_WIDTH as i32;
        cmds.push(DrawCmd::text(
            draw_title_x,
            title_y,
            now.title,
            theme.nowplay_text,
            NOWPLAY_TITLE_FONT,
            title_clip_left,
            title_clip_right,
        ));
        if now_scroll_x > 0.0 && !state.is_reversing {
            let wrap_x = draw_title_x + now_text_w as i32 + NOWPLAY_SCROLL_GAP as i32;
            cmds.push(DrawCmd::text(
                wrap_x,
                title_y,
                now.title,
                theme.nowplay_text,
                NOWPLAY_TITLE_FONT,
                title_clip_left,
                title_clip_right,
            ));
        }

        // Artist
        let artist_y = title_y + NOWPLAY_TITLE_FONT as i32 + 3;
        let (artist_scroll_x, artist_text_w) = scroll_offset(
            now.artist,
            NOWPLAY_ARTIST_FONT,
            NOWPLAY_ARTIST_MAX_WIDTH,
            ARTIST_SCROLL_GAP,
            state.artist_scroll_elapsed,
            state.is_reversing,
            measure,
        );
        let draw_artist_x = text_x - artist_scroll_x as i32;
        let artist_clip_left = text_x;
        let artist_clip_right = text_x + NOWPLAY_ARTIST_MAX_WIDTH as i32;
        cmds.push(DrawCmd::text(
            draw_artist_x,
            artist_y,
            now.artist,
            theme.nowplay_dim,
            NOWPLAY_ARTIST_FONT,
            artist_clip_left,
            artist_clip_right,
        ));
        if artist_scroll_x > 0.0 && !state.is_reversing {
            let wrap_x = draw_artist_x + artist_text_w as i32 + ARTIST_SCROLL_GAP as i32;
            cmds.push(DrawCmd::text(
                wrap_x,
                artist_y,
                now.artist,
                theme.nowplay_dim,
                NOWPLAY_ARTIST_FONT,
                artist_clip_left,
                artist_clip_right,
            ));
        }

        // Time display (right-aligned)
        let time_str = format!("{} / {}", fmt_time(now.elapsed), fmt_time(now.duration));
        let time_w = measure(&time_str, NOWPLAY_ARTIST_FONT) as i32;
        let timer_x = SCREEN_W - NOWPLAY_PAD - time_w;
        let icon_right = timer_x + time_w;

        // Play / pause glyph
        let stripe_w: i32 = 2;
        let stripe_h: i32 = 8;
        let stripe_gap: i32 = 3;
        let tri_w: i32 = 8;
        let tri_h: i32 = 8;

        if state.is_paused {
            cmds.push(DrawCmd::Triangle {
                x: icon_right - tri_w,
                y: title_y,
                w: tri_w as u32,
                h: tri_h as u32,
                radius: 1,
                color: theme.nowplay_dim,
            });
        } else {
            let total_w = stripe_w * 2 + stripe_gap;
            let stripe_x = icon_right - total_w;
            cmds.push(DrawCmd::rect_rounded(
                stripe_x,
                title_y,
                stripe_w as u32,
                stripe_h as u32,
                theme.nowplay_dim,
                1,
            ));
            cmds.push(DrawCmd::rect_rounded(
                stripe_x + stripe_w + stripe_gap,
                title_y,
                stripe_w as u32,
                stripe_h as u32,
                theme.nowplay_dim,
                1,
            ));
        }

        cmds.push(DrawCmd::text(
            timer_x,
            artist_y,
            &time_str,
            theme.nowplay_dim,
            NOWPLAY_ARTIST_FONT,
            0,
            SCREEN_W,
        ));

        // Progress bar
        let bar_y = np_y + NOWPLAY_H - PROGRESS_H;
        cmds.push(DrawCmd::rect(
            0,
            bar_y,
            SCREEN_W as u32,
            PROGRESS_H as u32,
            theme.progress_bg,
        ));
        let ratio = if now.duration > 0.0 {
            (now.elapsed / now.duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let bar_w = (SCREEN_W as f32 * ratio) as i32;
        if bar_w > 0 {
            cmds.push(DrawCmd::rect(
                0,
                bar_y,
                bar_w as u32,
                PROGRESS_H as u32,
                theme.progress_fg,
            ));
        }
    }

    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    /// Stub measure: one character ≈ size/2 px wide.
    fn mock_measure(text: &str, size: u16) -> f32 {
        text.chars().count() as f32 * (size as f32 * 0.5)
    }

    fn empty_frame() -> UiFrame<'static> {
        UiFrame {
            items: &[],
            selected_index: 0,
            scroll_offset: 0,
            title: "MyMuse",
            clock: "12:34",
            battery: 0.8,
            bluetooth: true,
            headphones: false,
            now: None,
            has_now_art: false,
            has_pane_art: false,
            has_pane_art_prev: false,
            list_scroll_elapsed: 0.0,
            now_scroll_elapsed: 0.0,
            header_scroll_elapsed: 0.0,
            artist_scroll_elapsed: 0.0,
            is_reversing: true,
            is_paused: true,
            is_settings: false,
            option_values: None,
            line_height_ratio: 1.16,
            theme: theme::colors(crate::settings::Theme::GrayDark),
            pane_art_progress: 1.0,
        }
    }

    #[test]
    fn empty_frame_produces_background_and_chrome() {
        let cmds = draw_screen(&empty_frame(), &mock_measure);
        assert!(!cmds.is_empty());
        // First command is full-screen background.
        assert!(matches!(
            cmds[0],
            DrawCmd::Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 240,
                ..
            }
        ));
    }

    #[test]
    fn now_playing_produces_progress_and_time_text() {
        let items = alloc::vec![];
        let mut frame = empty_frame();
        frame.items = &items;
        frame.now = Some(NowPlayingView {
            title: "Song",
            artist: "Artist",
            elapsed: 30.0,
            duration: 100.0,
        });
        frame.has_now_art = false;
        let cmds = draw_screen(&frame, &mock_measure);
        // Find a text command containing "/"
        let has_time = cmds.iter().any(|c| {
            matches!(c,
                DrawCmd::Text { text, .. } if text.contains('/')
            )
        });
        assert!(has_time, "expected time '0:30 / 1:40' in output");
    }
}
