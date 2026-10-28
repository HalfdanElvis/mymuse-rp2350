//! Application state machine.
//!
//! Direct port of the original `src/app/*.rs` from the desktop version.
//! Everything `std`-dependent (audio device, window, real clock) is
//! delegated: `tick()` and `handle_input()` take `now_ms` and `AudioState`
//! from firmware; they return `PlaybackIntent`s and `DrawCmd`s.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::draw::DrawCmd;
use crate::library::{AlbumArt, Track};
use crate::nav::{Level, NavState};
use crate::system::SystemInfo;
use crate::theme;
use crate::ui::{NowPlayingView, UiFrame, draw_screen};
use crate::util::{level_title, pseudo_random_index};

// ============================================================================
// Constants (ported 1:1 from src/app/seek.rs and slideshow.rs)
// ============================================================================

const SEEK_STEP_SECS: f32 = 5.0;
const SEEK_HOLD_THRESHOLD_MS: u64 = 500;
const SEEK_HOLD_REPEAT_MS: u64 = 500;
const PREV_TRACK_THRESHOLD_SECS: f32 = 3.0;
const SLIDESHOW_INTERVAL_MS: u64 = 4000;
const TRANSITION_DURATION_MS: u64 = 500;

// ============================================================================
// External interfaces
// ============================================================================

/// What firmware hands to core when the user does something.
#[derive(Debug, Clone, Copy)]
pub enum InputAction {
    Back,
    Up,
    Down,
    Enter,
    TogglePause,
    /// Direction: -1 = left, +1 = right. Pressed.
    SeekStart(i8),
    /// Same direction, released.
    SeekEnd(i8),
}

/// What core asks firmware to do with the audio device.
#[derive(Debug, Clone)]
pub enum PlaybackIntent {
    Play { path: String, from_secs: f32 },
    Pause,
    Resume,
    SeekTo(f32),
}

/// What firmware reports about the audio device each tick.
#[derive(Debug, Clone, Copy)]
pub struct AudioState {
    pub position_secs: f32,
    pub is_paused: bool,
    /// True when the current track has ended (or no track is loaded).
    pub is_empty: bool,
    /// Duration of the current track, if known.
    pub duration_secs: Option<f32>,
}

impl AudioState {
    pub fn idle() -> Self {
        Self {
            position_secs: 0.0,
            is_paused: true,
            is_empty: true,
            duration_secs: None,
        }
    }
}

// ============================================================================
// Internal state
// ============================================================================

pub struct NowPlayingState {
    pub track: Track,
    pub duration: f32,
    pub queue: Vec<Track>,
    pub queue_index: usize,
    pub has_ended: bool,
}

/// Decoded album art keyed by (artist, album).
pub struct ArtCache {
    entries: BTreeMap<(String, String), AlbumArt>,
}

impl ArtCache {
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn get(&self, key: &(String, String)) -> Option<&AlbumArt> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: (String, String), art: AlbumArt) {
        self.entries.insert(key, art);
    }

    pub fn contains(&self, key: &(String, String)) -> bool {
        self.entries.contains_key(key)
    }
}

pub struct PaneArtState {
    pub current: Option<(String, String)>,
    pub previous: Option<(String, String)>,
    pub context_artist: Option<String>,
    pub last_swap_ms: u64,
    pub transition_start_ms: u64,
}

impl PaneArtState {
    fn new(now_ms: u64) -> Self {
        Self {
            current: None,
            previous: None,
            context_artist: None,
            last_swap_ms: now_ms,
            transition_start_ms: now_ms,
        }
    }

    fn progress(&self, now_ms: u64) -> f32 {
        let elapsed = now_ms.saturating_sub(self.transition_start_ms);
        (elapsed as f32 / TRANSITION_DURATION_MS as f32).clamp(0.0, 1.0)
    }
}

struct SeekHold {
    direction: i8,
    started_ms: u64,
    last_action_ms: Option<u64>,
    hold_triggered: bool,
}

// ============================================================================
// App
// ============================================================================

pub struct App {
    pub nav: NavState,
    pub art_cache: ArtCache,
    pub now_playing: Option<NowPlayingState>,
    pub system_info: Box<dyn SystemInfo>,
    pub pane_art: PaneArtState,

    line_height_ratio: f32,

    hover_start_ms: u64,
    now_playing_start_ms: u64,
    header_scroll_start_ms: u64,
    last_header_title: Option<String>,

    seek_hold: Option<SeekHold>,
}

