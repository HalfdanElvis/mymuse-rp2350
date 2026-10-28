use alloc::format;
use alloc::string::String;

use crate::nav::Level;

/// Format a duration (seconds since Unix epoch) as HH:MM.
/// Firmware supplies the seconds — core doesn't know about clocks.
pub fn clock_string(utc_secs_since_epoch: u64) -> String {
    format!(
        "{:02}:{:02}",
        (utc_secs_since_epoch / 3600) % 24,
        (utc_secs_since_epoch / 60) % 60,
    )
}

/// Human-readable title for the current nav level.
pub fn level_title(level: &Level) -> String {
    match level {
        Level::Root => "MyMuse".into(),
        Level::Artists => "Artists".into(),
        Level::AlbumsOf(a) => a.clone(),
        Level::TracksOf(a, al) => format!("{} - {}", a, al),
        Level::AllAlbums => "Albums".into(),
        Level::AllSongs => "Songs".into(),
        Level::Settings => "Settings".into(),
    }
}

/// Deterministic pseudo-random index. Firmware supplies the seed
/// (e.g. from a counter, RTC, or entropy source).
pub fn pseudo_random_index(seed: u64, max: usize) -> usize {
    let mut x = seed;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D049BB133111EB);
    x ^= x >> 31;
    (x as usize) % max.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_string_formats_hours_and_minutes() {
        // 3661 s = 1h 1m
        assert_eq!(clock_string(3661), "01:01");
        assert_eq!(clock_string(0), "00:00");
        assert_eq!(clock_string(86_399), "23:59");
    }

    #[test]
    fn level_title_covers_every_variant() {
        assert_eq!(level_title(&Level::Root), "MyMuse");
        assert_eq!(level_title(&Level::Settings), "Settings");
        assert_eq!(level_title(&Level::TracksOf("A".into(), "B".into())), "A - B");
    }

    #[test]
    fn pseudo_random_is_deterministic_for_same_seed() {
        assert_eq!(pseudo_random_index(42, 10), pseudo_random_index(42, 10));
    }

    #[test]
    fn pseudo_random_stays_in_range() {
        for seed in 0..1000u64 {
            assert!(pseudo_random_index(seed, 7) < 7);
        }
    }
}
