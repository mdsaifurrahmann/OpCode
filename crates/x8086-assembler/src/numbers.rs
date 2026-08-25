//! Numeric literal parsing: both emu8086/MASM-style suffixed integers
//! (`0FFh`) and NASM/C-style prefixed ones (`0xFF`).
//!
//! The lexer only ever starts a `Number` token on an ASCII digit (see
//! `tokenize`), so every string this module receives already begins with
//! `0`-`9` - exactly the constraint that makes suffix-based radix
//! disambiguation unambiguous (a hex literal like `FFh` must be written
//! `0FFh` for the same reason real MASM/TASM/emu8086 require it), and
//! also why every prefixed form here still starts with a `0`.

/// Parse an integer literal in either dialect's notation:
///
/// - suffixed (MASM/emu8086): `1234`, `1234d`, `1234h`/`0FFh`, `1010b`,
///   `17o`/`17q`
/// - prefixed (NASM/C): `0x1F`, `0b1011`, `0o17`
///
/// Returns `None` for malformed input (e.g. `12h3`, or `b`-suffixed text
/// that isn't valid binary).
pub fn parse_number(text: &str) -> Option<i64> {
    let lower = text.to_ascii_lowercase();

    // Prefixes are tried first but fall through on failure rather than
    // erroring out, because the two notations genuinely overlap: `0b8h`
    // is a MASM hex literal (0B8h = 184), yet it also *looks* like a
    // `0b` binary prefix followed by "8h". Only the suffix reading is
    // valid there, so a failed prefix parse must not be the final word.
    if let Some(value) = parse_prefixed(&lower) {
        return Some(value);
    }

    if let Some(digits) = lower.strip_suffix('h') {
        return i64::from_str_radix(digits, 16).ok();
    }
    if let Some(digits) = lower.strip_suffix('o').or_else(|| lower.strip_suffix('q')) {
        return i64::from_str_radix(digits, 8).ok();
    }
    if let Some(digits) = lower.strip_suffix('b') {
        if !digits.is_empty() && digits.bytes().all(|b| b == b'0' || b == b'1') {
            return i64::from_str_radix(digits, 2).ok();
        }
        return None;
    }
    if let Some(digits) = lower.strip_suffix('d') {
        return digits.parse().ok();
    }
    lower.parse().ok()
}

/// `0x`/`0b`/`0o` prefixed forms. Requires at least one digit after the
/// prefix, so a bare `0b` stays what the suffix rules already made it
/// (binary "0", i.e. zero) instead of becoming a malformed prefix.
fn parse_prefixed(lower: &str) -> Option<i64> {
    let (radix, digits) = match lower.as_bytes() {
        [b'0', b'x', rest @ ..] if !rest.is_empty() => (16, &lower[2..]),
        [b'0', b'b', rest @ ..] if !rest.is_empty() => (2, &lower[2..]),
        [b'0', b'o', rest @ ..] if !rest.is_empty() => (8, &lower[2..]),
        _ => return None,
    };
    i64::from_str_radix(digits, radix).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_decimal() {
        assert_eq!(parse_number("42"), Some(42));
        assert_eq!(parse_number("0"), Some(0));
    }

    #[test]
    fn decimal_with_explicit_suffix() {
        assert_eq!(parse_number("42d"), Some(42));
        assert_eq!(parse_number("42D"), Some(42));
    }

    #[test]
    fn hexadecimal() {
        assert_eq!(parse_number("0FFh"), Some(0xFF));
        assert_eq!(parse_number("1234H"), Some(0x1234));
    }

    #[test]
    fn binary() {
        assert_eq!(parse_number("1010b"), Some(0b1010));
        assert_eq!(parse_number("0b"), Some(0));
    }

    #[test]
    fn octal() {
        assert_eq!(parse_number("17o"), Some(0o17));
        assert_eq!(parse_number("17q"), Some(0o17));
    }

    #[test]
    fn malformed_binary_suffix_is_rejected_not_misread() {
        // "19b" isn't valid binary (9 isn't 0/1), and without an 'h'
        // suffix it also isn't valid hex, so this must fail to parse
        // rather than silently guessing.
        assert_eq!(parse_number("19b"), None);
    }

    #[test]
    fn garbage_is_none() {
        assert_eq!(parse_number("12h3"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn nasm_style_prefixed_literals() {
        assert_eq!(parse_number("0xF9"), Some(0xF9));
        assert_eq!(parse_number("0XfF"), Some(0xFF));
        assert_eq!(parse_number("0b1011"), Some(0b1011));
        assert_eq!(parse_number("0o17"), Some(0o17));
    }

    #[test]
    fn both_notations_agree_on_the_same_value() {
        assert_eq!(parse_number("0xF9"), parse_number("0F9h"));
        assert_eq!(parse_number("0b1010"), parse_number("1010b"));
        assert_eq!(parse_number("0o17"), parse_number("17o"));
    }

    #[test]
    fn a_masm_hex_literal_that_looks_like_a_binary_prefix_still_parses_as_hex() {
        // "0B8h" starts with what looks like NASM's `0b` binary prefix,
        // but "8h" isn't binary - the suffix reading (0B8h = 184) is the
        // only valid one, so a failed prefix parse must fall through
        // rather than reject the literal outright.
        assert_eq!(parse_number("0B8h"), Some(0xB8));
        assert_eq!(parse_number("0b1h"), Some(0xB1));
    }

    #[test]
    fn a_bare_0b_is_still_binary_zero() {
        // Unchanged by prefix support: there are no digits after the
        // prefix, so this stays the suffix reading (binary "0").
        assert_eq!(parse_number("0b"), Some(0));
    }

    #[test]
    fn malformed_prefixed_literals_are_rejected() {
        assert_eq!(parse_number("0x"), None);
        assert_eq!(parse_number("0xZZ"), None);
        assert_eq!(parse_number("0b12"), None);
    }
}