impl App {
    pub fn new(
        nav: NavState,
        system_info: Box<dyn SystemInfo>,
        line_height_ratio: f32,
        now_ms: u64,
    ) -> Self {
        Self {
            nav,
            art_cache: ArtCache::new(),
            now_playing: None,
            system_info,
            pane_art: PaneArtState::new(now_ms),
            line_height_ratio,
            hover_start_ms: now_ms,
            now_playing_start_ms: now_ms,
            header_scroll_start_ms: now_ms,
            last_header_title: None,
            seek_hold: None,
        }
    }

    // --- Public API ------------------------------------------------------

    /// Called every tick. Handles hold-repeat seeking, track advancement,
    /// and pane-art slideshow. Returns at most one audio intent.
    pub fn tick(&mut self, now_ms: u64, audio: AudioState) -> Option<PlaybackIntent> {
        // Fill in duration once firmware knows it.
        if let (Some(np), Some(d)) = (&mut self.now_playing, audio.duration_secs) {
            if np.duration == 0.0 && d > 0.0 {
                np.duration = d;
            }
        }

        // Hold-to-seek repeat fires here.
        if let Some(intent) = self.process_seek_hold(now_ms, &audio) {
            return Some(intent);
        }

        // Track ended? Advance.
        let mut next = None;
        if audio.is_empty {
            if let Some(np) = &mut self.now_playing {
                if !np.has_ended {
                    let next_idx = np.queue_index + 1;
                    if next_idx < np.queue.len() {
                        next = Some((np.queue[next_idx].clone(), np.queue.clone(), next_idx));
                    }
                    np.has_ended = true;
                }
            }
        }
        if let Some((track, queue, idx)) = next {
            return Some(self.play_track_from_queue(track, queue, idx, now_ms));
        }

        self.update_pane_art(now_ms);
        None
    }

    /// Handle a user input event. Returns at most one audio intent.
    pub fn handle_input(
        &mut self,
        action: InputAction,
        now_ms: u64,
        audio: AudioState,
    ) -> Option<PlaybackIntent> {
        match action {
            InputAction::Back => {
                self.nav_back(now_ms);
                None
            }
            InputAction::Up => {
                self.nav_up(now_ms);
                None
            }
            InputAction::Down => {
                self.nav_down(now_ms);
                None
            }
            InputAction::Enter => self.nav_enter(now_ms),
            InputAction::TogglePause => {
                if audio.is_paused {
                    Some(PlaybackIntent::Resume)
                } else {
                    Some(PlaybackIntent::Pause)
                }
            }
            InputAction::SeekStart(dir) => {
                self.start_seek_hold(dir, now_ms);
                None
            }
            InputAction::SeekEnd(dir) => self.handle_seek_release(dir, now_ms, &audio),
        }
    }

    /// Render the current state to draw commands. Firmware executes them.
    pub fn render<M: Fn(&str, u16) -> f32>(
        &mut self,
        now_ms: u64,
        audio: &AudioState,
        clock_str: &str,
        measure: &M,
    ) -> Frame<'_> {
        let (level, selected_index, scroll_offset) = self.nav.current().clone();
        let title = level_title(&level);

        // Reset header scroll when the title changes.
        if self.last_header_title.as_deref() != Some(title.as_str()) {
            self.last_header_title = Some(title.clone());
            self.header_scroll_start_ms = now_ms;
        }

        let is_settings = matches!(level, Level::Settings);
        let option_values = if is_settings {
            Some(self.nav.settings.values())
        } else {
            None
        };

        let now_view = self.now_playing.as_ref().map(|np| NowPlayingView {
            title: &np.track.title,
            artist: &np.track.artist,
            elapsed: audio.position_secs,
            duration: if np.duration > 0.0 {
                np.duration
            } else {
                audio.duration_secs.unwrap_or(0.0)
            },
        });

        let now_art_present = self.now_art().is_some();
        let pane_progress = self.pane_art.progress(now_ms);
        let pane_prev_present = pane_progress < 1.0
            && self
                .pane_art
                .previous
                .as_ref()
                .and_then(|k| self.art_cache.get(k))
                .is_some();
        let pane_curr_present = self
            .pane_art
            .current
            .as_ref()
            .and_then(|k| self.art_cache.get(k))
            .is_some();

        let list_elapsed = now_ms.saturating_sub(self.hover_start_ms) as f32 / 1000.0;
        let now_elapsed = now_ms.saturating_sub(self.now_playing_start_ms) as f32 / 1000.0;
        let header_elapsed = now_ms.saturating_sub(self.header_scroll_start_ms) as f32 / 1000.0;

