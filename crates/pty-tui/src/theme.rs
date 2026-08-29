//! Themes and semantic colour tokens, ported from `src/tui/colors.ts:273-357`
//! and `src/tui/tokens.ts`.
//!
//! A [`Theme`] has 13 slots, each `Some(rgb)` or `None` ("use the terminal's
//! default"). Widgets style with the nine semantic [`Color`] names; the one
//! name → slot map lives in [`Color::slot`] and everything resolves through
//! it. [`Theme::to_palette`] turns a theme into the 16 ANSI colours an
//! embedded terminal is given (`builders.ts:35-61`).

use ratatui::symbols::border;

/// An `[r, g, b]` triple.
pub type Rgb = (u8, u8, u8);

/// The 13 theme slots (`colors.ts:273-287`). `None` = terminal default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Theme {
    pub bg1: Option<Rgb>,
    pub bg2: Option<Rgb>,
    pub bg_hi: Option<Rgb>,
    pub bg_ac: Option<Rgb>,
    pub fg1: Option<Rgb>,
    pub fg2: Option<Rgb>,
    pub fg_ac: Option<Rgb>,
    pub fg_mu: Option<Rgb>,
    pub ok: Option<Rgb>,
    pub warn: Option<Rgb>,
    pub err: Option<Rgb>,
    pub info: Option<Rgb>,
    pub border: Option<Rgb>,
}

/// The names of the 13 slots, in declaration order (`colors.ts:274-286`).
pub const SLOT_NAMES: [&str; 13] = [
    "bg1", "bg2", "bgHi", "bgAc", "fg1", "fg2", "fgAc", "fgMu", "ok", "warn", "err", "info",
    "border",
];

/// A colour a widget asks for: one of the nine semantic tokens or a literal
/// triple (`nodes.ts:6-10`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Primary,
    Secondary,
    Accent,
    Muted,
    Ok,
    Warn,
    Error,
    Info,
    Border,
    Rgb(u8, u8, u8),
}

/// The nine semantic token names in declaration order (`tokens.ts:20-30`).
pub const SEMANTIC_NAMES: [&str; 9] = [
    "primary",
    "secondary",
    "accent",
    "muted",
    "ok",
    "warn",
    "error",
    "info",
    "border",
];

/// The nine semantic colours in declaration order.
pub const SEMANTIC_COLORS: [Color; 9] = [
    Color::Primary,
    Color::Secondary,
    Color::Accent,
    Color::Muted,
    Color::Ok,
    Color::Warn,
    Color::Error,
    Color::Info,
    Color::Border,
];

impl Color {
    /// The theme slot backing a semantic name (`SEMANTIC_SLOTS`,
    /// `tokens.ts:20-30`); `None` for a literal triple.
    pub fn slot(&self) -> Option<&'static str> {
        Some(match self {
            Color::Primary => "fg1",
            Color::Secondary => "fg2",
            Color::Accent => "fgAc",
            Color::Muted => "fgMu",
            Color::Ok => "ok",
            Color::Warn => "warn",
            Color::Error => "err",
            Color::Info => "info",
            Color::Border => "border",
            Color::Rgb(..) => return None,
        })
    }

    /// The semantic name (`"accent"`, …); `None` for a literal triple.
    pub fn name(&self) -> Option<&'static str> {
        Some(match self {
            Color::Primary => "primary",
            Color::Secondary => "secondary",
            Color::Accent => "accent",
            Color::Muted => "muted",
            Color::Ok => "ok",
            Color::Warn => "warn",
            Color::Error => "error",
            Color::Info => "info",
            Color::Border => "border",
            Color::Rgb(..) => return None,
        })
    }

    /// Parse a semantic name; unknown names are `None` (Node resolves them
    /// to the host default).
    pub fn parse(name: &str) -> Option<Color> {
        SEMANTIC_NAMES
            .iter()
            .position(|n| *n == name)
            .map(|i| SEMANTIC_COLORS[i])
    }
}

impl From<Rgb> for Color {
    fn from((r, g, b): Rgb) -> Self {
        Color::Rgb(r, g, b)
    }
}

