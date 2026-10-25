//! JavaScript string semantics the TypeScript relies on.
//!
//! Rust's `str::trim` and `char::is_whitespace` use the Unicode `White_Space`
//! property, which differs from ECMAScript: JS treats U+FEFF as whitespace and
//! U+0085 as not. JS also compares strings by UTF-16 code unit and measures
//! `length` in code units. These helpers reproduce the JS behavior exactly.

use std::cmp::Ordering;

/// ECMAScript `WhiteSpace` and `LineTerminator`, the set matched by `\s` and
/// removed by `String.prototype.trim`.
pub(crate) fn is_space(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

/// `s.trim()`
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `!!s.trim()`
pub(crate) fn has_text(s: &str) -> bool {
    !trim(s).is_empty()
}

/// `s.trim().toLowerCase().replace(/\s+/g, " ")`
pub(crate) fn normalize(s: &str) -> String {
    collapse_space(&trim(s).to_lowercase())
}

/// `s.replace(/\s+/g, " ")`
pub(crate) fn collapse_space(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if is_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// `s.length`, counted in UTF-16 code units.
pub(crate) fn len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `a < b` and friends on JS strings: lexicographic by UTF-16 code unit.
pub(crate) fn cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `/^\d+$/` with JS `\d` (ASCII digits only).
pub(crate) fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_like_javascript() {
        assert_eq!(trim("\u{FEFF} a \u{3000}"), "a");
        assert_eq!(trim("\u{0085}a"), "\u{0085}a");
    }

    #[test]
    fn normalizes_like_javascript() {
        assert_eq!(normalize("  Acme \t\n  Co  "), "acme co");
    }

    #[test]
    fn compares_by_utf16_code_unit() {
        // U+FF5E is above the surrogate range in UTF-16 but below U+1F600 in
        // UTF-8 byte order; JS puts the emoji (a surrogate pair) first.
        assert_eq!(cmp("\u{1F600}", "\u{FF5E}"), Ordering::Less);
        assert_eq!("\u{1F600}".cmp("\u{FF5E}"), Ordering::Greater);
    }
}
