//! Positions in source text.

use std::fmt;

/// A position in a source file. Both `line` and `column` are 1-based, and
/// `column` counts characters (Unicode scalar values), so a tab or a
/// Japanese character each count as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// A range of source text. `end` is the position just after the last
/// character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: Location,
    pub end: Location,
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.start, self.end)
    }
}

/// Converts byte offsets in a source text into [`Location`]s.
///
/// Lines end with `\n`, `\r\n` or a lone `\r`, as in Java and YAML.
///
/// ```
/// use ubiqlint_core::{LineIndex, Location};
///
/// let source = "class 注文 {\n}";
/// let lines = LineIndex::new(source);
/// assert_eq!(lines.location(source.find('{').unwrap()), Location { line: 1, column: 10 });
/// assert_eq!(lines.location(source.find('}').unwrap()), Location { line: 2, column: 1 });
/// ```
#[derive(Clone, Debug)]
pub struct LineIndex<'a> {
    source: &'a str,
    /// The byte offset at which each line starts.
    line_starts: Vec<usize>,
    /// For each character longer than one byte, its byte offset and the
    /// number of bytes beyond the first in it and every such character
    /// before it. This turns byte offsets into character columns without
    /// rescanning long lines.
    multibyte: Vec<(usize, usize)>,
}

impl<'a> LineIndex<'a> {
    pub fn new(source: &'a str) -> Self {
        let bytes = source.as_bytes();
        let mut line_starts = vec![0];
        let mut multibyte = Vec::new();
        let mut extra_bytes = 0;
        for (offset, c) in source.char_indices() {
            if c.len_utf8() > 1 {
                extra_bytes += c.len_utf8() - 1;
                multibyte.push((offset, extra_bytes));
            }
            let ends_line = match c {
                '\n' => true,
                '\r' => bytes.get(offset + 1) != Some(&b'\n'),
                _ => false,
            };
            if ends_line {
                line_starts.push(offset + 1);
            }
        }
        Self {
            source,
            line_starts,
            multibyte,
        }
    }

    /// Returns the location of the byte at `offset`. An offset at the end of
    /// the source is the position just after the last character.
    ///
    /// # Panics
    ///
    /// Panics if `offset` is beyond the end of the source.
    pub fn location(&self, offset: usize) -> Location {
        assert!(
            offset <= self.source.len(),
            "offset {offset} is beyond the end of the source ({} bytes)",
            self.source.len()
        );
        // An offset inside a character is the location of that character.
        let mut offset = offset;
        while !self.source.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.line_starts.partition_point(|&start| start <= offset);
        let line_start = self.line_starts[line - 1];
        let extra_bytes = self.extra_bytes_before(offset) - self.extra_bytes_before(line_start);
        Location {
            line,
            column: offset - line_start - extra_bytes + 1,
        }
    }

    /// Returns how many bytes beyond the first the characters before
    /// `offset` have in total. `offset` must be a character boundary.
    fn extra_bytes_before(&self, offset: usize) -> usize {
        let count = self.multibyte.partition_point(|&(start, _)| start < offset);
        count
            .checked_sub(1)
            .map_or(0, |last| self.multibyte[last].1)
    }

    /// Returns the span of the bytes from `start` to `end`.
    pub fn span(&self, start: usize, end: usize) -> Span {
        Span {
            start: self.location(start),
            end: self.location(end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: usize, column: usize) -> Location {
        Location { line, column }
    }

    #[test]
    fn counts_lines_and_columns_from_one() {
        let lines = LineIndex::new("ab\ncd\n");
        assert_eq!(lines.location(0), at(1, 1));
        assert_eq!(lines.location(1), at(1, 2));
        assert_eq!(lines.location(2), at(1, 3));
        assert_eq!(lines.location(3), at(2, 1));
        assert_eq!(lines.location(5), at(2, 3));
        assert_eq!(lines.location(6), at(3, 1));
    }

    #[test]
    fn accepts_every_line_ending() {
        let lines = LineIndex::new("a\r\nb\rc\nd");
        assert_eq!(lines.location(1), at(1, 2));
        assert_eq!(lines.location(2), at(1, 3));
        assert_eq!(lines.location(3), at(2, 1));
        assert_eq!(lines.location(5), at(3, 1));
        assert_eq!(lines.location(7), at(4, 1));
    }

    #[test]
    fn counts_characters_rather_than_bytes() {
        let source = "\tint 注文 = 1;";
        let lines = LineIndex::new(source);
        let offset = source.find('=').unwrap();
        assert_eq!(lines.location(offset), at(1, 9));
        // Inside the three bytes of `注`.
        assert_eq!(lines.location(6), at(1, 6));
    }

    #[test]
    fn agrees_with_counting_characters_at_every_offset() {
        let source = "a\t注文😀b\r\n\tÉtat x\rß\n😀😀";
        let lines = LineIndex::new(source);
        for offset in 0..=source.len() {
            let mut boundary = offset;
            while !source.is_char_boundary(boundary) {
                boundary -= 1;
            }
            // Walk the characters before the offset one at a time.
            let mut expected = at(1, 1);
            for (i, c) in source[..boundary].char_indices() {
                let ends_line = c == '\n' || (c == '\r' && !source[i + 1..].starts_with('\n'));
                expected = if ends_line {
                    at(expected.line + 1, 1)
                } else {
                    at(expected.line, expected.column + 1)
                };
            }
            assert_eq!(lines.location(offset), expected, "offset {offset}");
        }
    }

    #[test]
    fn handles_empty_sources() {
        assert_eq!(LineIndex::new("").location(0), at(1, 1));
    }

    #[test]
    fn makes_spans() {
        let lines = LineIndex::new("ab\ncd");
        assert_eq!(
            lines.span(1, 4),
            Span {
                start: at(1, 2),
                end: at(2, 2)
            }
        );
        assert_eq!(lines.span(1, 4).to_string(), "1:2-2:2");
    }

    #[test]
    #[should_panic(expected = "offset 3 is beyond the end of the source (2 bytes)")]
    fn rejects_offsets_beyond_the_end() {
        LineIndex::new("ab").location(3);
    }
}
