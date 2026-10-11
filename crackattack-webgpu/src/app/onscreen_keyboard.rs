//! Pure model of the on-screen keyboard: layout, navigation and key application.
//!
//! No egui dependency; rendering lives in `renderer/ui/onscreen_keyboard.rs`.
//!
//! The keyboard is deliberately small: ASCII only (uppercase letters, digits,
//! punctuation) and four top-level actions (Space, Backspace, Clear, Done). There is no
//! Shift/Alt and no IME. It edits a `String` directly and injects no egui events. The
//! layout depends on the field (`CharSet`): alphanumeric name, IPv4 pad (digits and dot),
//! IPv6 pad (hex digits and colon), port pad (digits). All layouts share the same
//! navigation and editing code.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KbKey {
    /// Inserts this character.
    Char(char),
    /// Deletes the last character.
    Backspace,
    /// Empties the field.
    Clear,
    /// Ends the input and closes the keyboard.
    Done,
}

/// Returns the key layout for a field type.
///
/// The renderer draws every row in equal-width columns, so rows of different lengths
/// stay aligned.
pub fn layout(charset: CharSet) -> Vec<Vec<KbKey>> {
    use KbKey::*;
    let row = |s: &str| s.chars().map(Char).collect::<Vec<_>>();
    match charset {
        // Name: alphabetical order, the most predictable with a gamepad.
        CharSet::PlayerName => vec![
            row("1234567890"),
            row("ABCDEFGHIJ"),
            row("KLMNOPQRST"),
            // No apostrophe: the nameplate bitmap font has no glyph for it and the
            // character would vanish silently ("O'BRIEN" -> "OBRIEN"). The charset
            // must match the glyphs the font can draw.
            row("UVWXYZ.-"),
            vec![Char(' '), Backspace, Clear, Done],
        ],
        // IPv4: numeric pad and dot.
        CharSet::IpAddress => vec![
            row("12345"),
            row("67890"),
            vec![Char('.'), Backspace, Clear, Done],
        ],
        // IPv6: digits, lowercase hex letters and colon.
        CharSet::Ipv6 => vec![
            row("0123456789"),
            row("abcdef") .into_iter().chain(std::iter::once(Char(':'))).collect(),
            vec![Backspace, Clear, Done],
        ],
        // Port: numeric pad only.
        CharSet::Port => vec![
            row("12345"),
            row("67890"),
            vec![Backspace, Clear, Done],
        ],
    }
}

/// Returns the key at the focus position (row, column), if any.
pub fn key_at(rows: &[Vec<KbKey>], focus: (usize, usize)) -> Option<KbKey> {
    rows.get(focus.0).and_then(|r| r.get(focus.1)).copied()
}

/// Returns the horizontal position (fraction 0-1) of the center of key `col` in a row of
/// `len` keys. All rows have the same width, so this fraction is the on-screen position.
pub fn key_abscissa(col: usize, len: usize) -> f32 {
    if len == 0 {
        return 0.5;
    }
    (col as f32 + 0.5) / len as f32
}

/// Returns the column of a row of `len` keys whose cell contains `goal_x`.
///
/// Vertical navigation uses it to keep the horizontal position.
pub fn col_at_abscissa(goal_x: f32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    ((goal_x * len as f32).floor() as usize).min(len - 1)
}

/// Result of applying a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KbOutcome {
    /// The text was edited and the keyboard stays open.
    Edited,
    /// The keyboard should close (Done key).
    Close,
}

/// Atomic edit operation on a name: the single source of truth for text changes.
///
/// Both inputs converge here: physical typing (`egui::Event::Text`) and the virtual
/// keys of the on-screen keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameEditOp {
    /// Inserts a character (filtered).
    Insert(char),
    /// Deletes the last character.
    Backspace,
    /// Empties the field.
    Clear,
}

/// Allowed characters for each field type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharSet {
    /// Player name: alphanumeric plus space, `-` and `.`.
    PlayerName,
    /// IPv4 address: digits and dots only.
    IpAddress,
    /// IPv6 address: hex digits and colons (lowercase input).
    Ipv6,
    /// Port: digits only.
    Port,
}

