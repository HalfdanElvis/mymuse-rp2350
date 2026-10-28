use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec; // for the vec![] macro

use crate::library::{Library, Track};
use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq)]
pub enum Level {
    Root,
    Artists,
    AlbumsOf(String),
    TracksOf(String, String),
    AllAlbums,
    AllSongs,
    Settings,
}

pub struct NavState {
    pub library: Library,
    pub settings: Settings,
    pub level_stack: Vec<(Level, usize, usize)>,

    all_albums_list: Vec<(String, String)>,
    all_songs_cache: Vec<Track>,
    items_cache: Vec<String>,
}

impl NavState {
    pub fn new(library: Library) -> Self {
        let mut state = Self {
            library,
            settings: Settings::new(),
            level_stack: vec![(Level::Root, 0, 0)],
            all_albums_list: Vec::new(),
            all_songs_cache: Vec::new(),
            items_cache: Vec::new(),
        };
        state.rebuild_all_albums_list();
        state.rebuild_items_cache();
        state
    }

    pub fn current(&self) -> &(Level, usize, usize) {
        self.level_stack.last().unwrap()
    }

    // --- Album art context ----------------------------------------------

    pub fn current_album_context(&self) -> Option<(String, String)> {
        let (level, idx, _) = self.current();
        match level {
            Level::AlbumsOf(artist) => self
                .library
                .get(artist)
                .and_then(|a| a.keys().nth(*idx))
                .map(|album| (artist.clone(), album.clone())),
            Level::TracksOf(artist, album) => Some((artist.clone(), album.clone())),
            Level::AllAlbums => self.all_albums_list.get(*idx).cloned(),
            Level::AllSongs => self
                .all_songs_cache
                .get(*idx)
                .map(|t| (t.artist.clone(), t.album.clone())),
            Level::Root | Level::Artists | Level::Settings => None,
        }
    }

    pub fn get_album_art_bytes(&self, artist: &str, album: &str) -> Option<Arc<Vec<u8>>> {
        self.library.get(artist)?.get(album)?.art_bytes.clone()
    }

    /// Sorted, cached list of every (artist, album). Used by the slideshow.
    pub fn all_albums(&self) -> &[(String, String)] {
        &self.all_albums_list
    }

    // --- Playlist resolution --------------------------------------------

    pub fn get_current_playlist(&self) -> Vec<Track> {
        match &self.current().0 {
            Level::TracksOf(artist, album) => self
                .library
                .get(artist)
                .and_then(|a| a.get(album))
                .map(|al| al.tracks.clone())
                .unwrap_or_default(),
            Level::AllSongs => self.all_songs_cache.clone(),
            _ => vec![],
        }
    }

    // --- Display items --------------------------------------------------

    /// Cached display strings for the current level. O(1).
    pub fn get_items(&self) -> &[String] {
        &self.items_cache
    }

    fn rebuild_items_cache(&mut self) {
        let (level, _, _) = self.current();
        self.items_cache = match level {
            Level::Root => ["Artists", "Albums", "Songs", "Settings"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            Level::Artists => self.library.keys().cloned().collect(),
            Level::AlbumsOf(artist) => self
                .library
                .get(artist)
                .map(|a| a.keys().cloned().collect())
                .unwrap_or_default(),
            Level::TracksOf(artist, album) => self
                .library
                .get(artist)
                .and_then(|a| a.get(album))
                .map(|al| al.tracks.iter().map(|t| t.title.clone()).collect())
                .unwrap_or_default(),
            Level::AllAlbums => self
                .all_albums_list
                .iter()
                .map(|(_, album)| album.clone())
                .collect(),
            Level::AllSongs => self
                .all_songs_cache
                .iter()
                .map(|t| t.title.clone())
                .collect(),
            Level::Settings => self.settings.items(),
        };
    }

    fn rebuild_all_albums_list(&mut self) {
        let mut albums: Vec<(String, String)> = self
            .library
            .iter()
            .flat_map(|(artist, albums_map)| {
                albums_map
                    .keys()
                    .map(move |album| (artist.clone(), album.clone()))
            })
            .collect();
        albums.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        self.all_albums_list = albums;
    }

    fn rebuild_all_songs_cache(&mut self) {
        let mut songs: Vec<Track> = self
            .library
            .values()
            .flat_map(|albums_map| albums_map.values())
            .flat_map(|al| al.tracks.iter().cloned())
            .collect();
        songs.sort_by(|a, b| a.title.cmp(&b.title));
        self.all_songs_cache = songs;
    }

    // --- Navigation -----------------------------------------------------

    /// Returns `Some(index_into_current_playlist)` when the caller should
    /// start playback; `None` when this was a menu drill-down or a settings
    /// cycle.
    pub fn enter(&mut self) -> Option<usize> {
        let (level, selected_idx, _) = self.current().clone();
        if self.items_cache.is_empty() {
            return None;
        }
        let selected = self.items_cache[selected_idx].clone();

        let play = match level {
            Level::Root => {
                self.enter_root(&selected);
                false
            }
            Level::Artists => {
                self.level_stack.push((Level::AlbumsOf(selected), 0, 0));
                false
            }
            Level::AlbumsOf(artist) => {
                self.level_stack
                    .push((Level::TracksOf(artist, selected), 0, 0));
                false
            }
            Level::TracksOf(_, _) => true,
            Level::AllAlbums => {
                self.enter_all_album(selected_idx);
                false
            }
            Level::AllSongs => true,
            Level::Settings => {
                self.settings.cycle(selected_idx);
                false
            }
        };

        if play {
            Some(selected_idx)
        } else {
            self.rebuild_items_cache();
            None
        }
    }

    fn enter_root(&mut self, selected: &str) {
        match selected {
            "Artists" => self.level_stack.push((Level::Artists, 0, 0)),
            "Albums" => self.level_stack.push((Level::AllAlbums, 0, 0)),
            "Songs" => {
                self.rebuild_all_songs_cache();
                self.level_stack.push((Level::AllSongs, 0, 0));
            }
            "Settings" => self.level_stack.push((Level::Settings, 0, 0)),
            _ => {}
        }
    }

    fn enter_all_album(&mut self, idx: usize) {
        if let Some((artist, album)) = self.all_albums_list.get(idx) {
            self.level_stack
                .push((Level::TracksOf(artist.clone(), album.clone()), 0, 0));
        }
    }

    pub fn back(&mut self) -> bool {
        if self.level_stack.len() > 1 {
            self.level_stack.pop();
            self.rebuild_items_cache();
            true
        } else {
            false
        }
    }

    pub fn hovered_artist(&self) -> Option<String> {
        if !matches!(self.current().0, Level::Artists) {
            return None;
        }
        let (_, idx, _) = self.current();
        self.items_cache.get(*idx).cloned()
    }
}
