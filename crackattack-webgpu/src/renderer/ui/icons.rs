//! Lucide icons (<https://lucide.dev>) rendered as an egui font.
//!
//! ISC license, see `assets/licenses/LUCIDE-LICENSE.txt`. The icon font is embedded with
//! `include_bytes!`, so it needs no file on disk at runtime. It is installed in egui under a
//! font family named `"lucide"`, so an icon is drawn as text: it can be recolored and scaled.
//! Codepoints are in the Unicode private use area (see `codepoints.json` in the `lucide-font`
//! package).

/// Embedded Lucide font.
const LUCIDE_TTF: &[u8] = include_bytes!("../../../assets/fonts/lucide.ttf");

/// Embedded Noto Sans Symbols (OFL), a fallback for Unicode symbols missing from the default
/// egui font. Version 1 covers the Arrows block (↑↓←→, U+2190…) that version 2 lacks.
/// License: `assets/licenses/NOTOSANSSYMBOLS-OFL.txt`.
const NOTO_SYMBOLS_TTF: &[u8] = include_bytes!("../../../assets/fonts/NotoSansSymbols-Regular.ttf");
/// Embedded Noto Sans Symbols 2 (OFL), a second fallback (versions 1 and 2 cover different
/// blocks). License: `assets/licenses/NOTOSANSSYMBOLS2-OFL.txt`.
const NOTO_SYMBOLS2_TTF: &[u8] = include_bytes!("../../../assets/fonts/NotoSansSymbols2-Regular.ttf");
/// Embedded CJK fonts (OFL), each reduced to the characters of its own language by
/// `tools/fonts/subset_cjk.py` (tens of KiB instead of 10 to 18 MB). They come last in the
/// fallback chain, so they only draw what the other fonts lack. Chinese and Japanese share
/// ideographs whose shapes differ, so the font of the active language goes first (see
/// `font_definitions`). Licenses: `assets/licenses/NOTOSANS{SC,JP,KR}-OFL.txt`.
const NOTO_SC_TTF: &[u8] = include_bytes!("../../../assets/fonts/NotoSansSC-subset.ttf");
const NOTO_JP_TTF: &[u8] = include_bytes!("../../../assets/fonts/NotoSansJP-subset.ttf");
const NOTO_KR_TTF: &[u8] = include_bytes!("../../../assets/fonts/NotoSansKR-subset.ttf");

/// Language the installed fonts were ordered for (index into `Language::ALL`, `u8::MAX` = none).
static FONTS_LANGUAGE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(u8::MAX);

/// Name of the egui font family used for icons.
pub const FAMILY: &str = "lucide";

// --- Codepoints of the icons in use ---
/// `power`: quit the game.
pub const POWER: char = '\u{E140}';
/// `arrow-left`: back to the parent menu.
pub const BACK: char = '\u{E048}';
/// `rotate-ccw`: reset.
pub const ROTATE_CCW: char = '\u{E148}';
/// `copy`: copy to the clipboard.
pub const COPY: char = '\u{E09E}';
/// `clipboard-paste`: paste from the clipboard.
pub const PASTE: char = '\u{E3E8}';
/// `chevrons-left` (double chevron): back to the menu from the post-game screens.
pub const CHEVRONS_LEFT: char = '\u{E072}';
/// `arrow-right`: next (post-game navigation).
pub const ARROW_RIGHT: char = '\u{E049}';
/// `settings` (gear): options, in the menu icon bar.
pub const SETTINGS: char = '\u{E154}';
/// `info`: the About / Info screen, in the menu icon bar.
pub const INFO: char = '\u{E0F9}';
/// `trophy`: wins (network ladder summaries).
pub const TROPHY: char = '\u{E373}';
/// `skull`: losses (network ladder summaries).
pub const SKULL: char = '\u{E221}';
/// `wifi`: quick reconnect to an opponent (Opponents page).
pub const WIFI: char = '\u{E1AE}';
/// `ban`: a LAN peer that cannot be played (another network version).
pub const BAN: char = '\u{E051}';
/// `refresh-cw`: check for updates / try again (Updates page).
pub const REFRESH: char = '\u{E145}';
/// `download`: download the new version (Updates page).
pub const DOWNLOAD: char = '\u{E0B2}';
/// `check-circle`: up to date (Updates page).
pub const CHECK_CIRCLE: char = '\u{E07C}';
/// `wifi-off`: the update check could not reach the server (Updates page).
pub const WIFI_OFF: char = '\u{E1AF}';
/// `store`: updates are handled by the app store (Updates page).
pub const STORE: char = '\u{E3E4}';
/// `medal`: record beaten (stats screens). One icon, one meaning: [`TROPHY`] is for
/// victories, the medal is for records.
pub const MEDAL: char = '\u{E36F}';

