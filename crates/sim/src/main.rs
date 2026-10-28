//! Host simulator: runs the full MyMuse pipeline against real files
//! on your desktop's filesystem, renders each frame to `output/*.png`,
//! and accepts keyboard input from stdin.
//!
//! Run:  cargo run --target x86_64-unknown-linux-gnu -p sim --release -- ../mymuse/test_music
//! Keys: d=down u=up e=enter b=back p=play/pause l=left r=right q=quit
//! PNGs: output/frame_NNNN.png

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use fontdue::Font;
use image::{Rgba, RgbaImage};

use mymuse_core::app::{App, ArtContext, AudioState, InputAction, PlaybackIntent};
use mymuse_core::draw::DrawCmd;
use mymuse_core::library::{AlbumArt, HANDLED_EXTENSIONS, parse_id3_size, parse_id3_tag};
use mymuse_core::nav::NavState;
use mymuse_core::system::MockSystemInfo;

use mymuse_hal::{ArtDecoder, AudioOutput, Display, InputSource, Storage, TimeSource};

const SCREEN_W: u32 = 320;
const SCREEN_H: u32 = 240;

// ============================================================================
// Sim Time
// ============================================================================

struct SimTime {
    start: Instant,
}

impl SimTime {
    fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl TimeSource for SimTime {
    fn now_ms(&mut self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

// ============================================================================
// Sim Input (stdin, line-based)
// ============================================================================

struct SimInput {
    last_action: Option<InputAction>,
}

impl SimInput {
    fn new() -> Self {
        Self { last_action: None }
    }

    fn prompt(&mut self) -> Option<InputAction> {
        print!("[d/u/e/b/p/l/r/q] > ");
        io::stdout().flush().ok();
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).ok()? == 0 {
            return Some(InputAction::Back); // EOF
        }
        let action = match line.trim().chars().next()? {
            'd' => InputAction::Down,
            'u' => InputAction::Up,
            'e' => InputAction::Enter,
            'b' => InputAction::Back,
            'p' => InputAction::TogglePause,
            'l' => InputAction::SeekEnd(-1),
            'r' => InputAction::SeekEnd(1),
            'q' => return None,
            _ => return Some(InputAction::Down), // retry-ish
        };
        self.last_action = Some(action);
        Some(action)
    }
}

impl InputSource for SimInput {
    fn poll(&mut self) -> Option<InputAction> {
        self.last_action.take()
    }
}

// ============================================================================
// Sim Audio (stub; prints intents)
// ============================================================================

// ============================================================================
// Sim Audio (stub; prints intents)
// ============================================================================

struct SimAudio {
    is_paused: bool,
    current_track: Option<String>,
}

impl SimAudio {
    fn new() -> Self {
        Self {
            is_paused: true,
            current_track: None,
        }
    }
}

impl AudioOutput for SimAudio {
    fn execute(&mut self, intent: PlaybackIntent) {
        match intent {
            PlaybackIntent::Play { path, from_secs } => {
                println!("  → play {} @ {}s", path, from_secs);
                self.current_track = Some(path);
                self.is_paused = false;
            }
            PlaybackIntent::Pause => {
                println!("  → pause");
                self.is_paused = true;
            }
            PlaybackIntent::Resume => {
                println!("  → resume");
                self.is_paused = false;
            }
            PlaybackIntent::SeekTo(pos) => {
                println!("  → seek to {}s", pos);
            }
        }
    }

    fn state(&self) -> AudioState {
        AudioState {
            is_paused: self.is_paused,
            // THIS is the fix: it now correctly reports if a track is loaded
            is_empty: self.current_track.is_none(),
            position_secs: 0.0,
            duration_secs: Some(200.0), // Fake a 3:20 duration
        }
    }
}

// ============================================================================
// Sim Storage (std::fs)
// ============================================================================

struct SimStorage {
    root: PathBuf,
}

impl SimStorage {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Storage for SimStorage {
    fn list_files(&mut self, extensions: &[&str]) -> Result<Vec<(String, u32)>, ()> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, extensions, &mut out).map_err(|_| ())?;
        out.sort();
        Ok(out)
    }

    fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>, ()> {
        let full = self.root.join(path);
        let data = fs::read(&full).map_err(|_| ())?;
        Ok(data.into_iter().take(max_bytes).collect())
    }

    fn read_at(&mut self, path: &str, offset: u32, len: usize) -> Result<Vec<u8>, ()> {
        use std::io::{Read, Seek, SeekFrom};
        let full = self.root.join(path);
        let mut f = File::open(&full).map_err(|_| ())?;
        f.seek(SeekFrom::Start(offset as u64)).map_err(|_| ())?;
        let mut buf = vec![0u8; len];
        let n = f.read(&mut buf).map_err(|_| ())?;
        buf.truncate(n);
        Ok(buf)
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), ()> {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(|_| ())?;
        }
        fs::write(&full, data).map_err(|_| ())
    }

