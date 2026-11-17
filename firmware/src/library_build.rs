//! Format-aware metadata extraction for the RP2350 firmware.
//!
//! Keeps the existing Storage and mymuse-core APIs intact. Parsers are bounded:
//! no audio payload is read into RAM. Unsupported/malformed metadata falls back
//! to the existing filename/Unknown-* behaviour.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use mymuse_core::library::{
    HANDLED_EXTENSIONS, Id3Tags, Library, add_tagged_track, parse_id3_size, parse_id3_tag,
    sort_library_tracks,
};
use mymuse_hal::Storage;

const MAX_TAG_BYTES: usize = 256 * 1024;
const MAX_COMMENT_PACKET: usize = 64 * 1024;
const MAX_ART_BYTES: usize = 192 * 1024;

pub fn build_library_from_storage<S: Storage>(
    storage: &mut S,
    files: Vec<(String, u32)>,
) -> Library {
    let mut library = Library::new();

    for (path, file_size) in files {
        let ext = path.rsplit('.').next().unwrap_or("");
        if !HANDLED_EXTENSIONS
            .iter()
            .any(|e| e.eq_ignore_ascii_case(ext))
        {
            continue;
        }

        let mut tags = read_metadata(storage, &path, file_size).unwrap_or_else(empty_tags);
        apply_path_metadata_fallback(&path, &mut tags);
        add_tagged_track(&mut library, path, tags);
    }

    sort_library_tracks(&mut library);
    library
}

/// Fill missing metadata from the directory and filename.
///
/// Supports folders named "Artist_-_Album" and filenames such as
/// "05 - Song Title.mp3". Embedded tags always take priority.
fn apply_path_metadata_fallback(path: &str, tags: &mut Id3Tags) {
    let mut parts = path.rsplit('/');
    let filename = parts.next().unwrap_or(path);
    let parent = parts.next().unwrap_or("");

    if tags.artist.is_none() || tags.album.is_none() {
        if let Some((artist, album)) = parent.split_once("_-_") {
            let artist = artist.replace('_', " ");
            let album = album.replace('_', " ");

            if tags.artist.is_none() && !artist.trim().is_empty() {
                tags.artist = Some(artist.trim().to_string());
            }
            if tags.album.is_none() && !album.trim().is_empty() {
                tags.album = Some(album.trim().to_string());
            }
        }
    }

    if tags.title.is_none() {
        let stem = filename
            .rsplit_once('.')
            .map(|(name, _)| name)
            .unwrap_or(filename);
        let title = if let Some((prefix, rest)) = stem.split_once(" - ") {
            if !prefix.trim().is_empty() && prefix.trim().chars().all(|c| c.is_ascii_digit()) {
                rest
            } else {
                stem
            }
        } else {
            stem
        };

        if !title.trim().is_empty() {
            tags.title = Some(title.trim().to_string());
        }
    }
}

fn read_metadata<S: Storage>(s: &mut S, path: &str, size: u32) -> Option<Id3Tags> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => read_id3(s, path, size),
        "flac" => read_flac(s, path, size),
        "ogg" | "oga" | "opus" => read_ogg_comments(s, path, size),
        "wav" => read_wav_info(s, path, size),
        "m4a" | "m4b" => read_mp4(s, path, size),
        // AU/DSD have no single consistently-used metadata layout in the
        // desktop project; retain filename fallback rather than guess.
        _ => None,
    }
}

const MAX_ID3_TEXT_FRAME_BYTES: usize = 4096;