/// Installs the Lucide font in the egui context (family `"lucide"`) on top of the default
/// fonts, so normal text is unchanged. Call once at startup.
pub fn install(ctx: &egui::Context) {
    sync_fonts(ctx);
}

/// Reinstalls the fonts when the language has changed since the last call, so the CJK font of
/// the active language comes first. Call before each egui pass; otherwise a comparison only.
pub fn sync_fonts(ctx: &egui::Context) {
    use crate::app::locale::{current_language, Language};
    use std::sync::atomic::Ordering;
    let lang = current_language();
    let index = Language::ALL.iter().position(|&l| l == lang).unwrap_or(0) as u8;
    if FONTS_LANGUAGE.swap(index, Ordering::Relaxed) != index {
        ctx.set_fonts(font_definitions(lang));
    }
}

/// The game fonts for `lang`: egui defaults, Lucide, then the Noto fallbacks (symbols, then the
/// CJK fonts, the one of `lang` first).
pub(crate) fn font_definitions(lang: crate::app::locale::Language) -> egui::FontDefinitions {
    use crate::app::locale::Language;
    let mut fonts = egui::FontDefinitions::default();

    // Lucide: a named family "lucide", used explicitly through `icon()`.
    fonts.font_data.insert(
        FAMILY.to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(LUCIDE_TTF)),
    );
    fonts.families.insert(
        egui::FontFamily::Name(FAMILY.into()),
        vec![FAMILY.to_owned()],
    );

    // Noto Sans Symbols (v1 + v2): fallbacks at the end of the Proportional AND Monospace
    // families. egui switches to them glyph by glyph for any symbol missing from the default
    // font (arrows ↑↓←→…), so rendering code needs no special case. The two fonts cover
    // different Unicode blocks, so they are chained (v1 first, for the Arrows block).
    fonts.font_data.insert(
        "noto_symbols".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(NOTO_SYMBOLS_TTF)),
    );
    fonts.font_data.insert(
        "noto_symbols2".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(NOTO_SYMBOLS2_TTF)),
    );
    for (name, bytes) in [("noto_sc", NOTO_SC_TTF), ("noto_jp", NOTO_JP_TTF), ("noto_kr", NOTO_KR_TTF)] {
        fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(egui::FontData::from_static(bytes)));
    }
    let cjk = match lang {
        Language::Japanese => ["noto_jp", "noto_sc", "noto_kr"],
        Language::Korean => ["noto_kr", "noto_jp", "noto_sc"],
        _ => ["noto_sc", "noto_jp", "noto_kr"],
    };
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let chain = fonts.families.entry(family).or_default();
        chain.push("noto_symbols".to_owned());
        chain.push("noto_symbols2".to_owned());
        chain.extend(cjk.iter().map(|n| n.to_string()));
    }
    fonts
}

/// `RichText` for an icon, drawn with the icon font (family `"lucide"`).
/// Color and size it like text (`.color(...)`, `.size(...)`).
pub fn icon(c: char) -> egui::RichText {
    egui::RichText::new(c).family(egui::FontFamily::Name(FAMILY.into()))
}

/// Icon + text label for a button: a `LayoutJob` with two sections (the glyph in the
/// `"lucide"` font, the text in the proportional font). Pass it to `egui::Button::new`.
pub fn icon_text(glyph: char, text: &str, size: f32, color: egui::Color32) -> egui::text::LayoutJob {
    use egui::{text::LayoutJob, FontFamily, FontId, TextFormat};
    let mut job = LayoutJob::default();
    job.append(
        &glyph.to_string(),
        0.0,
        TextFormat { font_id: FontId::new(size, FontFamily::Name(FAMILY.into())), color, ..Default::default() },
    );
    job.append(
        &format!("  {}", text),
        0.0,
        TextFormat { font_id: FontId::proportional(size), color, ..Default::default() },
    );
    job
}