        let frame = UiFrame {
            items: self.nav.get_items(),
            selected_index,
            scroll_offset,
            title: &title,
            clock: clock_str,
            battery: self.system_info.battery(),
            bluetooth: self.system_info.bluetooth(),
            headphones: self.system_info.headphones(),
            now: now_view,
            has_now_art: now_art_present,
            has_pane_art: pane_curr_present,
            has_pane_art_prev: pane_prev_present,
            list_scroll_elapsed: list_elapsed,
            now_scroll_elapsed: now_elapsed,
            header_scroll_elapsed: header_elapsed,
            artist_scroll_elapsed: now_elapsed,
            is_reversing: self.nav.settings.pingpong_scroll,
            is_paused: audio.is_paused || audio.is_empty,
            is_settings,
            option_values: option_values.as_deref(),
            line_height_ratio: self.line_height_ratio,
            pane_art_progress: pane_progress,
            theme: theme::colors(self.nav.settings.theme),
        };

        let cmds = draw_screen(&frame, measure);
        drop(frame);

        let art = ArtContext {
            pane_current: self
                .pane_art
                .current
                .as_ref()
                .and_then(|k| self.art_cache.get(k)),
            pane_previous: if pane_progress < 1.0 {
                self.pane_art
                    .previous
                    .as_ref()
                    .and_then(|k| self.art_cache.get(k))
            } else {
                None
            },
            now: self.now_art(),
            pane_progress,
        };

