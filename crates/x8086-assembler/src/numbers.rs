//! Numeric literal parsing: emu8086/MASM-style suffixed integers.
//!
//! The lexer only ever starts a `Number` token on an ASCII digit (see
//! `tokenize`), so every string this module receives already begins with
//! `0`-`9` - exactly the constraint that makes suffix-based radix
//! disambiguation unambiguous (a hex literal like `FFh` must be written
//! `0FFh` for the same reason real MASM/TASM/emu8086 require it).

/// Parse a suffixed integer literal: `1234` (decimal), `1234d` (decimal),
/// `1234h`/`0FFh` (hex), `1010b` (binary), `17o`/`17q` (octal). Returns
/// `None` for malformed input (e.g. `12h3`, or `b`-suffixed text that
/// isn't valid binary).
pub fn parse_number(text: &str) -> Option<i64> {
    let lower = text.to_ascii_lowercase();

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
}
