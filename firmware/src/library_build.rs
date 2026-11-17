//! Build the core library by reading ID3v2 tags from files on the SD card.

use alloc::string::String;
use alloc::vec::Vec;

use mymuse_core::library::{
    HANDLED_EXTENSIONS, Id3Tags, Library, add_tagged_track, parse_id3_size, parse_id3_tag,
    sort_library_tracks,
};
use mymuse_hal::Storage;

// Avoid allocating an unbounded tag buffer from a corrupt or hostile file.
const MAX_ID3_TAG_BYTES: usize = 256 * 1024;

/// Scan results must contain file paths relative to the SD-card music root.
///
/// For MP3 files with an ID3v2 tag, artist/album/title/track number and cover
/// bytes are read from that tag. Files without a readable ID3v2 tag still
/// appear in the library using filename/Unknown-* fallbacks.
pub fn build_library_from_storage<S: Storage>(
    storage: &mut S,
    files: Vec<(String, u32)>,
) -> Library {
    let mut library = Library::new();

    for (path, file_size) in files {
        let extension = path.rsplit('.').next().unwrap_or("");
        if !HANDLED_EXTENSIONS
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(extension))
        {
            continue;
        }

        let tags = read_id3_tags(storage, &path, file_size).unwrap_or_else(empty_tags);
        add_tagged_track(&mut library, path, tags);
    }

    sort_library_tracks(&mut library);
    library
}

fn read_id3_tags<S: Storage>(storage: &mut S, path: &str, file_size: u32) -> Option<Id3Tags> {
    if file_size < 10 {
        return None;
    }

    let header = storage.read_at(path, 0, 10).ok()?;
    if header.len() < 10 {
        return None;
    }

    let tag_size = parse_id3_size(&header)?;
    if tag_size < 10 || tag_size > MAX_ID3_TAG_BYTES || tag_size > file_size as usize {
        return None;
    }

    let tag_bytes = storage.read_at(path, 0, tag_size).ok()?;
    if tag_bytes.len() != tag_size {
        return None;
    }

    parse_id3_tag(&tag_bytes)
}

fn empty_tags() -> Id3Tags {
    Id3Tags {
        title: None,
        artist: None,
        album: None,
        track_num: None,
        art_bytes: None,
    }
}
