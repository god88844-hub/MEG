use ratatui::style::Color;

/// The 12-field theme palette definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub name: &'static str,
    pub chrome_bg: Color,
    pub chrome_fg: Color,
    pub titlebar_bg: Color,
    pub titlebar_fg: Color,
    pub panel_border: Color,
    pub panel_title: Color,
    pub viewport_bg: Color,
    pub viewport_fg: Color,
    pub accent: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
}

pub const CLASSIC: Palette = Palette {
    name: "Classic",
    chrome_bg: Color::Rgb(192, 192, 192),
    chrome_fg: Color::Black,
    titlebar_bg: Color::Rgb(0, 0, 128),
    titlebar_fg: Color::White,
    panel_border: Color::Rgb(128, 128, 128),
    panel_title: Color::Rgb(0, 0, 128),
    viewport_bg: Color::White,
    viewport_fg: Color::Black,
    accent: Color::Rgb(0, 90, 156),
    success: Color::Rgb(0, 115, 0),
    error: Color::Rgb(160, 0, 0),
    warning: Color::Rgb(180, 100, 0),
};

pub const PHOSPHOR: Palette = Palette {
    name: "Phosphor",
    chrome_bg: Color::Rgb(4, 20, 10),
    chrome_fg: Color::Rgb(0, 255, 65),
    titlebar_bg: Color::Rgb(0, 59, 0),
    titlebar_fg: Color::Rgb(127, 255, 159),
    panel_border: Color::Rgb(0, 77, 26),
    panel_title: Color::Rgb(51, 255, 51),
    viewport_bg: Color::Rgb(1, 13, 5),
    viewport_fg: Color::Rgb(0, 255, 65),
    accent: Color::Rgb(51, 255, 51),
    success: Color::Rgb(51, 255, 51),
    error: Color::Rgb(255, 51, 51),
    warning: Color::Rgb(255, 204, 0),
};

pub const DARK_SLATE: Palette = Palette {
    name: "Dark Slate",
    chrome_bg: Color::Rgb(24, 24, 37),
    chrome_fg: Color::Rgb(166, 173, 200),
    titlebar_bg: Color::Rgb(30, 30, 46),
    titlebar_fg: Color::Rgb(205, 214, 244),
    panel_border: Color::Rgb(69, 71, 90),
    panel_title: Color::Rgb(137, 180, 250),
    viewport_bg: Color::Rgb(30, 30, 46),
    viewport_fg: Color::Rgb(205, 214, 244),
    accent: Color::Rgb(137, 180, 250),
    success: Color::Rgb(166, 227, 161),
    error: Color::Rgb(243, 139, 168),
    warning: Color::Rgb(250, 179, 135),
};

pub const DRACULA: Palette = Palette {
    name: "Dracula",
    chrome_bg: Color::Rgb(33, 34, 44),
    chrome_fg: Color::Rgb(98, 114, 164),
    titlebar_bg: Color::Rgb(40, 42, 54),
    titlebar_fg: Color::Rgb(248, 248, 242),
    panel_border: Color::Rgb(68, 71, 90),
    panel_title: Color::Rgb(189, 147, 249),
    viewport_bg: Color::Rgb(40, 42, 54),
    viewport_fg: Color::Rgb(248, 248, 242),
    accent: Color::Rgb(189, 147, 249),
    success: Color::Rgb(80, 250, 123),
    error: Color::Rgb(255, 85, 85),
    warning: Color::Rgb(255, 184, 108),
};

pub const TOKYO_NIGHT: Palette = Palette {
    name: "Tokyo Night",
    chrome_bg: Color::Rgb(22, 22, 30),
    chrome_fg: Color::Rgb(86, 95, 137),
    titlebar_bg: Color::Rgb(26, 27, 38),
    titlebar_fg: Color::Rgb(192, 202, 245),
    panel_border: Color::Rgb(41, 164, 232),
    panel_title: Color::Rgb(122, 162, 247),
    viewport_bg: Color::Rgb(26, 27, 38),
    viewport_fg: Color::Rgb(192, 202, 245),
    accent: Color::Rgb(122, 162, 247),
    success: Color::Rgb(158, 206, 106),
    error: Color::Rgb(247, 118, 142),
    warning: Color::Rgb(224, 175, 104),
};

