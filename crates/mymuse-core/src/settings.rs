use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Theme {
    GrayDark,
    SimpleLight,
    PinkLight,
    BlueDark,
}

impl Theme {
    pub fn name(self) -> &'static str {
        match self {
            Theme::GrayDark => "gray_dark",
            Theme::SimpleLight => "simple_light",
            Theme::PinkLight => "pink_light",
            Theme::BlueDark => "blue_dark",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Theme::GrayDark => "Gray (dark)",
            Theme::SimpleLight => "Simple (light)",
            Theme::PinkLight => "Pink (light)",
            Theme::BlueDark => "Blue (dark)",
        }
    }

    fn next(self) -> Self {
        match self {
            Theme::GrayDark => Theme::SimpleLight,
            Theme::SimpleLight => Theme::PinkLight,
            Theme::PinkLight => Theme::BlueDark,
            Theme::BlueDark => Theme::GrayDark,
        }
    }
}

pub struct Settings {
    pub pingpong_scroll: bool,
    pub theme: Theme,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            pingpong_scroll: true,
            theme: Theme::GrayDark,
        }
    }

    pub fn count(&self) -> usize {
        2
    }

    fn label(&self, idx: usize) -> Option<&'static str> {
        match idx {
            0 => Some("Long text effect"),
            1 => Some("Theme"),
            _ => None,
        }
    }

    fn value(&self, idx: usize) -> Option<&'static str> {
        match idx {
            0 => Some(if self.pingpong_scroll {
                "Ping-pong"
            } else {
                "Continuous scroll"
            }),
            1 => Some(self.theme.label()),
            _ => None,
        }
    }

    pub fn cycle(&mut self, idx: usize) {
        match idx {
            0 => self.pingpong_scroll = !self.pingpong_scroll,
            1 => self.theme = self.theme.next(),
            _ => {}
        }
    }

    pub fn items(&self) -> Vec<String> {
        (0..self.count())
            .filter_map(|i| self.label(i).map(String::from))
            .collect()
    }

    pub fn values(&self) -> Vec<String> {
        (0..self.count())
            .filter_map(|i| self.value(i).map(String::from))
            .collect()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings() {
        let s = Settings::new();
        assert!(s.pingpong_scroll);
        assert_eq!(s.theme, Theme::GrayDark);
    }

    #[test]
    fn cycling_pingpong_toggles() {
        let mut s = Settings::new();
        s.cycle(0);
        assert!(!s.pingpong_scroll);
        s.cycle(0);
        assert!(s.pingpong_scroll);
    }

    #[test]
    fn cycling_theme_wraps_after_four() {
        let mut s = Settings::new();
        let mut steps = 0;
        loop {
            let before = s.theme;
            s.cycle(1);
            steps += 1;
            if s.theme == Theme::GrayDark {
                assert_eq!(before, Theme::BlueDark);
                break;
            }
            assert!(steps < 10, "theme cycle did not wrap");
        }
        assert_eq!(steps, 4);
    }

    #[test]
    fn items_and_values_align_with_count() {
        let s = Settings::new();
        assert_eq!(s.items().len(), s.count());
        assert_eq!(s.values().len(), s.count());
    }
}
