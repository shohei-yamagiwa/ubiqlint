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
//! the original text, so that locations point at what the user wrote. A
//! malformed escape such as `\u00G1` is an error, as in Java, even in a
//! comment.

use std::borrow::Cow;

/// A Unicode escape that Java rejects, such as `\u00G1`, even in a comment.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MalformedEscape {
    /// The byte offset of its backslash in the original text.
    pub(crate) offset: usize,
}

/// Java source text after lexical translation.
pub(crate) struct Translated<'a> {
    text: Cow<'a, str>,
    /// For each Unicode escape, the offsets just after it in the translated
    /// and in the original text.
    escapes: Vec<(usize, usize)>,
}

impl<'a> Translated<'a> {
    /// Translates `source`, or returns its malformed Unicode escapes.
    pub(crate) fn new(source: &'a str) -> Result<Self, Vec<MalformedEscape>> {
        if !source.contains(['\\', '\r']) {
            return Ok(Self {
                text: Cow::Borrowed(source),
                escapes: Vec::new(),
            });
        }
        let mut text = String::with_capacity(source.len());
        let mut escapes = Vec::new();
        let mut malformed = Vec::new();
        let mut rest = source;
        // Which backslashes may start an escape follows javac. Backslashes
        // pair up, counting those produced by escapes, and one that completes
        // a pair starts nothing, so `\\\u0041` is not an escape. But a
        // backslash right after an escape may always start one.
        let mut completes_pair = false;
        let mut after_escape = false;
        while let Some(c) = rest.chars().next() {
            let offset = source.len() - rest.len();
            if c == '\\' && (!completes_pair || after_escape) {
                match unicode_escape(rest) {
                    Escape::None => {}
                    Escape::Malformed => malformed.push(MalformedEscape { offset }),
                    Escape::Unit(unit, length) => {
                        let (c, length) = combine_surrogates(unit, length, &rest[length..]);
                        text.push(c);
                        rest = &rest[length..];
                        escapes.push((text.len(), source.len() - rest.len()));
                        completes_pair = c == '\\' && !completes_pair;
                        after_escape = true;
                        continue;
                    }
                }
            }
            completes_pair = c == '\\' && !completes_pair;
            after_escape = false;
            text.push(c);
            rest = &rest[c.len_utf8()..];
        }
        if !malformed.is_empty() {
            return Err(malformed);
        }
        Ok(Self {
            text: Cow::Owned(normalize_lone_carriage_returns(text)),
            escapes,
        })
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

/// What follows a backslash that may start a Unicode escape.
enum Escape {
    /// No `u`, so the backslash is an ordinary character.
    None,
    /// `u` without four hexadecimal digits after it.
    Malformed,
    /// A UTF-16 code unit, and the length of the escape in bytes.
    Unit(u16, usize),
}

/// Reads a Unicode escape at the start of `text`: `\`, one or more `u`, and
/// four hexadecimal digits.
fn unicode_escape(text: &str) -> Escape {
    let Some(after_backslash) = text.strip_prefix('\\') else {
        return Escape::None;
    };
    let digits = after_backslash.trim_start_matches('u');
    if digits.len() == after_backslash.len() {
        return Escape::None;
    }
    match digits
        .get(..4)
        .filter(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        Some(hex) => Escape::Unit(
            u16::from_str_radix(hex, 16).expect("four hexadecimal digits fit in 16 bits"),
            text.len() - digits.len() + 4,
        ),
        None => Escape::Malformed,
    }
}

/// Turns a code unit into a character, joining a high surrogate with a low
/// surrogate escape right after it. A surrogate without its pair, which Java
/// allows only in literals and comments, becomes U+FFFD.
fn combine_surrogates(unit: u16, length: usize, rest: &str) -> (char, usize) {
    if (0xD800..0xDC00).contains(&unit) {
        if let Escape::Unit(low, low_length) = unicode_escape(rest) {
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
        let translated = Translated::new("class Order {}\n").unwrap();
        assert!(matches!(translated.text, Cow::Borrowed(_)));
        assert_eq!(translated.original_offset(6), 6);
    }

    #[test]
    fn translates_unicode_escapes() {
        let source = r"class \u004frder { int \uuu0062; }";
        let translated = Translated::new(source).unwrap();
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
        let translated = Translated::new(r#"s = "\\u0041" + "\\\u0041";"#).unwrap();
        assert_eq!(translated.text(), r#"s = "\\u0041" + "\\A";"#);
        // A backslash produced by an escape does not start another one.
        assert_eq!(text(r"\u005cu0041"), r"\u0041");
    }

    fn text(source: &str) -> String {
        Translated::new(source).unwrap().text().to_owned()
    }

    #[test]
    fn pairs_backslashes_as_javac_does() {
        // Each case was checked with javac 21: the final escape is a line
        // feed exactly when javac compiles a class declared after it.
        let cases = [
            // A produced backslash and a raw one make a pair.
            (r"\u005c\\u000a", concat!(r"\\", "\n")),
            // A backslash right after an escape may start one.
            (r"\u005c\u000a", concat!(r"\", "\n")),
            (r"\u005c\u005c\u000a", concat!(r"\\", "\n")),
            (r"\u005c\\u005c\u000a", concat!(r"\\\", "\n")),
            // A raw backslash that completes a pair starts nothing.
            (r"\\u000a", r"\\u000a"),
            (r"\u005c\\\u000a", r"\\\\u000a"),
            (r"\u005c\u005c\\u000a", r"\\\\u000a"),
            (r"\\\u000a", concat!(r"\\", "\n")),
        ];
        for (source, expected) in cases {
            assert_eq!(text(source), expected, "translating {source:?}");
        }
    }

    #[test]
    fn reports_malformed_escapes() {
        for (source, offset) in [
            (r"// \u00G1", 3),
            (r"\u004", 0),
            (r"a \uuu", 2),
            (r"\u0041 \u", 7),
        ] {
            assert_eq!(
                Translated::new(source).err(),
                Some(vec![MalformedEscape { offset }]),
                "translating {source:?}"
            );
        }
        let errors = Translated::new(r"\u \uG").err().unwrap();
        assert_eq!(
            errors,
            [MalformedEscape { offset: 0 }, MalformedEscape { offset: 3 }]
        );
    }

    #[test]
    fn accepts_backslashes_that_start_no_escape() {
        for source in [
            // Not followed by `u`.
            r"\x0041",
            r"\",
            // Completes a pair, so it cannot start an escape.
            r"\\u00G1",
            // A produced backslash does not start another escape.
            r"\u005cu00G1",
        ] {
            assert!(Translated::new(source).is_ok(), "translating {source:?}");
        }
        assert_eq!(text(r"\u005cu00G1"), r"\u00G1");
    }

    #[test]
    fn joins_surrogate_pairs() {
        let source = r"int \uD835\uDC00 = 1;";
        let translated = Translated::new(source).unwrap();
        assert_eq!(translated.text(), "int 𝐀 = 1;");
        assert_eq!(translated.original_offset(4), 4);
        assert_eq!(translated.original_offset(8), source.find(" =").unwrap());
        assert_eq!(text(r#""\uD800""#), "\"\u{FFFD}\"");
    }

    #[test]
    fn turns_lone_carriage_returns_into_line_feeds() {
        assert_eq!(text("// a\rb\r\nc\u{000D}"), "// a\nb\r\nc\n");
        // Including those written as escapes.
        assert_eq!(text(r"// a\u000db"), "// a\nb");
    }
}
