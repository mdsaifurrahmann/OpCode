//! Code Page 437: the character set real DOS and BIOS text-mode output
//! (and emu8086's own simulation of it) use, as opposed to Rust's
//! built-in `u8 as char` cast, which decodes a byte as Latin-1 instead.
//! The two agree for the printable ASCII range (32-126), but diverge
//! completely for bytes 128-255 - exactly the range real 8086 programs
//! use for box-drawing borders, accented letters, and math/Greek
//! symbols, all common in tutorial-style console output. Printing those
//! through the wrong table doesn't error, it just silently shows the
//! wrong character (or an invisible C1 control character for several of
//! them), which is what shows up as "garbage" in the console.

/// Bytes 128-255 of CP437, in order - Unicode Consortium's published
/// mapping (the standard reference for this code page).
const UPPER: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ',
    'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ',
    'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕',
    '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦',
    '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐',
    '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±',
    '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{00A0}',
];

/// Maps a raw DOS/BIOS output byte to the character it actually
/// represents under CP437.
pub fn to_char(byte: u8) -> char {
    if byte < 128 {
        byte as char
    } else {
        UPPER[(byte - 128) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_range_is_unchanged() {
        assert_eq!(to_char(b'A'), 'A');
        assert_eq!(to_char(b'$'), '$');
        assert_eq!(to_char(b'\n'), '\n');
        assert_eq!(to_char(0), '\0');
        assert_eq!(to_char(127), '\u{7F}');
    }

    #[test]
    fn extended_range_uses_cp437_not_latin1() {
        // 0xC9 is '╔' (box-drawing) under CP437, but 'É' under Latin-1 -
        // exactly the kind of divergence that showed up as garbage.
        assert_eq!(to_char(0xC9), '╔');
        assert_eq!(to_char(0xB2), '▓');
        assert_eq!(to_char(0xE1), 'ß');
        assert_eq!(to_char(0x87), 'ç');
    }

    #[test]
    fn table_has_exactly_128_entries() {
        assert_eq!(UPPER.len(), 128);
    }
}