#[cfg(test)]
mod coverage_tests {
    /// Every character the UI can show in any language (string literals of `locale.rs` and the
    /// text pages) has a glyph in one of the fonts of the Proportional chain. A missing glyph
    /// would be drawn as an empty box. For a Chinese character, rebuild the reduced font.
    #[test]
    fn ui_text_has_a_glyph_in_the_embedded_fonts() {
        use ab_glyph::Font;
        use crate::app::locale::Language;
        let defs = super::font_definitions(Language::English);
        let parse = |name: &str| ab_glyph::FontRef::try_from_slice(&defs.font_data[name].font).expect(name);
        let fonts: Vec<ab_glyph::FontRef> = defs.families[&egui::FontFamily::Proportional]
            .iter().map(|name| parse(name)).collect();
        let root = env!("CARGO_MANIFEST_DIR");
        let locale = std::fs::read_to_string(format!("{root}/src/app/locale.rs")).unwrap();
        let mut texts = vec![("", string_literals(&locale))];
        for entry in std::fs::read_dir(format!("{root}/assets/text")).unwrap() {
            let path = entry.unwrap().path();
            texts.push(("page", std::fs::read_to_string(path).unwrap()));
        }
        let mut missing = std::collections::BTreeSet::new();
        for (_, text) in &texts {
            for c in text.chars().filter(|c| !c.is_control()) {
                if !fonts.iter().any(|f| f.glyph_id(c).0 != 0) {
                    missing.insert(c);
                }
            }
        }
        // Each CJK language must find its characters in ITS OWN font, not in another CJK font
        // that happens to have them (a Japanese ideograph drawn with the Chinese shape).
        for (table, md, font) in [("ZH", "zh", "noto_sc"), ("JA", "ja", "noto_jp"), ("KO", "ko", "noto_kr")] {
            let own = parse(font);
            let start = locale.find(&format!("pub const {table}: Locale = Locale {{")).expect(table);
            let end = start + locale[start..].find("\n};").unwrap();
            let mut text = string_literals(&locale[start..end]);
            for kind in ["rules", "credits"] {
                text += &std::fs::read_to_string(format!("{root}/assets/text/{kind}.{md}.md")).unwrap();
            }
            for c in text.chars().filter(|&c| c as u32 >= 0x2E80) {
                if own.glyph_id(c).0 == 0 {
                    missing.insert(c);
                }
            }
        }
        assert!(missing.is_empty(),
            "no glyph for {:?}; for Chinese, Japanese or Korean characters run \
             `uv run --no-project --with fonttools python tools/fonts/subset_cjk.py`",
            missing);
    }

    /// The CJK font of the active language comes first among the CJK fonts.
    #[test]
    fn cjk_font_of_the_language_comes_first() {
        use crate::app::locale::Language;
        let first_cjk = |lang| {
            let defs = super::font_definitions(lang);
            defs.families[&egui::FontFamily::Proportional].iter()
                .find(|n| n.starts_with("noto_") && n.len() == 7).cloned().unwrap()
        };
        assert_eq!(first_cjk(Language::Chinese), "noto_sc");
        assert_eq!(first_cjk(Language::Japanese), "noto_jp");
        assert_eq!(first_cjk(Language::Korean), "noto_kr");
    }

    /// Contents of the `"..."` literals of a Rust source, outside `//` comments, joined.
    fn string_literals(src: &str) -> String {
        let mut out = String::new();
        for line in src.lines() {
            let mut in_str = false;
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                match (in_str, c) {
                    (false, '/') if chars.peek() == Some(&'/') => break,
                    (false, '"') => in_str = true,
                    (true, '\\') => { chars.next(); }
                    (true, '"') => in_str = false,
                    (true, c) => out.push(c),
                    _ => {}
                }
            }
        }
        out
    }
}