    fn exists(&mut self, path: &str) -> bool {
        self.root.join(path).exists()
    }

    fn file_size(&mut self, path: &str) -> Option<u32> {
        fs::metadata(self.root.join(path))
            .ok()
            .map(|m| m.len() as u32)
    }
}

fn walk(root: &Path, dir: &Path, exts: &[&str], out: &mut Vec<(String, u32)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, exts, out)?;
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        if !exts.contains(&ext.as_str()) {
            continue;
        }
        let size = entry.metadata()?.len() as u32;
        let rel = path.strip_prefix(root).unwrap_or(&path);
        out.push((rel.to_string_lossy().into_owned(), size));
    }
    Ok(())
}

// ============================================================================
// Sim ArtDecoder (image crate)
// ============================================================================

struct SimDecoder;

impl ArtDecoder for SimDecoder {
    fn decode(&mut self, bytes: &[u8]) -> Result<AlbumArt, ()> {
        let img = image::load_from_memory(bytes).map_err(|_| ())?;
        let thumb = img.thumbnail(128, 128);
        let rgba = thumb.to_rgba8();
        let width = rgba.width() as usize;
        let height = rgba.height() as usize;
        let mut pixels = Vec::with_capacity(width * height);
        for chunk in rgba.as_raw().chunks_exact(4) {
            pixels.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        Ok(AlbumArt {
            width,
            height,
            rgba: pixels,
        })
    }
}

// ============================================================================
// Sim Display (PNG output)
// ============================================================================

struct SimDisplay {
    font: Font,
    frame_count: u32,
    output_dir: PathBuf,
}

impl SimDisplay {
    fn new(output_dir: PathBuf) -> Self {
        let candidates = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ];
        let (font_data, path) = candidates
            .iter()
            .find_map(|p| fs::read(p).ok().map(|d| (d, *p)))
            .expect("No system font found. Install DejaVuSans or pass a custom path later.");
        println!("Using font: {}", path);
        let font = Font::from_bytes(font_data, fontdue::FontSettings::default())
            .expect("font parse failed");
        fs::create_dir_all(&output_dir).ok();
        Self {
            font,
            frame_count: 0,
            output_dir,
        }
    }
}

impl Display for SimDisplay {
    fn measure_text(&self, text: &str, size: u16) -> f32 {
        let mut w = 0.0f32;
        for c in text.chars() {
            let (m, _) = self.font.rasterize(c, size as f32);
            w += m.advance_width;
        }
        w
    }

    fn present(&mut self, cmds: &[DrawCmd], art: &ArtContext<'_>) {
        let mut img = RgbaImage::new(SCREEN_W, SCREEN_H);

        for cmd in cmds {
            self.exec(&mut img, cmd, art);
        }

        let path = self
            .output_dir
            .join(format!("frame_{:04}.png", self.frame_count));
        img.save(&path).ok();
        println!("  → saved {}", path.display());
        self.frame_count += 1;
    }
}