/// Read ID3v2 metadata a frame at a time.
///
/// Does not allocate the complete ID3 tag. Large APIC/image and other
/// frames are skipped, while the small text frames are passed to the
/// existing parser.
fn read_id3<S: Storage>(s: &mut S, path: &str, size: u32) -> Option<Id3Tags> {
    if size < 10 {
        return None;
    }

    let header = s.read_at(path, 0, 10).ok()?;

    if header.len() != 10 || &header[0..3] != b"ID3" {
        return None;
    }

    let version = header[3];

    // This parser supports ID3v2.3 and ID3v2.4.
    if version != 3 && version != 4 {
        return None;
    }

    let flags = header[5];

    // Reject unsupported tag-level unsynchronisation rather than
    // silently parsing incorrect frame boundaries.
    if flags & 0x80 != 0 {
        return None;
    }

    let total = parse_id3_size(&header)?;

    if total < 10 {
        return None;
    }

    // The standard text frames are at the start of these files.
    // Read them together rather than seeking separately for every frame.
    // This also avoids having to read the very large APIC artwork frame.
    let prefix_len = core::cmp::min(total, 16 * 1024);
    if let Ok(prefix) = s.read_at(path, 0, prefix_len) {
        let mut prefix_tags = empty_tags();
        parse_id3_frames(&prefix, &mut prefix_tags);

        // Artist and album are enough to avoid the Unknown-* fallback.
        if prefix_tags.artist.is_some() && prefix_tags.album.is_some() {
            return Some(prefix_tags);
        }
    }

    let mut tags = empty_tags();

    let mut offset = 10usize;
    let tag_end = total;

    // Skip an optional extended header.
    if flags & 0x40 != 0 {
        let ext_header = s.read_at(path, 10, 4).ok()?;

        if ext_header.len() != 4 {
            return None;
        }

        let ext_size = if version == 4 {
            decode_syncsafe_u32(&ext_header)?
        } else {
            u32::from_be_bytes([ext_header[0], ext_header[1], ext_header[2], ext_header[3]])
        } as usize;

        offset = if version == 4 {
            // ID3v2.4 extended-header size includes its own 4 bytes.
            10usize.checked_add(ext_size)?
        } else {
            // ID3v2.3 extended-header size excludes its own 4 bytes.
            10usize.checked_add(4)?.checked_add(ext_size)?
        };

        if offset > tag_end {
            return None;
        }
    }

    while offset.checked_add(10)? <= tag_end {
        let frame_offset = u32::try_from(offset).ok()?;
        let frame_header = s.read_at(path, frame_offset, 10).ok()?;

        if frame_header.len() != 10 {
            break;
        }

        // Zero bytes indicate padding / end of frames.
        if frame_header[0] == 0 {
            break;
        }

        let frame_id = &frame_header[0..4];

        // Frame IDs consist of uppercase ASCII letters and digits.
        if !frame_id
            .iter()
            .all(|b| (*b >= b'A' && *b <= b'Z') || (*b >= b'0' && *b <= b'9'))
        {
            break;
        }

        let frame_size = if version == 4 {
            decode_syncsafe_u32(&frame_header[4..8])?
        } else {
            u32::from_be_bytes([
                frame_header[4],
                frame_header[5],
                frame_header[6],
                frame_header[7],
            ])
        } as usize;

        let payload_offset = offset.checked_add(10)?;
        let next_offset = payload_offset.checked_add(frame_size)?;

        if next_offset > tag_end {
            break;
        }

        // Read and decode only the metadata text frames.
        // Large frames such as APIC are skipped without allocating them.
        let is_text_frame = frame_id == b"TIT2"
            || frame_id == b"TPE1"
            || frame_id == b"TALB"
            || frame_id == b"TRCK";

        if is_text_frame && frame_size > 0 && frame_size <= MAX_ID3_TEXT_FRAME_BYTES {
            let payload_offset_u32 = u32::try_from(payload_offset).ok()?;
            let payload = s.read_at(path, payload_offset_u32, frame_size).ok()?;

            if payload.len() == frame_size {
                let value = decode_id3_text(&payload)
                    .trim_matches('\0')
                    .trim()
                    .to_string();

                match frame_id {
                    b"TIT2" => set_if_empty(&mut tags.title, value),
                    b"TPE1" => set_if_empty(&mut tags.artist, value),
                    b"TALB" => set_if_empty(&mut tags.album, value),
                    b"TRCK" => tags.track_num = parse_track(&value),
                    _ => {}
                }
            }
        }

        offset = next_offset;
    }

    Some(tags)
}

