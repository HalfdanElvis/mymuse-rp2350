//! Field names match the original `themes/*.lua` keys 1:1 so you can
//! copy hex values straight across. Each value is 0xAARRGGBB.

use crate::settings::Theme;

pub struct ThemeColors {
    pub bg: u32,
    pub status_bg: u32,
    pub status_text: u32,
    pub status_off: u32,
    pub header_top: u32,
    pub header_bot: u32,
    pub header_hilite: u32,
    pub header_shadow: u32,
    pub header_text: u32,
    pub item_text: u32,
    pub selected: u32,
    pub art_frame: u32,
    pub nowplay_bg: u32,
    pub nowplay_text: u32,
    pub nowplay_dim: u32,
    pub progress_bg: u32,
    pub progress_fg: u32,
    pub battery: u32,
    pub placeholder_colors: &'static [u32],
}

pub const GRAY_DARK: ThemeColors = ThemeColors {
    bg: 0xFF1A1A1A,
    status_bg: 0xFF000000,
    status_text: 0xFFE0E0E0,
    status_off: 0xFF505050,
    header_top: 0xFF3A3A3A,
    header_bot: 0xFF1A1A1A,
    header_hilite: 0xFF505050,
    header_shadow: 0xFF000000,
    header_text: 0xFFFFFFFF,
    item_text: 0xFFCCCCCC,
    selected: 0xFF404040,
    art_frame: 0xFF303030,
    nowplay_bg: 0xFF0D0D0D,
    nowplay_text: 0xFFF0F0F0,
    nowplay_dim: 0xFF909090,
    progress_bg: 0xFF202020,
    progress_fg: 0xFF60A0E0,
    battery: 0xFF60C060,
    placeholder_colors: &[0xFF3A3A3A, 0xFF4A4A4A, 0xFF2A2A2A],
};

pub const SIMPLE_LIGHT: ThemeColors = ThemeColors {
    bg: 0xFFF5F5F5,
    status_bg: 0xFFFFFFFF,
    status_text: 0xFF202020,
    status_off: 0xFFB0B0B0,
    header_top: 0xFFFFFFFF,
    header_bot: 0xFFE0E0E0,
    header_hilite: 0xFFFFFFFF,
    header_shadow: 0xFFB0B0B0,
    header_text: 0xFF000000,
    item_text: 0xFF202020,
    selected: 0xFFD0D0D0,
    art_frame: 0xFFC0C0C0,
    nowplay_bg: 0xFFEEEEEE,
    nowplay_text: 0xFF000000,
    nowplay_dim: 0xFF707070,
    progress_bg: 0xFFCCCCCC,
    progress_fg: 0xFF3070C0,
    battery: 0xFF308030,
    placeholder_colors: &[0xFFD0D0D0, 0xFFE0E0E0, 0xFFC0C0C0],
};

pub const PINK_LIGHT: ThemeColors = ThemeColors {
    bg: 0xFFFDE8EF,
    status_bg: 0xFFFFE0EB,
    status_text: 0xFF602040,
    status_off: 0xFFB890A0,
    header_top: 0xFFFFD0E0,
    header_bot: 0xFFF8B8D0,
    header_hilite: 0xFFFFF0F8,
    header_shadow: 0xFFB06080,
    header_text: 0xFF401020,
    item_text: 0xFF301020,
    selected: 0xFFFFC0D8,
    art_frame: 0xFFE0A0B8,
    nowplay_bg: 0xFFF8D0E0,
    nowplay_text: 0xFF301020,
    nowplay_dim: 0xFFA06080,
    progress_bg: 0xFFE8C0D0,
    progress_fg: 0xFFD04080,
    battery: 0xFF40A050,
    placeholder_colors: &[0xFFFFC0D8, 0xFFFFD8E8, 0xFFF0B0C8],
};

pub const BLUE_DARK: ThemeColors = ThemeColors {
    bg: 0xFF101A2A,
    status_bg: 0xFF08101C,
    status_text: 0xFFD0E0F0,
    status_off: 0xFF405070,
    header_top: 0xFF203858,
    header_bot: 0xFF101A2A,
    header_hilite: 0xFF3A5070,
    header_shadow: 0xFF000814,
    header_text: 0xFFE8F0FF,
    item_text: 0xFFB8C8E0,
    selected: 0xFF284870,
    art_frame: 0xFF203040,
    nowplay_bg: 0xFF081020,
    nowplay_text: 0xFFE8F0FF,
    nowplay_dim: 0xFF7080A0,
    progress_bg: 0xFF182030,
    progress_fg: 0xFF4090D0,
    battery: 0xFF60C060,
    placeholder_colors: &[0xFF203858, 0xFF284870, 0xFF182838],
};

pub fn colors(theme: Theme) -> &'static ThemeColors {
    match theme {
        Theme::GrayDark => &GRAY_DARK,
        Theme::SimpleLight => &SIMPLE_LIGHT,
        Theme::PinkLight => &PINK_LIGHT,
        Theme::BlueDark => &BLUE_DARK,
    }
}