impl CharSet {
    /// Normalizes a name that comes from outside the UI into the displayable charset.
    ///
    /// Applies to saved state, `--set config:player.name` and names received from a peer.
    /// Without it, a name containing `'` or `_` would lose characters silently on the
    /// bitmap nameplate. `_` becomes `-`, characters outside the charset are dropped, the
    /// ends are trimmed and the result is cut to 18 characters. Accented Latin letters are
    /// kept: the nameplate folds them with `strip_diacritic` and egui shows them as is. Other
    /// scripts (Cyrillic, CJK...) are dropped, because the nameplate font cannot draw them.
    pub fn sanitize_player_name(input: &str) -> String {
        let filtered: String = input
            .chars()
            .filter_map(|c| {
                if c == '_' {
                    Some('-')
                } else if CharSet::PlayerName.permits(c) || ('\u{00C0}'..='\u{024F}').contains(&c) {
                    Some(c)
                } else {
                    None
                }
            })
            .collect();
        filtered.trim().chars().take(18).collect()
    }

    /// Returns whether the character is allowed in this field type.
    pub fn permits(self, c: char) -> bool {
        match self {
            // No `_` or `'`: the bitmap font has no glyph for them. The charset must
            // match the glyphs the nameplate can draw.
            CharSet::PlayerName => c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '.'),
            CharSet::IpAddress => c.is_ascii_digit() || c == '.',
            CharSet::Ipv6 => c.is_ascii_hexdigit() || c == ':',
            CharSet::Port => c.is_ascii_digit(),
        }
    }
}

/// Format of an input field: max length, allowed characters, first-letter capitalization
/// and default value when empty.
///
/// It groups the constraints of a field so that one editor serves every field.
#[derive(Debug, Clone, Copy)]
pub struct InputFormat {
    pub max_len: usize,
    pub charset: CharSet,
    pub capitalize_first: bool,
    pub default_value: &'static str,
}

impl InputFormat {
    /// Player name: 18 characters, alphanumeric plus punctuation, capital first letter.
    ///
    /// The default value is localized ("Player 1" / "Joueur 1").
    // Not `const`: the default is read from the locale at call time.
    pub fn player_name() -> Self {
        Self { max_len: 18, charset: CharSet::PlayerName, capitalize_first: true, default_value: crate::app::locale::t().default_player_name }
    }
    /// IPv4 address: 15 characters (xxx.xxx.xxx.xxx), digits and dots, no capital, empty default.
    pub const fn ip_address() -> Self {
        Self { max_len: 15, charset: CharSet::IpAddress, capitalize_first: false, default_value: "" }
    }
    /// IPv6 address: 39 characters max, hex digits and colons, no capital, empty default.
    pub const fn ipv6() -> Self {
        Self { max_len: 39, charset: CharSet::Ipv6, capitalize_first: false, default_value: "" }
    }
    /// Port: 5 digits max, empty default (the default port is used when connecting).
    pub const fn port() -> Self {
        Self { max_len: 5, charset: CharSet::Port, capitalize_first: false, default_value: "" }
    }
    /// Normalizes a validated value: trims it and falls back to the default if empty.
    ///
    /// For `capitalize_first` fields, an all-uppercase input becomes "First letter
    /// capital, rest lowercase" (caps-lock guard). This runs when the keyboard closes, so
    /// the case fix applies at validation, not only when the screen is left.
    pub fn finalize(&self, text: &str) -> String {
        let t = text.trim();
        let base = if t.is_empty() { self.default_value } else { t };
        let mut s = base.to_string();
        if self.capitalize_first
            && s.chars().any(|c| c.is_ascii_alphabetic())
            && s == s.to_ascii_uppercase()
        {
            s = s.to_ascii_lowercase();
            if let Some(first) = s.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
        }
        s
    }
}

/// Applies an edit operation according to `format`.
///
/// It rejects forbidden characters, caps the length at `max_len` and optionally
/// capitalizes the first letter. Returns `true` if the text changed. Physical typing and
/// on-screen keys both go through it.
pub fn apply_edit(text: &mut String, op: NameEditOp, format: InputFormat) -> bool {
    match op {
        NameEditOp::Insert(c) => {
            if !format.charset.permits(c) || text.chars().count() >= format.max_len {
                return false;
            }
            text.push(c);
            if format.capitalize_first {
                if let Some(first) = text.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
            }
            true
        }
        NameEditOp::Backspace => text.pop().is_some(),
        NameEditOp::Clear => {
            let had = !text.is_empty();
            text.clear();
            had
        }
    }
}