impl SimDisplay {
    fn exec(&self, img: &mut RgbaImage, cmd: &DrawCmd, art: &ArtContext<'_>) {
        match cmd {
            DrawCmd::Rect {
                x, y, w, h, color, ..
            } => {
                fill_rect(img, *x, *y, *w, *h, *color);
            }
            DrawCmd::GradientRect {
                x,
                y,
                w,
                h,
                color_start,
                color_end,
            } => {
                fill_gradient(img, *x, *y, *w, *h, *color_start, *color_end);
            }
            DrawCmd::Text {
                x,
                y,
                text,
                color,
                size,
                clip_left,
                clip_right,
            } => {
                draw_text(
                    img,
                    &self.font,
                    *x,
                    *y,
                    text,
                    *color,
                    *size as f32,
                    *clip_left,
                    *clip_right,
                );
            }
            DrawCmd::Triangle {
                x, y, w, h, color, ..
            } => {
                fill_triangle(img, *x, *y, *w, *h, *color);
            }
            DrawCmd::AlbumArt {
                x,
                y,
                w,
                h,
                progress,
            } => {
                blit_crossfade(
                    img,
                    art.pane_previous,
                    art.pane_current,
                    *x,
                    *y,
                    *w,
                    *h,
                    *progress,
                );
            }
            DrawCmd::AlbumArtPrev { x, y, w, h, alpha } => {
                if let Some(prev) = art.pane_previous {
                    blit_art(img, prev, *x, *y, *w, *h, *alpha);
                }
            }
            DrawCmd::NowArtThumb { x, y, w, h } => {
                if let Some(now) = art.now {
                    blit_art(img, now, *x, *y, *w, *h, 255);
                }
            }
        }
    }
}

// ============================================================================
// Low-level rendering helpers (NOW WITH ALPHA BLENDING)
// ============================================================================

fn unpack_argb(c: u32) -> [u8; 4] {
    let a = ((c >> 24) & 0xFF) as u8;
    let r = ((c >> 16) & 0xFF) as u8;
    let g = ((c >> 8) & 0xFF) as u8;
    let b = (c & 0xFF) as u8;
    [r, g, b, a]
}

fn blend(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let sa = src[3] as f32 / 255.0;
    if sa >= 1.0 {
        return src;
    } // Fast path for opaque
    if sa <= 0.0 {
        return dst;
    } // Fast path for transparent
    let inv = 1.0 - sa;
    [
        (src[0] as f32 * sa + dst[0] as f32 * inv) as u8,
        (src[1] as f32 * sa + dst[1] as f32 * inv) as u8,
        (src[2] as f32 * sa + dst[2] as f32 * inv) as u8,
        255,
    ]
}

fn put_blended(img: &mut RgbaImage, x: i32, y: i32, src: [u8; 4]) {
    let dst = img.get_pixel(x as u32, y as u32);
    let out = blend([dst[0], dst[1], dst[2], dst[3]], src);
    img.put_pixel(x as u32, y as u32, Rgba(out));
}

fn fill_rect(img: &mut RgbaImage, x: i32, y: i32, w: u32, h: u32, color: u32) {
    let src = unpack_argb(color);
    for dy in 0..h as i32 {
        let py = y + dy;
        if py < 0 || py >= img.height() as i32 {
            continue;
        }
        for dx in 0..w as i32 {
            let pxx = x + dx;
            if pxx < 0 || pxx >= img.width() as i32 {
                continue;
            }
            put_blended(img, pxx, py, src);
        }
    }
}

fn fill_gradient(img: &mut RgbaImage, x: i32, y: i32, w: u32, h: u32, c0: u32, c1: u32) {
    let [r0, g0, b0, a0] = unpack_argb(c0);
    let [r1, g1, b1, a1] = unpack_argb(c1);
    for dy in 0..h as i32 {
        let py = y + dy;
        if py < 0 || py >= img.height() as i32 {
            continue;
        }
        let t = if h > 1 {
            dy as f32 / (h - 1) as f32
        } else {
            0.0
        };
        let src = [
            (r0 as f32 * (1.0 - t) + r1 as f32 * t) as u8,
            (g0 as f32 * (1.0 - t) + g1 as f32 * t) as u8,
            (b0 as f32 * (1.0 - t) + b1 as f32 * t) as u8,
            (a0 as f32 * (1.0 - t) + a1 as f32 * t) as u8,
        ];
        for dx in 0..w as i32 {
            let pxx = x + dx;
            if pxx < 0 || pxx >= img.width() as i32 {
                continue;
            }
            put_blended(img, pxx, py, src);
        }
    }
}

