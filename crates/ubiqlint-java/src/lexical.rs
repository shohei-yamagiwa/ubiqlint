//! The lexical translations that Java applies before tokenizing
//! (JLS §3.2–3.3), which `tree-sitter-java` does not.
//!
//! - Unicode escapes such as `A` are replaced by the characters they
//!   stand for, so `class \u004frder` declares `Order` and `\u000a` ends a
//!   line comment.
//! - A lone `\r` becomes `\n`, since the grammar ends line comments only at
//!   `\n`.
//!
//! The translated text is what gets parsed. Offsets in it are mapped back to
//! the original text, so that locations point at what the user wrote.

use std::borrow::Cow;

/// Java source text after lexical translation.
pub(crate) struct Translated<'a> {
    text: Cow<'a, str>,
    /// For each Unicode escape, the offsets just after it in the translated
    /// and in the original text.
    escapes: Vec<(usize, usize)>,
}

impl<'a> Translated<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        if !source.contains(['\\', '\r']) {
            return Self {
                text: Cow::Borrowed(source),
                escapes: Vec::new(),
            };
        }
        let mut text = String::with_capacity(source.len());
        let mut escapes = Vec::new();
        let mut rest = source;
        // Whether the next raw `\` follows an odd number of raw `\`, in which
        // case it cannot start an escape: `\\u0041` is not an escape.
        let mut escaped = false;
        while let Some(c) = rest.chars().next() {
            if c == '\\' && !escaped {
                if let Some((unit, length)) = unicode_escape(rest) {
                    let (c, length) = combine_surrogates(unit, length, &rest[length..]);
                    text.push(c);
                    rest = &rest[length..];
                    escapes.push((text.len(), source.len() - rest.len()));
                    continue;
                }
            }
            escaped = c == '\\' && !escaped;
            text.push(c);
            rest = &rest[c.len_utf8()..];
        }
        Self {
            text: Cow::Owned(normalize_lone_carriage_returns(text)),
            escapes,
        }
    }

    /// Returns the text to parse.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Maps an offset in the translated text to the original text.
    pub(crate) fn original_offset(&self, offset: usize) -> usize {
        let count = self.escapes.partition_point(|&(after, _)| after <= offset);
        match count.checked_sub(1) {
            Some(last) => {
                let (translated, original) = self.escapes[last];
                original + (offset - translated)
            }
            None => offset,
        }
    }
}

/// Reads a Unicode escape at the start of `text`: `\`, one or more `u`, and
/// four hexadecimal digits. Returns the UTF-16 code unit and the length of
/// the escape in bytes.
fn unicode_escape(text: &str) -> Option<(u16, usize)> {
    let after_backslash = text.strip_prefix('\\')?;
    let digits = after_backslash.trim_start_matches('u');
    if digits.len() == after_backslash.len() {
        return None;
    }
    let hex = digits.get(..4)?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let unit = u16::from_str_radix(hex, 16).ok()?;
    Some((unit, text.len() - digits.len() + 4))
}

/// Turns a code unit into a character, joining a high surrogate with a low
/// surrogate escape right after it. A surrogate without its pair, which Java
/// allows only in literals and comments, becomes U+FFFD.
fn combine_surrogates(unit: u16, length: usize, rest: &str) -> (char, usize) {
    if (0xD800..0xDC00).contains(&unit) {
        if let Some((low, low_length)) = unicode_escape(rest) {
            if let Some(Ok(c)) = char::decode_utf16([unit, low]).next() {
                return (c, length + low_length);
            }
        }
    }
    let c = char::decode_utf16([unit])
        .next()
        .and_then(Result::ok)
        .unwrap_or(char::REPLACEMENT_CHARACTER);
    (c, length)
}

fn normalize_lone_carriage_returns(text: String) -> String {
    if !text.contains('\r') {
        return text;
    }
    let mut bytes = text.into_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) != Some(&b'\n') {
            bytes[i] = b'\n';
        }
    }
    String::from_utf8(bytes).expect("replacing one ASCII byte with another keeps UTF-8 valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_plain_text_alone() {
        let translated = Translated::new("class Order {}\n");
        assert!(matches!(translated.text, Cow::Borrowed(_)));
        assert_eq!(translated.original_offset(6), 6);
    }

    #[test]
    fn translates_unicode_escapes() {
        let source = r"class \u004frder { int \uuu0062; }";
        let translated = Translated::new(source);
        assert_eq!(translated.text(), "class Order { int b; }");
        // `O` and the text after it.
        assert_eq!(translated.original_offset(6), 6);
        assert_eq!(translated.original_offset(7), 12);
        // `b` and the `;` after it.
        assert_eq!(
            translated.original_offset(18),
            source.find(r"\uuu").unwrap()
        );
        assert_eq!(translated.original_offset(19), source.find(';').unwrap());
    }

    #[test]
    fn skips_backslashes_that_are_escaped() {
        // The second `\` follows an odd number of `\`, so it starts nothing.
        let translated = Translated::new(r#"s = "\\u0041" + "\\\u0041";"#);
        assert_eq!(translated.text(), r#"s = "\\u0041" + "\\A";"#);
        // A backslash produced by an escape does not start another one.
        assert_eq!(Translated::new(r"\u005cu0041").text(), r"\u0041");
    }

    #[test]
    fn leaves_malformed_escapes_alone() {
        for source in [r"\u004", r"\u00G1", r"\x0041", r"\"] {
            assert_eq!(Translated::new(source).text(), source);
        }
    }

    #[test]
    fn joins_surrogate_pairs() {
        let source = r"int \uD835\uDC00 = 1;";
        let translated = Translated::new(source);
        assert_eq!(translated.text(), "int 𝐀 = 1;");
        assert_eq!(translated.original_offset(4), 4);
        assert_eq!(translated.original_offset(8), source.find(" =").unwrap());
        assert_eq!(Translated::new(r#""\uD800""#).text(), "\"\u{FFFD}\"");
    }

    #[test]
    fn turns_lone_carriage_returns_into_line_feeds() {
        assert_eq!(
            Translated::new("// a\rb\r\nc\u{000D}").text(),
            "// a\nb\r\nc\n"
        );
        // Including those written as escapes.
        assert_eq!(Translated::new(r"// a\u000db").text(), "// a\nb");
    }
}
