//! Converting the core's character columns to the negotiated LSP encoding.

use lsp_types::{Position, PositionEncodingKind, Range};
use ubiqlint_core::{Diagnostic, Location};

pub(crate) struct Positions<'a> {
    lines: Vec<&'a str>,
    utf32: bool,
}

impl<'a> Positions<'a> {
    pub(crate) fn new(text: &'a str, encoding: &PositionEncodingKind) -> Self {
        // str::lines does not treat a lone CR as a line ending. Both Java
        // and LSP do, and a CRLF is one line ending, not two.
        // Keep this consistent with ubiqlint_core::LineIndex's line endings.
        let mut lines = Vec::new();
        let mut start = 0;
        let bytes = text.as_bytes();
        let mut offset = 0;
        // ASCII line terminators cannot occur inside a multibyte UTF-8 character.
        while offset < bytes.len() {
            let byte = bytes[offset];
            if byte == b'\r' || byte == b'\n' {
                lines.push(&text[start..offset]);
                if byte == b'\r' && bytes.get(offset + 1) == Some(&b'\n') {
                    offset += 1;
                }
                start = offset + 1;
            }
            offset += 1;
        }
        lines.push(&text[start..]);
        Self {
            lines,
            utf32: *encoding == PositionEncodingKind::UTF32,
        }
    }

    fn position(&self, location: Location) -> Position {
        let line = location.line.saturating_sub(1).min(self.lines.len() - 1);
        let chars = self.lines[line]
            .chars()
            .take(location.column.saturating_sub(1));
        let character = if self.utf32 {
            chars.count()
        } else {
            chars.map(char::len_utf16).sum()
        };
        Position::new(line as u32, character as u32)
    }

    pub(crate) fn range(&self, diagnostic: &Diagnostic) -> Range {
        match diagnostic.span {
            Some(span) => Range::new(self.position(span.start), self.position(span.end)),
            None => {
                let position = self.position(diagnostic.location);
                Range::new(position, position)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_scalars_to_utf16_and_accepts_every_line_ending() {
        let text = "😀注文 x\r\n😀 y\rz\n";
        for (encoding, expected) in [
            (PositionEncodingKind::UTF16, 5),
            (PositionEncodingKind::UTF32, 4),
        ] {
            let positions = Positions::new(text, &encoding);
            assert_eq!(
                positions.position(Location { line: 1, column: 5 }),
                Position::new(0, expected)
            );
            assert_eq!(
                positions.position(Location { line: 3, column: 1 }),
                Position::new(2, 0)
            );
            assert_eq!(
                positions.position(Location { line: 4, column: 1 }),
                Position::new(3, 0)
            );
        }
    }

    #[test]
    fn agrees_with_core_line_index_at_character_boundaries() {
        for text in ["", "a\r\n", "\r\n\r\nb", "😀注文\rx\n\r\n終\n"] {
            let core = ubiqlint_core::LineIndex::new(text);
            let positions = Positions::new(text, &PositionEncodingKind::UTF32);
            for offset in 0..=text.len() {
                if !text.is_char_boundary(offset)
                    || (offset > 0
                        && text.as_bytes().get(offset) == Some(&b'\n')
                        && text.as_bytes()[offset - 1] == b'\r')
                {
                    // LSP has no position between the CR and LF of a CRLF.
                    continue;
                }
                let location = core.location(offset);
                assert_eq!(
                    positions.position(location),
                    Position::new((location.line - 1) as u32, (location.column - 1) as u32),
                    "{text:?} at {offset}"
                );
            }
        }
    }
}