fn fill_triangle(img: &mut RgbaImage, x: i32, y: i32, w: u32, h: u32, color: u32) {
    let src = unpack_argb(color);
    let ax = x as f32;
    let ay = y as f32;
    let bx = x as f32;
    let by = (y + h as i32) as f32;
    let cx = (x + w as i32) as f32;
    let cy = (y + h as i32 / 2) as f32;

    for dy in 0..h as i32 {
        let py = y + dy;
        if py < 0 || py >= img.height() as i32 {
            continue;
        }
        for dx in 0..w as i32 {
            let pxx = x + dx;
            if pxx < 0 || pxx >= img.width() as i32 {
                continue;
            }

            let px = pxx as f32;
            let pyf = py as f32;

            let d1 = (px - bx) * (ay - by) - (ax - bx) * (pyf - by);
            let d2 = (px - cx) * (by - cy) - (bx - cx) * (pyf - cy);
            let d3 = (px - ax) * (cy - ay) - (cx - ax) * (pyf - ay);

            let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;

            if !(has_neg && has_pos) {
                put_blended(img, pxx, py, src);
            }
        }
    }
}

fn draw_text(
    img: &mut RgbaImage,
    font: &Font,
    x: i32,
    y: i32,
    text: &str,
    color: u32,
    size: f32,
    clip_left: i32,
    clip_right: i32,
) {
    let [cr, cg, cb, _] = unpack_argb(color);
    let ascent = font
        .horizontal_line_metrics(size)
        .map(|m| m.ascent)
        .unwrap_or(size);
    let baseline = (y as f32 + ascent).round() as i32;
    let mut pen_x = x as f32;
    for c in text.chars() {
        if pen_x as i32 >= clip_right {
            break;
        }
        let (metrics, bitmap) = font.rasterize(c, size);
        let top = baseline - metrics.ymin - metrics.height as i32;
        let left = pen_x.round() as i32 + metrics.xmin;
        for row in 0..metrics.height {
            let py = top + row as i32;
            if py < 0 || py >= img.height() as i32 {
                continue;
            }
            for col in 0..metrics.width {
                let px = left + col as i32;
                if px < clip_left || px >= clip_right {
                    continue;
                }
                if px < 0 || px >= img.width() as i32 {
                    continue;
                }
                let alpha = bitmap[row * metrics.width + col];
                if alpha == 0 {
                    continue;
                }
                let a = alpha as f32 / 255.0;
                let src = [cr, cg, cb, (a * 255.0) as u8];
                put_blended(img, px, py, src);
            }
        }
        pen_x += metrics.advance_width;
    }
}

fn blit_art(img: &mut RgbaImage, art: &AlbumArt, x: i32, y: i32, w: u32, h: u32, alpha: u8) {
    if art.width == 0 || art.height == 0 || alpha == 0 {
        return;
    }
    let scale = (w as f32 / art.width as f32).max(h as f32 / art.height as f32);
    let ox = (art.width as f32 * scale - w as f32) / 2.0;
    let oy = (art.height as f32 * scale - h as f32) / 2.0;
    let a_mul = alpha as f32 / 255.0;
    for dy in 0..h as i32 {
        let py = y + dy;
        if py < 0 || py >= img.height() as i32 {
            continue;
        }
        for dx in 0..w as i32 {
            let pxx = x + dx;
            if pxx < 0 || pxx >= img.width() as i32 {
                continue;
            }
            let sx = (((dx as f32 + ox) / scale) as usize).min(art.width - 1);
            let sy = (((dy as f32 + oy) / scale) as usize).min(art.height - 1);
            let [r, g, b, a] = art.rgba[sy * art.width + sx];
            if a == 0 {
                continue;
            }
            let src_a = (a as f32 / 255.0) * a_mul;
            if src_a <= 0.0 {
                continue;
            }
            put_blended(img, pxx, py, [r, g, b, (src_a * 255.0) as u8]);
        }
    }
}