/// The reference "MEG Terminal Dashboard" look (2026-11-20 user mockup):
/// Near-black canvas, neutral borders, cyan accents, green done-states.
pub const DASHBOARD: Palette = Palette {
    name: "Dashboard",
    chrome_bg: Color::Rgb(3, 7, 12),
    chrome_fg: Color::Rgb(145, 157, 171),
    titlebar_bg: Color::Rgb(3, 7, 12),
    titlebar_fg: Color::Rgb(238, 242, 247),
    panel_border: Color::Rgb(205, 215, 224),
    panel_title: Color::Rgb(238, 242, 247),
    viewport_bg: Color::Rgb(3, 7, 12),
    viewport_fg: Color::Rgb(230, 237, 243),
    accent: Color::Rgb(0, 224, 229),
    success: Color::Rgb(38, 235, 81),
    error: Color::Rgb(248, 81, 73),
    warning: Color::Rgb(227, 179, 65),
};

pub const ALL_PALETTES: &[Palette] = &[
    DASHBOARD,
    DARK_SLATE,
    CLASSIC,
    PHOSPHOR,
    DRACULA,
    TOKYO_NIGHT,
];

pub type ThemePalette = Palette;

/// Resolves a palette by name, safely falling back to DARK_SLATE.
pub fn get_palette_by_name(name: &str) -> &'static Palette {
    match name
        .trim()
        .to_lowercase()
        .replace([' ', '-', '_'], "")
        .as_str()
    {
        "dashboard" | "reference" => &DASHBOARD,
        "classic" => &CLASSIC,
        "phosphor" => &PHOSPHOR,
        "darkslate" | "slate" => &DARK_SLATE,
        "dracula" => &DRACULA,
        "tokyonight" => &TOKYO_NIGHT,
        _ => &DARK_SLATE,
    }
}

/// Resolves a palette by name, case-insensitively, supporting space-collapsed aliases.
pub fn get_palette(name: &str) -> Option<Palette> {
    let query = name.trim().to_lowercase();
    let normalized = query.replace([' ', '-', '_'], "");
    ALL_PALETTES.iter().copied().find(|p| {
        p.name.eq_ignore_ascii_case(name)
            || p.name.to_lowercase().replace([' ', '-', '_'], "") == normalized
    })
}

/// Tracks the active theme, supporting live preview and cancel-restore snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeState {
    pub active: Palette,
    pub snapshot: Option<Palette>,
}

impl Default for ThemeState {
    fn default() -> Self {
        Self {
            active: DARK_SLATE,
            snapshot: None,
        }
    }
}

impl ThemeState {
    pub fn new(palette: Palette) -> Self {
        Self {
            active: palette,
            snapshot: None,
        }
    }

    /// Begins a live preview session, recording the initial palette as snapshot.
    pub fn start_preview(&mut self) {
        if self.snapshot.is_none() {
            self.snapshot = Some(self.active);
        }
    }

    /// Updates active palette with a preview candidate.
    pub fn preview(&mut self, candidate: Palette) {
        if self.snapshot.is_none() {
            self.snapshot = Some(self.active);
        }
        self.active = candidate;
    }

    /// Commits the active preview, discarding the snapshot.
    pub fn commit(&mut self) -> Palette {
        self.snapshot = None;
        self.active
    }

    /// Restores the initial palette snapshot, cancelling the preview.
    pub fn restore(&mut self) -> Palette {
        if let Some(snap) = self.snapshot.take() {
            self.active = snap;
        }
        self.active
    }

    /// Directly sets active palette by name.
    pub fn set_by_name(&mut self, name: &str) {
        self.active = *get_palette_by_name(name);
    }
}