/// Applies a virtual key to the text according to `format`.
///
/// Delegates to `apply_edit`, which physical typing also uses.
pub fn apply(text: &mut String, key: KbKey, format: InputFormat) -> KbOutcome {
    match key {
        KbKey::Char(c) => {
            apply_edit(text, NameEditOp::Insert(c), format);
            KbOutcome::Edited
        }
        KbKey::Backspace => {
            apply_edit(text, NameEditOp::Backspace, format);
            KbOutcome::Edited
        }
        KbKey::Clear => {
            apply_edit(text, NameEditOp::Clear, format);
            KbOutcome::Edited
        }
        KbKey::Done => KbOutcome::Close,
    }
}

// --- Free navigation (focus plus target abscissa), shared by the name editor and the
// network editor. Vertical moves are re-projected on the target abscissa, so the column
// does not drift. Horizontal moves wrap around. `charset` selects the layout. ---

/// Moves the focus to (row, col) and stores the matching target abscissa.
pub fn osk_focus_to(focus: &mut (usize, usize), goal_x: &mut f32, charset: CharSet, row: usize, col: usize) {
    let rows = layout(charset);
    *focus = (row, col);
    let len = rows.get(row).map(|r| r.len()).unwrap_or(1);
    *goal_x = key_abscissa(col, len);
}

/// Moves the focus.
///
/// Vertical (`dy`): wraps top to bottom and re-projects the target abscissa on the new
/// row. Horizontal (`dx`): wraps within the row and updates the abscissa.
pub fn osk_move(focus: &mut (usize, usize), goal_x: &mut f32, charset: CharSet, dx: i32, dy: i32) {
    let rows = layout(charset);
    if rows.is_empty() {
        return;
    }
    let (row, col) = *focus;
    if dy != 0 {
        let nrow = crate::app::focus::ring_move(row, rows.len(), dy);
        let ncol = col_at_abscissa(*goal_x, rows[nrow].len());
        *focus = (nrow, ncol); // goal_x is kept (sticky)
    } else if dx != 0 {
        let len = rows.get(row).map(|r| r.len()).unwrap_or(1).max(1);
        // Wraps at both ends. This is an intentional exception: other screens stop at
        // the edge horizontally.
        let ncol = crate::app::focus::ring_move(col, len, dx);
        osk_focus_to(focus, goal_x, charset, row, ncol);
    }
}

/// Returns the position of the Done key in the layout of `charset`.
///
/// Every layout has one (see the tests). The fallback `(0, 0)` is only a safeguard.
pub fn done_focus(charset: CharSet) -> (usize, usize) {
    let rows = layout(charset);
    for (r, row) in rows.iter().enumerate() {
        if let Some(c) = row.iter().position(|k| matches!(k, KbKey::Done)) {
            return (r, c);
        }
    }
    (0, 0)
}