fn decode_syncsafe_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 4 {
        return None;
    }

    if bytes[0] & 0x80 != 0 || bytes[1] & 0x80 != 0 || bytes[2] & 0x80 != 0 || bytes[3] & 0x80 != 0
    {
        return None;
    }

    Some(
        ((bytes[0] as u32) << 21)
            | ((bytes[1] as u32) << 14)
            | ((bytes[2] as u32) << 7)
            | (bytes[3] as u32),
    )
}

fn encode_syncsafe_u32(value: u32) -> [u8; 4] {
    [
        ((value >> 21) & 0x7f) as u8,
        ((value >> 14) & 0x7f) as u8,
        ((value >> 7) & 0x7f) as u8,
        (value & 0x7f) as u8,
    ]
}

fn parse_id3_frames(b: &[u8], tags: &mut Id3Tags) {
    if b.len() < 10 || &b[..3] != b"ID3" {
        return;
    }

    let version = b[3];
    if version != 3 && version != 4 {
        return;
    }

    let mut p = 10usize;

    // Extended header, if present.
    if b[5] & 0x40 != 0 {
        if p + 4 > b.len() {
            return;
        }

        let ext_size = if version == 4 {
            syncsafe_u32(&b[p..p + 4]) as usize
        } else {
            u32::from_be_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]) as usize
        };

        p = match p.checked_add(ext_size + if version == 3 { 4 } else { 0 }) {
            Some(v) if v <= b.len() => v,
            _ => return,
        };
    }

    while p + 10 <= b.len() {
        let id = &b[p..p + 4];

        if id.iter().all(|v| *v == 0) {
            break;
        }

        let frame_size = if version == 4 {
            syncsafe_u32(&b[p + 4..p + 8]) as usize
        } else {
            u32::from_be_bytes([b[p + 4], b[p + 5], b[p + 6], b[p + 7]]) as usize
        };

        let frame_flags = b[p + 8];
        p += 10;

        if frame_size == 0 || p.checked_add(frame_size).map_or(true, |v| v > b.len()) {
            break;
        }

        let data = &b[p..p + frame_size];
        p += frame_size;

        // Ignore compressed or encrypted frames.
        if frame_flags & 0x0c != 0 {
            continue;
        }

        if data.is_empty() {
            continue;
        }

        let value = decode_id3_text(data);

        match id {
            b"TIT2" => set_if_empty(&mut tags.title, value),
            b"TPE1" => set_if_empty(&mut tags.artist, value),
            b"TALB" => set_if_empty(&mut tags.album, value),
            b"TRCK" => tags.track_num = parse_track(&value),
            _ => {}
        }
    }
}

fn syncsafe_u32(b: &[u8]) -> u32 {
    if b.len() < 4 {
        return 0;
    }

    ((b[0] as u32 & 0x7f) << 21)
        | ((b[1] as u32 & 0x7f) << 14)
        | ((b[2] as u32 & 0x7f) << 7)
        | (b[3] as u32 & 0x7f)
}