impl Theme {
    /// A slot by name.
    pub fn slot(&self, name: &str) -> Option<Rgb> {
        match name {
            "bg1" => self.bg1,
            "bg2" => self.bg2,
            "bgHi" => self.bg_hi,
            "bgAc" => self.bg_ac,
            "fg1" => self.fg1,
            "fg2" => self.fg2,
            "fgAc" => self.fg_ac,
            "fgMu" => self.fg_mu,
            "ok" => self.ok,
            "warn" => self.warn,
            "err" => self.err,
            "info" => self.info,
            "border" => self.border,
            _ => None,
        }
    }

    /// `resolveSemantic` (`tokens.ts:39-44`): a semantic name maps through
    /// its slot, a triple passes through, `None` stays `None`.
    pub fn resolve_rgb(&self, color: Option<Color>) -> Option<Rgb> {
        match color? {
            Color::Rgb(r, g, b) => Some((r, g, b)),
            c => self.slot(c.slot()?),
        }
    }

    /// Resolve to a ratatui colour; `None` becomes [`ratatui::style::Color::Reset`].
    pub fn resolve(&self, color: Option<Color>) -> ratatui::style::Color {
        to_ratatui(self.resolve_rgb(color))
    }

    /// Resolve a semantic colour (never `None` input).
    pub fn color(&self, color: Color) -> ratatui::style::Color {
        self.resolve(Some(color))
    }

    /// `themeTokens` (`tokens.ts:55-61`): every semantic name with its RGB.
    pub fn tokens(&self) -> Vec<(&'static str, Option<Rgb>)> {
        SEMANTIC_COLORS
            .iter()
            .map(|c| (c.name().unwrap(), self.resolve_rgb(Some(*c))))
            .collect()
    }

    /// The 16-colour ANSI palette for an embedded terminal
    /// (`themeToXterm`, `builders.ts:35-61`): black..white from the theme
    /// slots, the bright variants brightened by 40, each with Node's fallback
    /// when the slot is unset.
    pub fn to_palette(&self) -> [Rgb; 16] {
        let or = |c: Option<Rgb>, hex: u32| c.unwrap_or(hex_rgb(hex));
        let bright = |c: Option<Rgb>, hex: u32| c.map(brighten).unwrap_or(hex_rgb(hex));
        [
            or(self.bg1, 0x0f111a),
            or(self.err, 0xf05050),
            or(self.ok, 0x50c878),
            or(self.warn, 0xf0b432),
            or(self.info, 0x50aaf0),
            or(self.fg_ac, 0x64a0ff),
            or(self.fg2, 0x8c9bb9),
            or(self.fg1, 0xd2dae8),
            or(self.fg_mu, 0x465069),
            bright(self.err, 0xff6666),
            bright(self.ok, 0x78e0a0),
            bright(self.warn, 0xffcc5a),
            bright(self.info, 0x78c8ff),
            bright(self.fg_ac, 0x8cc0ff),
            bright(self.fg2, 0xb4c3e1),
            bright(self.fg1, 0xffffff),
        ]
    }

    /// The default foreground/background/cursor for an embedded terminal
    /// (`themeToXterm` `foreground`/`background`/`cursor`).
    pub fn terminal_defaults(&self) -> (Rgb, Rgb, Rgb) {
        (
            self.fg1.unwrap_or(hex_rgb(0xd2dae8)),
            self.bg1.unwrap_or(hex_rgb(0x0f111a)),
            self.fg_ac.unwrap_or(hex_rgb(0x64a0ff)),
        )
    }

    /// Is every slot unset (the `terminal` theme)?
    pub fn is_terminal(&self) -> bool {
        SLOT_NAMES.iter().all(|n| self.slot(n).is_none())
    }
}

