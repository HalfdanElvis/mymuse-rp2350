//! On-SD cache for decoded album art.
//!
//! File layout (`/mymuse/art.cache`):
//!
//!   [Header]  (32 bytes)
//!     magic:      [u8; 4] = b"MMAC"   ("MyMuse Art Cache")
//!     version:    u16 LE              (bump when format changes)
//!     entry_count:u32 LE
//!     _reserved:  [u8; 22]
//!
//!   [Index]   (entry_count * 24 bytes, sorted by key)
//!     key:        [u8; 16]            (128-bit hash of "artist\0album")
//!     offset:     u32 LE              (byte offset in file to thumbnail data)
//!     width:      u16 LE
//!     height:     u16 LE
//!
//!   [Data]
//!     For each entry: width*height*4 bytes of RGBA8, in index order.
//!
//! On boot, firmware reads the header + full index into RAM (a few KB for
//! thousands of albums), then does binary search per lookup + a single
//! seek+read to fetch the matching thumbnail.

pub const CACHE_MAGIC: [u8; 4] = *b"MMAC";
pub const CACHE_VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 32;
pub const ENTRY_SIZE: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheEntry {
    pub key: [u8; 16],
    pub offset: u32,
    pub width: u16,
    pub height: u16,
}

impl CacheEntry {
    /// Serialize this entry to its 24-byte on-disk form.
    pub fn write_to(&self, buf: &mut [u8; ENTRY_SIZE]) {
        buf[0..16].copy_from_slice(&self.key);
        buf[16..20].copy_from_slice(&self.offset.to_le_bytes());
        buf[20..22].copy_from_slice(&self.width.to_le_bytes());
        buf[22..24].copy_from_slice(&self.height.to_le_bytes());
    }

    /// Parse a 24-byte on-disk entry.
    pub fn from_bytes(buf: &[u8; ENTRY_SIZE]) -> Self {
        let mut key = [0u8; 16];
        key.copy_from_slice(&buf[0..16]);
        let offset = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        let width = u16::from_le_bytes([buf[20], buf[21]]);
        let height = u16::from_le_bytes([buf[22], buf[23]]);
        Self { key, offset, width, height }
    }
}

/// Header serialized into 32 bytes at the start of the cache file.
#[derive(Debug, Clone, Copy)]
pub struct CacheHeader {
    pub entry_count: u32,
    /// FNV-1a digest of all (path, size) pairs at indexing time.
    /// If this changes, the cache is considered stale.
    pub library_digest: u64,
}

impl CacheHeader {
    pub fn write_to(&self, buf: &mut [u8; HEADER_SIZE]) {
        buf[0..4].copy_from_slice(&CACHE_MAGIC);
        buf[4..6].copy_from_slice(&CACHE_VERSION.to_le_bytes());
        buf[6..10].copy_from_slice(&self.entry_count.to_le_bytes());
        buf[10..18].copy_from_slice(&self.library_digest.to_le_bytes());
        // 14 reserved bytes, zero
        for b in &mut buf[18..32] {
            *b = 0;
        }
    }

    pub fn from_bytes(buf: &[u8; HEADER_SIZE]) -> Option<Self> {
        if buf[0..4] != CACHE_MAGIC {
            return None;
        }
        let version = u16::from_le_bytes([buf[4], buf[5]]);
        if version != CACHE_VERSION {
            return None;
        }
        let entry_count = u32::from_le_bytes([buf[6], buf[7], buf[8], buf[9]]);
        let library_digest = u64::from_le_bytes([
            buf[10], buf[11], buf[12], buf[13],
            buf[14], buf[15], buf[16], buf[17],
        ]);
        Some(Self { entry_count, library_digest })
    }
}

/// Compute the stable 128-bit key for an album.
/// Must be identical on the writer and reader sides.
pub fn album_key(artist: &str, album: &str) -> [u8; 16] {
    // 128-bit FNV-1a implemented as two independent 64-bit halves.
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x84222325cbf29ce4;
    for b in artist
        .bytes()
        .chain(core::iter::once(0u8))
        .chain(album.bytes())
    {
        h1 = (h1 ^ b as u64).wrapping_mul(0x100000001b3);
        h2 = (h2 ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&h1.to_le_bytes());
    out[8..].copy_from_slice(&h2.to_le_bytes());
    out
}

/// Binary-search a sorted entry slice for the given album.
/// Entries MUST be sorted by `key`.
pub fn lookup<'a>(entries: &'a [CacheEntry], artist: &str, album: &str) -> Option<&'a CacheEntry> {
    let key = album_key(artist, album);
    entries
        .binary_search_by(|e| e.key.cmp(&key))
        .ok()
        .map(|i| &entries[i])
}

/// Compute the library digest used to detect stale caches.
/// Given (path, file_size) pairs, returns a stable FNV-1a 64-bit hash.
pub fn library_digest<'a, I>(entries: I) -> u64
where
    I: IntoIterator<Item = (&'a str, u32)>,
{
    let mut h: u64 = 0xcbf29ce484222325;
    for (path, size) in entries {
        for b in path.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
        for b in size.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_key_is_deterministic() {
        assert_eq!(
            album_key("Artist", "Album"),
            album_key("Artist", "Album")
        );
    }

    #[test]
    fn album_key_differs_by_artist() {
        assert_ne!(
            album_key("Artist A", "Album"),
            album_key("Artist B", "Album")
        );
    }

    #[test]
    fn album_key_differs_by_album() {
        assert_ne!(
            album_key("Artist", "Album A"),
            album_key("Artist", "Album B")
        );
    }

    #[test]
    fn header_roundtrips() {
        let h = CacheHeader { entry_count: 42, library_digest: 0xDEADBEEF };
        let mut buf = [0u8; HEADER_SIZE];
        h.write_to(&mut buf);
        let parsed = CacheHeader::from_bytes(&buf).unwrap();
        assert_eq!(parsed.entry_count, 42);
        assert_eq!(parsed.library_digest, 0xDEADBEEF);
    }

    #[test]
    fn header_rejects_bad_magic() {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(b"XXXX");
        assert!(CacheHeader::from_bytes(&buf).is_none());
    }

    #[test]
    fn header_rejects_bad_version() {
        let h = CacheHeader { entry_count: 1, library_digest: 0 };
        let mut buf = [0u8; HEADER_SIZE];
        h.write_to(&mut buf);
        buf[4] = 99; // corrupt version
        assert!(CacheHeader::from_bytes(&buf).is_none());
    }

    #[test]
    fn entry_roundtrips() {
        let e = CacheEntry {
            key: album_key("A", "B"),
            offset: 12345,
            width: 128,
            height: 128,
        };
        let mut buf = [0u8; ENTRY_SIZE];
        e.write_to(&mut buf);
        assert_eq!(CacheEntry::from_bytes(&buf), e);
    }

    #[test]
    fn lookup_finds_inserted_key() {
        let key_a = album_key("A", "Album");
        let key_b = album_key("B", "Album");
        let mut entries = [
            CacheEntry { key: key_a, offset: 0, width: 128, height: 128 },
            CacheEntry { key: key_b, offset: 65536, width: 128, height: 128 },
        ];
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        assert!(lookup(&entries, "A", "Album").is_some());
        assert!(lookup(&entries, "B", "Album").is_some());
        assert!(lookup(&entries, "C", "Album").is_none());
    }
}