/// Activates the focused key according to `format` and returns the outcome (`Close` for Done).
pub fn osk_activate(text: &mut String, focus: (usize, usize), format: InputFormat) -> KbOutcome {
    let rows = layout(format.charset);
    match key_at(&rows, focus) {
        Some(key) => apply(text, key, format),
        None => KbOutcome::Edited,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abscissa_helpers() {
        // Center of the key.
        assert!((key_abscissa(0, 10) - 0.05).abs() < 1e-6);
        assert!((key_abscissa(9, 10) - 0.95).abs() < 1e-6);
        // Re-projection: abscissa in a row of 10 maps to a cell in a row of 4.
        assert_eq!(col_at_abscissa(key_abscissa(7, 10), 4), 3); // ~0.75 -> cell 3
        assert_eq!(col_at_abscissa(key_abscissa(0, 10), 4), 0);
        assert_eq!(col_at_abscissa(0.99, 4), 3); // clamped
        assert_eq!(col_at_abscissa(0.0, 10), 0);
    }

    #[test]
    fn apply_edits_and_bounds() {
        let fmt = InputFormat { max_len: 3, charset: CharSet::PlayerName, capitalize_first: true, default_value: "" };
        let mut s = String::new();
        assert_eq!(apply(&mut s, KbKey::Char('A'), fmt), KbOutcome::Edited);
        apply(&mut s, KbKey::Char('B'), fmt);
        apply(&mut s, KbKey::Char('C'), fmt);
        apply(&mut s, KbKey::Char('D'), fmt); // capped at 3
        assert_eq!(s, "ABC");
        apply(&mut s, KbKey::Backspace, fmt);
        assert_eq!(s, "AB");
        apply(&mut s, KbKey::Clear, fmt);
        assert_eq!(s, "");
        assert_eq!(apply(&mut s, KbKey::Done, fmt), KbOutcome::Close);
    }

    #[test]
    fn name_format_filters_capitalizes_and_bounds() {
        let fmt = InputFormat::player_name();
        let mut s = String::new();
        // The first letter is capitalized; lowercase input is accepted afterwards.
        assert!(apply_edit(&mut s, NameEditOp::Insert('t'), fmt));
        assert_eq!(s, "T");
        apply_edit(&mut s, NameEditOp::Insert('h'), fmt);
        assert_eq!(s, "Th");
        // Forbidden character: ignored, no change.
        assert!(!apply_edit(&mut s, NameEditOp::Insert('@'), fmt));
        assert_eq!(s, "Th");
    }

    // Normalization of names from outside the UI (disk, CLI, network).
    #[test]
    fn sanitize_player_name_boundaries() {
        assert_eq!(CharSet::sanitize_player_name("O'BRIEN"), "OBRIEN");
        assert_eq!(CharSet::sanitize_player_name("A_B"), "A-B"); // '_' becomes '-'
        assert_eq!(CharSet::sanitize_player_name("  Zoé  "), "Zoé"); // accents kept (folded for the bitmap)
        assert_eq!(CharSet::sanitize_player_name("Игрок Max"), "Max"); // the nameplate cannot draw Cyrillic
        assert_eq!(CharSet::sanitize_player_name("x".repeat(30).as_str()).chars().count(), 18);
        assert_eq!(CharSet::sanitize_player_name("<script>"), "script");
    }

    #[test]
    fn ip_format_allows_only_digits_and_dots() {
        let fmt = InputFormat::ip_address();
        let mut s = String::new();
        for c in "12a.3b4".chars() {
            apply_edit(&mut s, NameEditOp::Insert(c), fmt);
        }
        assert_eq!(s, "12.34"); // letters rejected, digits and dot kept
        // An IP address is never capitalized.
        assert!(!fmt.capitalize_first);
    }

    #[test]
    fn ipv6_format_allows_hex_and_colon_only() {
        let fmt = InputFormat::ipv6();
        let mut s = String::new();
        for c in "2001:db8::g1Z".chars() {
            apply_edit(&mut s, NameEditOp::Insert(c), fmt);
        }
        // 'g' and 'Z' are not hex digits and are rejected. Hex digits (uppercase allowed) and ':' are kept.
        assert_eq!(s, "2001:db8::1");
    }

    #[test]
    fn port_format_allows_digits_only_and_bounds() {
        let fmt = InputFormat::port();
        let mut s = String::new();
        for c in "45a678".chars() {
            apply_edit(&mut s, NameEditOp::Insert(c), fmt);
        }
        assert_eq!(s, "45678"); // letter rejected, 5 digits (max)
        assert!(!apply_edit(&mut s, NameEditOp::Insert('9'), fmt)); // capped at 5
    }

    #[test]
    fn layouts_are_non_empty_per_charset() {
        for cs in [CharSet::PlayerName, CharSet::IpAddress, CharSet::Ipv6, CharSet::Port] {
            let rows = layout(cs);
            assert!(!rows.is_empty() && rows.iter().all(|r| !r.is_empty()));
            // Every layout has a Done key.
            assert!(rows.iter().flatten().any(|k| matches!(k, KbKey::Done)));
        }
    }

    #[test]
    fn finalize_trims_and_defaults_when_empty() {
        let name = InputFormat::player_name();
        assert_eq!(name.finalize("  Bob  "), "Bob"); // trimmed, case already correct
        // Automatic case fix (caps-lock guard) applies at validation.
        assert_eq!(name.finalize("BOB"), "Bob");
        assert_eq!(name.finalize("HELLO WORLD"), "Hello world");
        // Empty -> default.
        assert_eq!(name.finalize("   "), "Player 1");
        assert_eq!(name.finalize(""), "Player 1");
        let ip = InputFormat::ip_address();
        assert_eq!(ip.finalize("  "), ""); // the IP default is empty
    }
}
