use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use id3_core::{AttachedPicture, Tag, TextField};

const UNKNOWN_ARTIST: &str = "Unknown Artist";
const UNKNOWN_ALBUM: &str = "Unknown Album";

pub const HANDLED_EXTENSIONS: [&str; 6] = ["mp3", "flac", "m4a", "ogg", "wav", "opus"];

#[derive(Debug, Clone)]
pub struct Track {
    /// Path relative to the music root, e.g. "Artist/Album/01 - Song.mp3".
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_num: u32,
}

#[derive(Debug, Clone)]
pub struct AlbumArt {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<[u8; 4]>,
}

pub struct Album {
    pub tracks: Vec<Track>,
    /// Raw APIC bytes (JPEG/PNG). Decoded later, not during scan.
    pub art_bytes: Option<Arc<Vec<u8>>>,
}

pub type ArtistMap = BTreeMap<String, Album>;
pub type Library = BTreeMap<String, ArtistMap>;

pub struct Id3Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_num: Option<u32>,
    pub art_bytes: Option<Vec<u8>>,
}

/// Parse a 10-byte ID3v2 header and return the total tag size in bytes
/// (header + frames + footer), or `None` if this file has no ID3v2 tag.
pub fn parse_id3_size(header: &[u8]) -> Option<usize> {
    if header.len() < 10 || &header[0..3] != b"ID3" {
        return None;
    }
    // ID3v2 size: four syncsafe bytes (high bit of each is 0).
    let size = ((header[6] as usize & 0x7F) << 21)
        | ((header[7] as usize & 0x7F) << 14)
        | ((header[8] as usize & 0x7F) << 7)
        | (header[9] as usize & 0x7F);
    Some(10 + size)
}

/// Parse a complete ID3v2 tag from `data` (header included).
pub fn parse_id3_tag(data: &[u8]) -> Option<Id3Tags> {
    let tag = Tag::parse(data).ok()?;

    let title = tag
        .text_field(TextField::Title)
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty());
    let artist = tag
        .text_field(TextField::Artist)
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty());
    let album = tag
        .text_field(TextField::Album)
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty());

    // TRCK is often "3" or "3/12"; take the part before the slash.
    // NOTE: if `TextField::Track` doesn't compile, try `TextField::TrackNumber`.
    let track_num = tag
        .text_field(TextField::Track)
        .and_then(|s| s.split('/').next())
        .and_then(|s| s.trim().parse::<u32>().ok());

    let art_bytes = tag.cover().map(|pic: &AttachedPicture| pic.data.clone());

    Some(Id3Tags {
        title,
        artist,
        album,
        track_num,
        art_bytes,
    })
}

/// Build a library from path strings alone, without reading tags.
/// Useful for skeleton/testing. Firmware will typically use
/// `add_tagged_track` with parsed tag data per file.
pub fn build_library(paths: &[String]) -> Library {
    let mut library = Library::new();
    for path in paths {
        let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
        if !HANDLED_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let title = fallback_title_from_path(path);
        insert_into_library(
            &mut library,
            UNKNOWN_ARTIST.to_string(),
            UNKNOWN_ALBUM.to_string(),
            title,
            path.clone(),
            u32::MAX,
            None,
        );
    }
    sort_library_tracks(&mut library);
    library
}

/// Add a track with parsed tags to the library.
pub fn add_tagged_track(library: &mut Library, path: String, tags: Id3Tags) {
    let title = tags
        .title
        .unwrap_or_else(|| fallback_title_from_path(&path));
    let artist = tags.artist.unwrap_or_else(|| UNKNOWN_ARTIST.to_string());
    let album = tags.album.unwrap_or_else(|| UNKNOWN_ALBUM.to_string());
    let track_num = tags.track_num.unwrap_or(u32::MAX);
    let art = tags.art_bytes.map(Arc::new);

    insert_into_library(library, artist, album, title, path, track_num, art);
}

/// Sort every album's tracks by track number, then path.
pub fn sort_library_tracks(library: &mut Library) {
    for albums in library.values_mut() {
        for album in albums.values_mut() {
            album.tracks.sort_by(|a, b| {
                a.track_num
                    .cmp(&b.track_num)
                    .then_with(|| a.path.cmp(&b.path))
            });
        }
    }
}

fn fallback_title_from_path(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(path)
        .to_string()
}

#[allow(clippy::too_many_arguments)]
fn insert_into_library(
    library: &mut Library,
    artist: String,
    album: String,
    title: String,
    path: String,
    track_num: u32,
    art_bytes: Option<Arc<Vec<u8>>>,
) {
    let album_entry = library
        .entry(artist.clone())
        .or_default()
        .entry(album.clone())
        .or_insert_with(|| Album {
            tracks: Vec::new(),
            art_bytes: None,
        });

    if album_entry.art_bytes.is_none() {
        album_entry.art_bytes = art_bytes;
    }

    album_entry.tracks.push(Track {
        path,
        title,
        artist,
        album,
        track_num,
    });
}

/// Stub: real decoding happens in firmware with tjpgdec-rs / nopng.
/// Kept here so the type is available for future use.
pub fn decode_art(_bytes: &[u8]) -> Option<AlbumArt> {
    None
}

// ============================================================================
// TESTS — these run on your laptop, not on the RP2350.
// Command: cargo test -p mymuse-core --target x86_64-unknown-linux-gnu
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id3_header_size_parses_known_values() {
        // ID3v2.3 header, size field = 16 (syncsafe).
        let header = [b'I', b'D', b'3', 3, 0, 0, 0, 0, 0, 16];
        assert_eq!(parse_id3_size(&header), Some(26));
    }

    #[test]
    fn id3_header_rejects_non_tag() {
        assert_eq!(parse_id3_size(b"hello world"), None);
    }

    #[test]
    fn id3_header_rejects_short_input() {
        assert_eq!(parse_id3_size(b"ID3"), None);
    }

    #[test]
    fn empty_input_gives_empty_library() {
        assert!(build_library(&[]).is_empty());
    }

    #[test]
    fn path_is_preserved() {
        let paths = vec!["Artist/Album/song.mp3".to_string()];
        let lib = build_library(&paths);
        let album = &lib["Unknown Artist"]["Unknown Album"];
        assert_eq!(album.tracks[0].path, "Artist/Album/song.mp3");
    }

    #[test]
    fn non_audio_extension_is_skipped() {
        let paths = vec!["notes.txt".to_string(), "cover.jpg".to_string()];
        assert!(build_library(&paths).is_empty());
    }

    #[test]
    fn fallback_title_strips_directory_and_extension() {
        assert_eq!(fallback_title_from_path("A/B/01 - Song.mp3"), "01 - Song");
    }

    #[test]
    fn parse_real_mp3() {
        let bytes = include_bytes!("../tests/fixtures/sample.mp3");
        let size = parse_id3_size(&bytes[..10]).expect("has ID3v2 tag");
        let tags = parse_id3_tag(&bytes[..size]).expect("parses");
        assert!(tags.title.is_some(), "expected a title tag");
    }
}