        Frame { cmds, art }
    }

    /// Firmware calls this after decoding art for `key`.
    pub fn cache_art(&mut self, key: (String, String), art: AlbumArt) {
        self.art_cache.insert(key, art);
    }

    /// Firmware polls this to know which album's art to decode next.
    pub fn current_art_key(&self) -> Option<&(String, String)> {
        self.pane_art.current.as_ref()
    }

    /// Firmware polls this to know which art is needed for the now-playing bar.
    pub fn now_art_key(&self) -> Option<(String, String)> {
        self.now_playing
            .as_ref()
            .map(|np| (np.track.artist.clone(), np.track.album.clone()))
    }

    pub fn now_art(&self) -> Option<&AlbumArt> {
        let np = self.now_playing.as_ref()?;
        self.art_cache
            .get(&(np.track.artist.clone(), np.track.album.clone()))
    }

    // --- Nav events ------------------------------------------------------

    fn nav_down(&mut self, now_ms: u64) {
        let (_, idx, _) = self.nav.current();
        let items_len = self.nav.get_items().len();
        if *idx >= items_len.saturating_sub(1) {
            return;
        }
        if let Some((_, idx_mut, scroll_mut)) = self.nav.level_stack.last_mut() {
            *idx_mut += 1;
            if *idx_mut >= *scroll_mut + 7 {
                *scroll_mut += 1;
            }
        }
        self.hover_start_ms = now_ms;
    }

    fn nav_up(&mut self, now_ms: u64) {
        let (_, idx, _) = self.nav.current();
        if *idx == 0 {
            return;
        }
        if let Some((_, idx_mut, scroll_mut)) = self.nav.level_stack.last_mut() {
            *idx_mut -= 1;
            if *idx_mut < *scroll_mut {
                *scroll_mut -= 1;
            }
        }
        self.hover_start_ms = now_ms;
    }

    fn nav_back(&mut self, now_ms: u64) {
        if self.nav.back() {
            self.hover_start_ms = now_ms;
        }
    }

    fn nav_enter(&mut self, now_ms: u64) -> Option<PlaybackIntent> {
        self.hover_start_ms = now_ms;
        if let Some(idx) = self.nav.enter() {
            let queue = self.nav.get_current_playlist();
            if let Some(track) = queue.get(idx).cloned() {
                return Some(self.play_track_from_queue(track, queue, idx, now_ms));
            }
        }
        None
    }

    // --- Playback --------------------------------------------------------

    fn play_track_from_queue(
        &mut self,
        track: Track,
        queue: Vec<Track>,
        index: usize,
        now_ms: u64,
    ) -> PlaybackIntent {
        self.now_playing_start_ms = now_ms;
        let path = track.path.clone();
        self.now_playing = Some(NowPlayingState {
            track,
            duration: 0.0,
            queue,
            queue_index: index,
            has_ended: false,
        });
        PlaybackIntent::Play {
            path,
            from_secs: 0.0,
        }
    }

    fn tap_right(&mut self, now_ms: u64) -> Option<PlaybackIntent> {
        let (track, queue, next_idx) = {
            let np = self.now_playing.as_ref()?;
            let next_idx = np.queue_index + 1;
            if next_idx >= np.queue.len() {
                return None;
            }
            (np.queue[next_idx].clone(), np.queue.clone(), next_idx)
        };
        Some(self.play_track_from_queue(track, queue, next_idx, now_ms))
    }

    fn tap_left(&mut self, audio: &AudioState, now_ms: u64) -> Option<PlaybackIntent> {
        if audio.position_secs < PREV_TRACK_THRESHOLD_SECS {
            let prev = {
                let np = self.now_playing.as_ref()?;
                if np.queue_index == 0 {
                    None
                } else {
                    let prev_idx = np.queue_index - 1;
                    Some((np.queue[prev_idx].clone(), np.queue.clone(), prev_idx))
                }
            };
            if let Some((track, queue, prev_idx)) = prev {
                return Some(self.play_track_from_queue(track, queue, prev_idx, now_ms));
            }
        }
        Some(PlaybackIntent::SeekTo(0.0))
    }

    fn seek_relative(&self, delta: f32, audio: &AudioState) -> Option<PlaybackIntent> {
        let np = self.now_playing.as_ref()?;
        let duration = audio.duration_secs.unwrap_or(np.duration);
        if duration <= 0.0 {
            return None;
        }
        let target = (audio.position_secs + delta).clamp(0.0, duration);
        Some(PlaybackIntent::SeekTo(target))
    }

    // --- Seek hold -------------------------------------------------------

    fn start_seek_hold(&mut self, direction: i8, now_ms: u64) {
        self.seek_hold = Some(SeekHold {
            direction,
            started_ms: now_ms,
            last_action_ms: None,
            hold_triggered: false,
        });
    }

    fn process_seek_hold(&mut self, now_ms: u64, audio: &AudioState) -> Option<PlaybackIntent> {
        let dir = {
            let hold = self.seek_hold.as_mut()?;
            if now_ms.saturating_sub(hold.started_ms) < SEEK_HOLD_THRESHOLD_MS {
                return None;
            }
            let due = match hold.last_action_ms {
                None => true,
                Some(t) => now_ms.saturating_sub(t) >= SEEK_HOLD_REPEAT_MS,
            };
            if !due {
                return None;
            }
            hold.hold_triggered = true;
            hold.last_action_ms = Some(now_ms);
            hold.direction as f32
        };
        self.seek_relative(dir * SEEK_STEP_SECS, audio)
    }

    fn handle_seek_release(
        &mut self,
        direction: i8,
        now_ms: u64,
        audio: &AudioState,
    ) -> Option<PlaybackIntent> {
        let hold = self.seek_hold.take()?;
        if hold.direction != direction {
            return None;
        }
        let held_for = now_ms.saturating_sub(hold.started_ms);
        if held_for < SEEK_HOLD_THRESHOLD_MS {
            if direction > 0 {
                self.tap_right(now_ms)
            } else {
                self.tap_left(audio, now_ms)
            }
        } else if !hold.hold_triggered {
            self.seek_relative(direction as f32 * SEEK_STEP_SECS, audio)
        } else {
            None
        }
    }

    // --- Pane art / slideshow --------------------------------------------

    fn update_pane_art(&mut self, now_ms: u64) {
        let level = self.nav.current().0.clone();

        if matches!(level, Level::Settings) {
            return;
        }

        let desired = if matches!(level, Level::Root | Level::Artists) {
            self.slideshow_pick(&level, now_ms)
        } else {
            self.nav.current_album_context()
        };

        if desired != self.pane_art.current {
            self.pane_art.previous = self.pane_art.current.take();
            self.pane_art.current = desired;
            self.pane_art.transition_start_ms = now_ms;
        }
    }

    fn slideshow_pick(&mut self, level: &Level, now_ms: u64) -> Option<(String, String)> {
        let context_artist = match level {
            Level::Artists => self.nav.hovered_artist(),
            _ => None,
        };

        let context_changed = self.pane_art.context_artist != context_artist;
        let fresh_enough = !context_changed
            && self.pane_art.current.is_some()
            && now_ms.saturating_sub(self.pane_art.last_swap_ms) < SLIDESHOW_INTERVAL_MS;
        if fresh_enough {
            return self.pane_art.current.clone();
        }

        let pool: Vec<(String, String)> = match &context_artist {
            Some(artist) => self
                .nav
                .all_albums()
                .iter()
                .filter(|(a, _)| a == artist)
                .cloned()
                .collect(),
            None => self.nav.all_albums().to_vec(),
        };

        self.pane_art.context_artist = context_artist;
        self.pane_art.last_swap_ms = now_ms;

        if pool.is_empty() {
            return None;
        }
        if pool.len() == 1 {
            return Some(pool[0].clone());
        }

        let mut idx = pseudo_random_index(now_ms, pool.len());
        if let Some(cur) = &self.pane_art.current {
            if &pool[idx] == cur {
                idx = (idx + 1) % pool.len();
            }
        }
        Some(pool[idx].clone())
    }
}