fn decode_id3_text(data: &[u8]) -> String {
    if data.len() < 2 {
        return String::new();
    }

    let encoding = data[0];
    let bytes = &data[1..];

    match encoding {
        // ISO-8859-1
        0 => {
            let mut out = String::new();
            for &b in bytes {
                if b == 0 {
                    break;
                }
                out.push(char::from(b));
            }
            out
        }

        // UTF-16 with BOM
        1 => {
            let mut words = Vec::new();
            for pair in bytes.chunks_exact(2) {
                let word = u16::from_le_bytes([pair[0], pair[1]]);
                if word == 0 {
                    break;
                }
                words.push(word);
            }

            if bytes.len() >= 2 && bytes[0] == 0xfe && bytes[1] == 0xff {
                words.clear();
                for pair in bytes[2..].chunks_exact(2) {
                    let word = u16::from_be_bytes([pair[0], pair[1]]);
                    if word == 0 {
                        break;
                    }
                    words.push(word);
                }
            } else if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] == 0xfe {
                words.clear();
                for pair in bytes[2..].chunks_exact(2) {
                    let word = u16::from_le_bytes([pair[0], pair[1]]);
                    if word == 0 {
                        break;
                    }
                    words.push(word);
                }
            }

            core::char::decode_utf16(words)
                .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        }

        // UTF-16BE without BOM
        2 => {
            let words = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .take_while(|w| *w != 0);

            core::char::decode_utf16(words)
                .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        }

        // UTF-8
        3 => {
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..end]).trim().to_string()
        }

        _ => String::new(),
    }
}

fn read_flac<S: Storage>(s: &mut S, path: &str, size: u32) -> Option<Id3Tags> {
    if size < 8 {
        return None;
    }
    let head = s.read_at(path, 0, 4).ok()?;
    if head.as_slice() != b"fLaC" {
        return None;
    }
    let mut pos = 4u32;
    let mut tags = empty_tags();
    let mut got_comment = false;
    for _ in 0..32 {
        if pos.checked_add(4)? > size {
            break;
        }
        let h = s.read_at(path, pos, 4).ok()?;
        if h.len() != 4 {
            break;
        }
        let last = h[0] & 0x80 != 0;
        let kind = h[0] & 0x7f;
        let len = ((h[1] as usize) << 16) | ((h[2] as usize) << 8) | h[3] as usize;
        pos = pos.checked_add(4)?;
        if len > MAX_TAG_BYTES || pos.checked_add(len as u32)? > size {
            break;
        }
        if kind == 4 && len <= MAX_COMMENT_PACKET {
            let b = s.read_at(path, pos, len).ok()?;
            parse_vorbis_comments(&b, &mut tags);
            got_comment = true;
        } else if kind == 6 && len <= MAX_ART_BYTES && tags.art_bytes.is_none() {
            let b = s.read_at(path, pos, len).ok()?;
            // FLAC PICTURE block: parse MIME, description, dimensions, then data.
            if let Some(img) = flac_picture_data(&b) {
                tags.art_bytes = Some(img);
            }
        }
        pos = pos.checked_add(len as u32)?;
        if last {
            break;
        }
    }
    if got_comment || tags.art_bytes.is_some() {
        Some(tags)
    } else {
        None
    }
}

fn flac_picture_data(b: &[u8]) -> Option<Vec<u8>> {
    if b.len() < 32 {
        return None;
    }
    let mut p = 4usize; // picture type
    let mime = be_u32(b, p)? as usize;
    p += 4;
    p = p.checked_add(mime)?;
    let desc = be_u32(b, p)? as usize;
    p += 4;
    p = p.checked_add(desc)?;
    if p.checked_add(20)? > b.len() {
        return None;
    }
    p += 16; // width, height, depth, colors
    let n = be_u32(b, p)? as usize;
    p += 4;
    if n > MAX_ART_BYTES || p.checked_add(n)? > b.len() {
        return None;
    }
    Some(b[p..p + n].to_vec())
}

fn read_ogg_comments<S: Storage>(s: &mut S, path: &str, size: u32) -> Option<Id3Tags> {
    // Scan a bounded prefix for the Vorbis-comment packet signature. This
    // handles common single-page/comment packets without allocating the audio.
    let cap = core::cmp::min(size as usize, 256 * 1024);
    let data = s.read_at(path, 0, cap).ok()?;
    let mut tags = empty_tags();
    let mut found = false;
    for sig in [b"\x03vorbis".as_slice(), b"OpusTags".as_slice()] {
        if let Some(i) = find_bytes(&data, sig) {
            let start = i + sig.len();
            if start < data.len() {
                parse_vorbis_comments(&data[start..], &mut tags);
                found = tags.title.is_some() || tags.artist.is_some() || tags.album.is_some();
                if found {
                    break;
                }
            }
        }
    }
    if found { Some(tags) } else { None }
}