fn blit_crossfade(
    img: &mut RgbaImage,
    prev: Option<&AlbumArt>,
    curr: Option<&AlbumArt>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    progress: f32,
) {
    let p = progress.clamp(0.0, 1.0);
    if p >= 0.5 {
        if let Some(c) = curr {
            blit_art(img, c, x, y, w, h, 255);
        }
    } else {
        if let Some(pr) = prev {
            blit_art(img, pr, x, y, w, h, 255);
        }
    }
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../mymuse/test_music"));

    println!("Root: {}", root.display());

    let mut storage = SimStorage::new(root.clone());
    let files = storage.list_files(&HANDLED_EXTENSIONS).expect("walk");
    println!("Found {} audio files", files.len());

    let mut art_cache_pre: HashMap<(String, String), AlbumArt> = HashMap::new();

    use mymuse_core::library::{Library, add_tagged_track, sort_library_tracks};
    let mut library = Library::new();
    for (path, _size) in &files {
        let Ok(bytes) = storage.read_file(path, 10) else {
            continue;
        };
        let Some(tag_size) = parse_id3_size(&bytes) else {
            let fake = mymuse_core::library::Id3Tags {
                title: None,
                artist: None,
                album: None,
                track_num: None,
                art_bytes: None,
            };
            add_tagged_track(&mut library, path.clone(), fake);
            continue;
        };
        let tag_bytes = storage.read_file(path, tag_size).unwrap_or_default();
        match parse_id3_tag(&tag_bytes) {
            Some(tags) => {
                let art_opt = tags.art_bytes.as_ref().and_then(|b| {
                    let mut dec = SimDecoder;
                    dec.decode(b).ok()
                });
                let key_hint = (
                    tags.artist
                        .clone()
                        .unwrap_or_else(|| "Unknown Artist".to_string()),
                    tags.album
                        .clone()
                        .unwrap_or_else(|| "Unknown Album".to_string()),
                );
                add_tagged_track(&mut library, path.clone(), tags);
                if let Some(art) = art_opt {
                    art_cache_pre.insert(key_hint, art);
                }
            }
            None => {
                let fake = mymuse_core::library::Id3Tags {
                    title: None,
                    artist: None,
                    album: None,
                    track_num: None,
                    art_bytes: None,
                };
                add_tagged_track(&mut library, path.clone(), fake);
            }
        }
    }
    sort_library_tracks(&mut library);
    println!("Library: {} artists", library.len());

    let nav = NavState::new(library);
    let mut app = App::new(nav, Box::new(MockSystemInfo::new()), 1.16, 0);

    for (key, art) in art_cache_pre {
        app.cache_art(key, art);
    }

    let mut time = SimTime::new();
    let mut display = SimDisplay::new(PathBuf::from("output"));
    let mut audio = SimAudio::new();
    let mut input = SimInput::new();

    println!("Ready. Each keypress renders one frame.");

    loop {
        let Some(action) = input.prompt() else { break };
        let now = time.now_ms();
        let mut audio_state = audio.state();

        // 1. Handle Input
        if let Some(intent) = app.handle_input(action, now, audio_state) {
            audio.execute(intent);
            // CRITICAL: Update state immediately so tick() sees the new track
            audio_state = audio.state();
        }

        // 2. Tick (only ONE tick!)
        if let Some(intent) = app.tick(now, audio_state) {
            audio.execute(intent);
            audio_state = audio.state();
        }

        // 3. Render
        let frame = {
            let measure = |t: &str, s: u16| display.measure_text(t, s);
            app.render(now, &audio_state, "12:00", &measure)
        };
        display.present(&frame.cmds, &frame.art);
    }
}