fn hex_rgb(hex: u32) -> Rgb {
    ((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// `brighten(c, 40)` (`builders.ts:22-24`).
pub fn brighten((r, g, b): Rgb) -> Rgb {
    (
        r.saturating_add(40),
        g.saturating_add(40),
        b.saturating_add(40),
    )
}

/// An RGB triple as a ratatui colour, `None` as `Reset`.
pub fn to_ratatui(c: Option<Rgb>) -> ratatui::style::Color {
    match c {
        Some((r, g, b)) => ratatui::style::Color::Rgb(r, g, b),
        None => ratatui::style::Color::Reset,
    }
}

const fn t(
    bg1: Rgb,
    bg2: Rgb,
    bg_hi: Rgb,
    bg_ac: Rgb,
    fg1: Rgb,
    fg2: Rgb,
    fg_ac: Rgb,
    fg_mu: Rgb,
    ok: Rgb,
    warn: Rgb,
    err: Rgb,
    info: Rgb,
    border: Rgb,
) -> Theme {
    Theme {
        bg1: Some(bg1),
        bg2: Some(bg2),
        bg_hi: Some(bg_hi),
        bg_ac: Some(bg_ac),
        fg1: Some(fg1),
        fg2: Some(fg2),
        fg_ac: Some(fg_ac),
        fg_mu: Some(fg_mu),
        ok: Some(ok),
        warn: Some(warn),
        err: Some(err),
        info: Some(info),
        border: Some(border),
    }
}

/// `themes.coolBlue` (`colors.ts:290-295`), the `app()` default.
pub const COOL_BLUE: Theme = t(
    (15, 17, 26),
    (22, 27, 42),
    (30, 40, 65),
    (40, 80, 140),
    (210, 218, 235),
    (140, 155, 185),
    (100, 160, 255),
    (70, 80, 105),
    (80, 200, 120),
    (240, 180, 50),
    (240, 80, 80),
    (80, 170, 240),
    (50, 60, 85),
);
/// `themes.warmAmber`.
pub const WARM_AMBER: Theme = t(
    (24, 18, 12),
    (36, 28, 18),
    (55, 42, 25),
    (120, 80, 30),
    (235, 220, 195),
    (180, 160, 130),
    (255, 190, 60),
    (100, 85, 60),
    (120, 200, 80),
    (255, 200, 80),
    (220, 80, 60),
    (100, 180, 220),
    (80, 65, 40),
);
/// `themes.mono`.
pub const MONO: Theme = t(
    (18, 18, 18),
    (28, 28, 28),
    (48, 48, 48),
    (70, 70, 70),
    (220, 220, 220),
    (160, 160, 160),
    (255, 255, 255),
    (90, 90, 90),
    (160, 220, 160),
    (220, 200, 130),
    (220, 140, 140),
    (140, 180, 220),
    (60, 60, 60),
);
/// `themes.dracula`.
pub const DRACULA: Theme = t(
    (40, 42, 54),
    (50, 52, 68),
    (68, 71, 90),
    (98, 114, 164),
    (248, 248, 242),
    (189, 147, 249),
    (139, 233, 253),
    (98, 114, 164),
    (80, 250, 123),
    (241, 250, 140),
    (255, 85, 85),
    (139, 233, 253),
    (80, 83, 105),
);
/// `themes.forest`.
pub const FOREST: Theme = t(
    (12, 20, 14),
    (18, 32, 22),
    (28, 48, 32),
    (40, 80, 50),
    (200, 225, 205),
    (140, 175, 150),
    (100, 220, 130),
    (65, 90, 70),
    (80, 230, 120),
    (230, 200, 80),
    (230, 90, 80),
    (80, 190, 220),
    (45, 65, 48),
);
/// `themes.coolBlueLight`.
pub const COOL_BLUE_LIGHT: Theme = t(
    (240, 244, 250),
    (230, 236, 245),
    (210, 220, 238),
    (70, 120, 200),
    (30, 35, 50),
    (80, 90, 115),
    (40, 100, 220),
    (140, 150, 175),
    (30, 140, 70),
    (180, 120, 0),
    (200, 40, 40),
    (30, 120, 200),
    (180, 190, 210),
);
/// `themes.warmAmberLight`.
pub const WARM_AMBER_LIGHT: Theme = t(
    (252, 245, 235),
    (242, 232, 218),
    (230, 215, 195),
    (180, 130, 50),
    (50, 40, 30),
    (100, 80, 55),
    (190, 120, 10),
    (150, 135, 110),
    (40, 150, 40),
    (200, 140, 0),
    (190, 50, 30),
    (50, 130, 180),
    (200, 185, 160),
);
/// `themes.monoLight`.
pub const MONO_LIGHT: Theme = t(
    (245, 245, 245),
    (235, 235, 235),
    (215, 215, 215),
    (180, 180, 180),
    (30, 30, 30),
    (80, 80, 80),
    (0, 0, 0),
    (150, 150, 150),
    (40, 140, 40),
    (170, 140, 20),
    (180, 50, 50),
    (40, 120, 180),
    (190, 190, 190),
);
/// `themes.draculaLight`.
pub const DRACULA_LIGHT: Theme = t(
    (248, 248, 242),
    (238, 236, 230),
    (220, 218, 210),
    (150, 140, 200),
    (40, 42, 54),
    (100, 70, 180),
    (50, 160, 180),
    (140, 150, 170),
    (30, 170, 70),
    (180, 170, 40),
    (210, 50, 50),
    (50, 160, 180),
    (200, 198, 190),
);
/// `themes.forestLight`.
pub const FOREST_LIGHT: Theme = t(
    (242, 250, 244),
    (230, 242, 232),
    (210, 230, 215),
    (70, 140, 85),
    (25, 45, 30),
    (60, 100, 70),
    (40, 160, 70),
    (130, 155, 135),
    (30, 160, 60),
    (170, 140, 20),
    (190, 50, 40),
    (40, 130, 170),
    (175, 200, 180),
);
/// `themes.terminal` (`colors.ts:351-356`): every slot unset, so every
/// colour is the terminal's default.
pub const TERMINAL: Theme = Theme {
    bg1: None,
    bg2: None,
    bg_hi: None,
    bg_ac: None,
    fg1: None,
    fg2: None,
    fg_ac: None,
    fg_mu: None,
    ok: None,
    warn: None,
    err: None,
    info: None,
    border: None,
};

/// The built-in themes by name, in `colors.ts` declaration order (the order
/// `ctrl+g` cycles them in the session manager).
pub const THEMES: [(&str, Theme); 11] = [
    ("coolBlue", COOL_BLUE),
    ("warmAmber", WARM_AMBER),
    ("mono", MONO),
    ("dracula", DRACULA),
    ("forest", FOREST),
    ("coolBlueLight", COOL_BLUE_LIGHT),
    ("warmAmberLight", WARM_AMBER_LIGHT),
    ("monoLight", MONO_LIGHT),
    ("draculaLight", DRACULA_LIGHT),
    ("forestLight", FOREST_LIGHT),
    ("terminal", TERMINAL),
];

/// A built-in theme by name.
pub fn theme_by_name(name: &str) -> Option<Theme> {
    THEMES.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// The names of the built-in themes in cycle order.
pub fn theme_names() -> Vec<&'static str> {
    THEMES.iter().map(|(n, _)| *n).collect()
}

/// The four box styles (`colors.ts:213`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxStyle {
    #[default]
    Rounded,
    Sharp,
    Double,
    Heavy,
}

impl BoxStyle {
    /// The ratatui border set for this style.
    pub fn border_set(&self) -> border::Set<'static> {
        match self {
            BoxStyle::Rounded => border::ROUNDED,
            BoxStyle::Sharp => border::PLAIN,
            BoxStyle::Double => border::DOUBLE,
            BoxStyle::Heavy => border::THICK,
        }
    }

    /// The left/right junction glyphs a separator uses to join the sides
    /// (`lj`/`rj`, `colors.ts:220-225`).
    pub fn junctions(&self) -> (&'static str, &'static str) {
        match self {
            BoxStyle::Rounded | BoxStyle::Sharp => ("\u{251c}", "\u{2524}"),
            BoxStyle::Double => ("\u{2560}", "\u{2563}"),
            BoxStyle::Heavy => ("\u{2523}", "\u{252b}"),
        }
    }

    /// The horizontal rule glyph.
    pub fn horizontal(&self) -> &'static str {
        match self {
            BoxStyle::Rounded | BoxStyle::Sharp => "\u{2500}",
            BoxStyle::Double => "\u{2550}",
            BoxStyle::Heavy => "\u{2501}",
        }
    }

    /// Parse `rounded | sharp | double | heavy`.
    pub fn parse(s: &str) -> Option<BoxStyle> {
        Some(match s {
            "rounded" => BoxStyle::Rounded,
            "sharp" => BoxStyle::Sharp,
            "double" => BoxStyle::Double,
            "heavy" => BoxStyle::Heavy,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/tokens.test.ts:11-16
    #[test]
    fn maps_each_semantic_name_to_its_slot() {
        for c in SEMANTIC_COLORS {
            assert_eq!(COOL_BLUE.resolve_rgb(Some(c)), COOL_BLUE.slot(c.slot().unwrap()));
        }
    }

    /// node: tests/tokens.test.ts:18-20
    #[test]
    fn passes_triples_through() {
        assert_eq!(COOL_BLUE.resolve_rgb(Some(Color::Rgb(12, 34, 56))), Some((12, 34, 56)));
    }

    /// node: tests/tokens.test.ts:22-25
    #[test]
    fn resolves_none_and_unknown_to_default() {
        assert_eq!(COOL_BLUE.resolve_rgb(None), None);
        assert_eq!(Color::parse("nope"), None);
        assert_eq!(COOL_BLUE.resolve(None), ratatui::style::Color::Reset);
    }

    /// node: tests/tokens.test.ts:27-31
    #[test]
    fn covers_exactly_nine_semantic_colors() {
        let mut names: Vec<&str> = SEMANTIC_NAMES.to_vec();
        names.sort();
        assert_eq!(
            names,
            ["accent", "border", "error", "info", "muted", "ok", "primary", "secondary", "warn"]
        );
    }

    /// node: tests/tokens.test.ts:43-49
    #[test]
    fn honors_the_historical_slot_mapping() {
        let t = COOL_BLUE;
        assert_eq!(t.resolve_rgb(Some(Color::Accent)), t.fg_ac);
        assert_eq!(t.resolve_rgb(Some(Color::Error)), t.err);
        assert_eq!(t.resolve_rgb(Some(Color::Muted)), t.fg_mu);
        assert_eq!(t.resolve_rgb(Some(Color::Primary)), t.fg1);
    }

    /// node: tests/tokens.test.ts:53-59
    #[test]
    fn tokens_emit_a_name_to_rgb_map() {
        let tok = COOL_BLUE.tokens();
        let mut names: Vec<&str> = tok.iter().map(|(n, _)| *n).collect();
        names.sort();
        let mut expected = SEMANTIC_NAMES.to_vec();
        expected.sort();
        assert_eq!(names, expected);
        let get = |n: &str| tok.iter().find(|(k, _)| *k == n).unwrap().1;
        assert_eq!(get("accent"), COOL_BLUE.fg_ac);
        assert_eq!(get("border"), COOL_BLUE.border);
    }

    /// node: tests/tokens.test.ts:61-64
    #[test]
    fn terminal_theme_serializes_to_all_none() {
        assert!(TERMINAL.tokens().iter().all(|(_, v)| v.is_none()));
        assert!(TERMINAL.is_terminal());
        assert!(!COOL_BLUE.is_terminal());
    }

    /// node: src/tui/builders.ts:35-61
    #[test]
    fn palette_follows_theme_to_xterm() {
        let p = COOL_BLUE.to_palette();
        assert_eq!(p[0], (15, 17, 26)); // black = bg1
        assert_eq!(p[1], (240, 80, 80)); // red = err
        assert_eq!(p[9], (255, 120, 120)); // brightRed = err + 40
        assert_eq!(p[15], (250, 255, 255)); // brightWhite = fg1 + 40 (clamped)
        let term = TERMINAL.to_palette();
        assert_eq!(term[0], (0x0f, 0x11, 0x1a));
        assert_eq!(term[9], (0xff, 0x66, 0x66));
        assert_eq!(term[15], (0xff, 0xff, 0xff));
    }

    #[test]
    fn eleven_themes_in_cycle_order() {
        assert_eq!(THEMES.len(), 11);
        assert_eq!(theme_names()[0], "coolBlue");
        assert_eq!(theme_names()[10], "terminal");
        assert_eq!(theme_by_name("dracula"), Some(DRACULA));
        assert_eq!(theme_by_name("nope"), None);
    }
}