fn read_wav_info<S: Storage>(s: &mut S, path: &str, size: u32) -> Option<Id3Tags> {
    if size < 12 {
        return None;
    }
    let h = s.read_at(path, 0, 12).ok()?;
    if &h[0..4] != b"RIFF" || &h[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12u32;
    let mut tags = empty_tags();
    let mut found = false;
    for _ in 0..128 {
        if pos.checked_add(8)? > size {
            break;
        }
        let h = s.read_at(path, pos, 8).ok()?;
        if h.len() != 8 {
            break;
        }
        let len = le_u32(&h, 4)? as usize;
        let data_pos = pos.checked_add(8)?;
        if len > MAX_COMMENT_PACKET || data_pos.checked_add(len as u32)? > size {
            break;
        }
        if &h[0..4] == b"LIST" && len >= 4 {
            let b = s.read_at(path, data_pos, len).ok()?;
            if b.len() == len && &b[0..4] == b"INFO" {
                parse_info_chunk(&b[4..], &mut tags);
                found = true;
            }
        }
        pos = data_pos
            .checked_add(len as u32)?
            .checked_add((len & 1) as u32)?;
    }
    if found { Some(tags) } else { None }
}

fn parse_info_chunk(b: &[u8], t: &mut Id3Tags) {
    let mut p = 0usize;
    while p + 8 <= b.len() {
        let id = &b[p..p + 4];
        let n = le_u32(b, p + 4).unwrap_or(0) as usize;
        p += 8;
        if p + n > b.len() {
            break;
        }
        let v = trim_nul(&b[p..p + n]);
        let val = String::from_utf8_lossy(v).trim().to_string();
        match id {
            b"INAM" => set_if_empty(&mut t.title, val),
            b"IART" => set_if_empty(&mut t.artist, val),
            b"IPRD" => set_if_empty(&mut t.album, val),
            b"ITRK" => t.track_num = parse_track(&val),
            _ => {}
        }
        p += n + (n & 1);
    }
}

fn read_mp4<S: Storage>(s: &mut S, path: &str, size: u32) -> Option<Id3Tags> {
    // Walk top-level atoms and parse the ilst metadata atom when it fits in a
    // small bounded read. Nested container traversal is intentionally bounded.
    let cap = core::cmp::min(size as usize, MAX_TAG_BYTES);
    let b = s.read_at(path, 0, cap).ok()?;
    if b.len() < 8 {
        return None;
    }
    let mut tags = empty_tags();
    let mut found = false;
    scan_mp4_atoms(&b, 0, b.len(), &mut tags, &mut found, 0);
    if found { Some(tags) } else { None }
}

fn scan_mp4_atoms(
    b: &[u8],
    mut p: usize,
    end: usize,
    t: &mut Id3Tags,
    found: &mut bool,
    depth: u8,
) {
    if depth > 8 {
        return;
    }
    while p + 8 <= end {
        let n32 = be_u32(b, p).unwrap_or(0);
        let kind = &b[p + 4..p + 8];
        let (n, head) = if n32 == 1 {
            if p + 16 > end {
                break;
            }
            (be_u64(b, p + 8).unwrap_or(u64::MAX) as usize, 16usize)
        } else {
            (n32 as usize, 8usize)
        };
        if n < head || p.checked_add(n).map_or(true, |v| v > end) {
            break;
        }
        let start = p + head;
        let stop = p + n;
        if kind == b"ilst" || kind == b"moov" || kind == b"udta" || kind == b"meta" {
            let skip = if kind == b"meta" {
                core::cmp::min(4, stop - start)
            } else {
                0
            };
            scan_mp4_atoms(b, start + skip, stop, t, found, depth + 1);
        } else {
            let key = match kind {
                b"\xa9nam" => Some(0),
                b"\xa9ART" | b"aART" => Some(1),
                b"\xa9alb" => Some(2),
                b"trkn" => Some(3),
                _ => None,
            };
            if let Some(k) = key {
                let mut q = start;
                while q + 8 <= stop {
                    let dn = be_u32(b, q).unwrap_or(0) as usize;
                    if dn < 8 || q + dn > stop {
                        break;
                    }
                    if &b[q + 4..q + 8] == b"data" && q + 16 <= stop {
                        let v = &b[q + 16..q + dn];
                        match k {
                            0 => set_if_empty(
                                &mut t.title,
                                String::from_utf8_lossy(v)
                                    .trim_matches(char::from(0))
                                    .to_string(),
                            ),
                            1 => set_if_empty(
                                &mut t.artist,
                                String::from_utf8_lossy(v)
                                    .trim_matches(char::from(0))
                                    .to_string(),
                            ),
                            2 => set_if_empty(
                                &mut t.album,
                                String::from_utf8_lossy(v)
                                    .trim_matches(char::from(0))
                                    .to_string(),
                            ),
                            3 if v.len() >= 6 => {
                                t.track_num = Some(u16::from_be_bytes([v[2], v[3]]) as u32);
                            }
                            _ => {}
                        }
                        *found = true;
                    }
                    q += dn;
                }
            }
        }
        p = stop;
    }
}

fn parse_vorbis_comments(b: &[u8], t: &mut Id3Tags) {
    let mut p = 0usize;
    let vendor = le_u32(b, p).unwrap_or(usize::MAX);
    p += 4;
    if vendor > b.len().saturating_sub(p) {
        return;
    }
    p += vendor;
    let count = match le_u32(b, p) {
        Some(v) if v <= 4096 => v,
        _ => return,
    };
    p += 4;
    for _ in 0..count {
        let n = match le_u32(b, p) {
            Some(v) if v <= b.len().saturating_sub(p + 4) => v,
            _ => break,
        };
        p += 4;
        let raw = &b[p..p + n];
        p += n;
        let s = String::from_utf8_lossy(raw);
        let mut it = s.splitn(2, '=');
        let key = it.next().unwrap_or("").to_ascii_uppercase();
        let val = it.next().unwrap_or("").trim().to_string();
        match key.as_str() {
            "TITLE" => set_if_empty(&mut t.title, val),
            "ARTIST" | "ALBUMARTIST" => set_if_empty(&mut t.artist, val),
            "ALBUM" => set_if_empty(&mut t.album, val),
            "TRACKNUMBER" | "TRACK" => t.track_num = parse_track(&val),
            _ => {}
        }
    }
}

fn parse_track(s: &str) -> Option<u32> {
    let n = s.split('/').next()?.trim().parse::<u32>().ok()?;
    if n == 0 { None } else { Some(n) }
}
fn set_if_empty(slot: &mut Option<String>, v: String) {
    if slot.is_none() && !v.is_empty() {
        *slot = Some(v);
    }
}
fn trim_nul(b: &[u8]) -> &[u8] {
    let end = b.iter().position(|x| *x == 0).unwrap_or(b.len());
    &b[..end]
}
fn find_bytes(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    h.windows(n.len()).position(|w| w == n)
}
fn le_u32(b: &[u8], p: usize) -> Option<usize> {
    let a = b.get(p..p + 4)?;
    Some(u32::from_le_bytes([a[0], a[1], a[2], a[3]]) as usize)
}
fn be_u32(b: &[u8], p: usize) -> Option<usize> {
    let a = b.get(p..p + 4)?;
    Some(u32::from_be_bytes([a[0], a[1], a[2], a[3]]) as usize)
}
fn be_u64(b: &[u8], p: usize) -> Option<u64> {
    let a = b.get(p..p + 8)?;
    Some(u64::from_be_bytes([
        a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7],
    ]))
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
