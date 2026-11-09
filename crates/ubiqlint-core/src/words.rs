//! Word sequences used for exact matching.

use std::fmt;

/// A non-empty sequence of lowercase words, such as `line item`.
///
/// Dictionaries and configuration files spell every form out as a word
/// sequence, so the linter only ever needs exact comparisons.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Words(Vec<String>);

impl Words {
    /// Parses lowercase ASCII words separated by single spaces.
    ///
    /// On failure, returns a message explaining the expected format.
    pub fn parse(text: &str) -> Result<Self, String> {
        let words: Vec<&str> = text.split(' ').collect();
        let valid = words.iter().all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
        if valid {
            Ok(Self(words.into_iter().map(str::to_owned).collect()))
        } else {
            Err(format!(
                "`{text}` is not a valid word sequence: use lowercase letters and digits, \
                 with a single space between words (e.g. `line item`)"
            ))
        }
    }

    /// Returns the words in order.
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Display for Words {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_and_multiple_words() {
        assert_eq!(Words::parse("order").unwrap().as_slice(), ["order"]);
        assert_eq!(
            Words::parse("line items").unwrap().as_slice(),
            ["line", "items"]
        );
        assert_eq!(Words::parse("b2b").unwrap().as_slice(), ["b2b"]);
    }

    #[test]
    fn displays_words_separated_by_spaces() {
        assert_eq!(Words::parse("line item").unwrap().to_string(), "line item");
    }

    #[test]
    fn rejects_invalid_sequences() {
        for text in [
            "",
            "LineItem",
            "line_item",
            "line  item",
            " order",
            "order ",
            "注文",
        ] {
            assert!(Words::parse(text).is_err(), "{text:?} should be rejected");
        }
    }
}
