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
        let mut lines = Vec::new();
        let mut start = 0;
        let mut chars = text.char_indices().peekable();
        while let Some((offset, c)) = chars.next() {
            if c == '\r' || c == '\n' {
                lines.push(&text[start..offset]);
                start = offset + 1;
                if c == '\r' && chars.peek().is_some_and(|(_, c)| *c == '\n') {
                    chars.next();
                    start += 1;
                }
            }
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
}