// ============================================================================
// Render output
// ============================================================================

pub struct ArtContext<'a> {
    pub pane_current: Option<&'a AlbumArt>,
    pub pane_previous: Option<&'a AlbumArt>,
    pub now: Option<&'a AlbumArt>,
    pub pane_progress: f32,
}

pub struct Frame<'a> {
    pub cmds: Vec<DrawCmd>,
    pub art: ArtContext<'a>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Album, Library};
    use crate::system::MockSystemInfo;

    fn test_nav() -> NavState {
        let mut lib = Library::new();
        let mut album = Album {
            tracks: Vec::new(),
            art_bytes: None,
        };
        album.tracks.push(Track {
            path: "Artist/Album/01.mp3".into(),
            title: "Track 1".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            track_num: 1,
        });
        lib.entry("Artist".into())
            .or_insert_with(alloc::collections::BTreeMap::new)
            .insert("Album".into(), album);
        NavState::new(lib)
    }

    fn test_app(now_ms: u64) -> App {
        App::new(test_nav(), Box::new(MockSystemInfo::new()), 1.16, now_ms)
    }

    #[test]
    fn new_app_starts_at_root() {
        let app = test_app(0);
        assert_eq!(app.nav.current().0, Level::Root);
        assert!(app.now_playing.is_none());
    }

    #[test]
    fn enter_from_root_drills_to_artists() {
        let mut app = test_app(0);
        // Root items: [Artists, Albums, Songs, Settings], selected=0
        let intent = app.handle_input(InputAction::Enter, 0, AudioState::idle());
        assert!(intent.is_none());
        assert!(matches!(app.nav.current().0, Level::Artists));
    }

    #[test]
    fn back_from_artists_returns_to_root() {
        let mut app = test_app(0);
        app.handle_input(InputAction::Enter, 0, AudioState::idle());
        app.handle_input(InputAction::Back, 0, AudioState::idle());
        assert_eq!(app.nav.current().0, Level::Root);
    }

    #[test]
    fn down_moves_selection() {
        let mut app = test_app(0);
        assert_eq!(app.nav.current().1, 0);
        app.handle_input(InputAction::Down, 0, AudioState::idle());
        assert_eq!(app.nav.current().1, 1);
    }

    #[test]
    fn up_at_top_is_noop() {
        let mut app = test_app(0);
        app.handle_input(InputAction::Up, 0, AudioState::idle());
        assert_eq!(app.nav.current().1, 0);
    }

    #[test]
    fn toggle_pause_when_idle_returns_resume() {
        let mut app = test_app(0);
        let audio = AudioState::idle();
        let intent = app.handle_input(InputAction::TogglePause, 0, audio);
        assert!(matches!(intent, Some(PlaybackIntent::Resume)));
    }

    #[test]
    fn toggle_pause_when_playing_returns_pause() {
        let mut app = test_app(0);
        let audio = AudioState {
            is_paused: false,
            ..AudioState::idle()
        };
        let intent = app.handle_input(InputAction::TogglePause, 0, audio);
        assert!(matches!(intent, Some(PlaybackIntent::Pause)));
    }

    #[test]
    fn tick_advances_after_track_ends() {
        let mut app = test_app(0);
        // Manually install a two-track queue with the first playing.
        app.now_playing = Some(NowPlayingState {
            track: Track {
                path: "a/1.mp3".into(),
                title: "One".into(),
                artist: "A".into(),
                album: "B".into(),
                track_num: 1,
            },
            duration: 100.0,
            queue: alloc::vec![
                Track {
                    path: "a/1.mp3".into(),
                    title: "One".into(),
                    artist: "A".into(),
                    album: "B".into(),
                    track_num: 1,
                },
                Track {
                    path: "a/2.mp3".into(),
                    title: "Two".into(),
                    artist: "A".into(),
                    album: "B".into(),
                    track_num: 2,
                },
            ],
            queue_index: 0,
            has_ended: false,
        });

        let audio = AudioState {
            is_empty: true,
            is_paused: false,
            ..AudioState::idle()
        };
        let intent = app.tick(1000, audio);
        assert!(matches!(
            intent,
            Some(PlaybackIntent::Play { from_secs: 0.0, .. })
        ));
        assert_eq!(app.now_playing.as_ref().unwrap().queue_index, 1);
    }
}
