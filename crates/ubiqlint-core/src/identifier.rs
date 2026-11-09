//! Splitting identifiers into lowercase words.
//!
//! The rules are deterministic and depend only on the characters of the
//! identifier:
//!
//! - Any character that is not a letter or digit, such as `_` or `$`,
//!   separates words: `ORDER_ID` → `order id`.
//! - An uppercase letter after a lowercase letter starts a word:
//!   `placeOrder` → `place order`.
//! - In a run of uppercase letters, the last one starts a word if a
//!   lowercase letter follows it: `HTTPOrder` → `http order`.
//! - The exception is a lone `s` ending the run's word, which is read as
//!   the plural of an acronym: `URLs` → `urls`, `IDsByOrder` → `ids by order`.
//! - Digits belong to the word they follow and never start a word by
//!   themselves, except after a separator or at the start:
//!   `base64Encode` → `base64 encode`, `B2BOrder` → `b2b order`,
//!   `3DSecure` → `3d secure`.
//! - Letters without case, such as Japanese, behave like lowercase letters.
//!
//! Words are lowercased, so `{noun}` written as `order` in a dictionary
//! matches `Order`, `order` and `ORDER` alike.

/// Splits an identifier into lowercase words.
///
/// ```
/// use ubiqlint_core::identifier::split;
///
/// assert_eq!(split("HTTPOrderClient"), ["http", "order", "client"]);
/// assert_eq!(split("ORDER_PLACED"), ["order", "placed"]);
/// ```
pub fn split(identifier: &str) -> Vec<String> {
    let mut words = Vec::new();
    for segment in identifier.split(|c: char| !c.is_alphanumeric()) {
        split_segment(segment, &mut words);
    }
    words
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Upper,
    Lower,
    Digit,
}

fn class(c: char) -> Class {
    if c.is_uppercase() {
        Class::Upper
    } else if c.is_numeric() {
        Class::Digit
    } else {
        // Lowercase letters and letters without case.
        Class::Lower
    }
}

/// Splits a run of letters and digits, appending its words to `words`.
fn split_segment(segment: &str, words: &mut Vec<String>) {
    let chars: Vec<(usize, char)> = segment.char_indices().collect();
    let mut start = 0;
    // The class of the last letter in the current word, skipping digits.
    let mut last_letter = None;
    for (i, &(offset, c)) in chars.iter().enumerate() {
        let current = class(c);
        if current == Class::Upper && starts_word(last_letter, &chars[i + 1..]) {
            words.push(segment[start..offset].to_lowercase());
            start = offset;
            last_letter = None;
        }
        if current != Class::Digit {
            last_letter = Some(current);
        }
    }
    if start < segment.len() {
        words.push(segment[start..].to_lowercase());
    }
}

/// Tells whether an uppercase letter starts a new word, given the class of
/// the last letter before it in the current word and the characters after it.
fn starts_word(last_letter: Option<Class>, rest: &[(usize, char)]) -> bool {
    match last_letter {
        // `placeOrder`, `order2Csv`
        Some(Class::Lower) => true,
        // The last capital of an acronym: `HTTPOrder`, `B2BOrder`, but not
        // a plural acronym such as `URLs`.
        Some(Class::Upper) => {
            let next = rest.first().map(|&(_, c)| c);
            let after_next = rest.get(1).map(|&(_, c)| class(c));
            match next {
                Some('s') => after_next == Some(Class::Lower),
                Some(c) => class(c) == Class::Lower,
                None => false,
            }
        }
        // The word so far is empty or digits only: `3DSecure`.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(cases: &[(&str, &[&str])]) {
        for (identifier, expected) in cases {
            assert_eq!(split(identifier), *expected, "splitting {identifier:?}");
        }
    }

    #[test]
    fn splits_camel_and_pascal_case() {
        check(&[
            ("order", &["order"]),
            ("Order", &["order"]),
            ("placeOrder", &["place", "order"]),
            ("PlaceOrder", &["place", "order"]),
            ("lineItems", &["line", "items"]),
            ("OrderPlacedEvent", &["order", "placed", "event"]),
            ("x", &["x"]),
            ("A", &["a"]),
        ]);
    }

    #[test]
    fn splits_at_separators() {
        check(&[
            ("ORDER_ID", &["order", "id"]),
            ("order_id", &["order", "id"]),
            ("__order__", &["order"]),
            ("order$1", &["order", "1"]),
            ("MAX_ORDERS_2", &["max", "orders", "2"]),
            ("place_Order", &["place", "order"]),
            ("", &[]),
            ("_", &[]),
        ]);
    }

    #[test]
    fn splits_acronyms() {
        check(&[
            ("HTTPOrderClient", &["http", "order", "client"]),
            ("getURL", &["get", "url"]),
            ("IOStream", &["io", "stream"]),
            ("HTTPServer", &["http", "server"]),
            ("HTTPSession", &["http", "session"]),
            ("ASet", &["a", "set"]),
            ("AB", &["ab"]),
            ("Ab", &["ab"]),
        ]);
    }

    #[test]
    fn keeps_plural_acronyms_together() {
        check(&[
            ("URLs", &["urls"]),
            ("orderIDs", &["order", "ids"]),
            ("IDsByOrder", &["ids", "by", "order"]),
            ("HTTPs", &["https"]),
            ("UUIDs2", &["uuids2"]),
        ]);
    }

    #[test]
    fn attaches_digits_to_the_preceding_word() {
        check(&[
            ("order2Csv", &["order2", "csv"]),
            ("order2CSV", &["order2", "csv"]),
            ("base64Encode", &["base64", "encode"]),
            ("Base64URL", &["base64", "url"]),
            ("SHA256Hash", &["sha256", "hash"]),
            ("UTF8String", &["utf8", "string"]),
            ("B2BOrder", &["b2b", "order"]),
            ("B2bOrder", &["b2b", "order"]),
            ("b2bOrder", &["b2b", "order"]),
            ("V2Api", &["v2", "api"]),
            ("getV2", &["get", "v2"]),
            ("Order2", &["order2"]),
        ]);
    }

    #[test]
    fn starts_words_with_leading_digits() {
        check(&[
            ("2fa", &["2fa"]),
            ("2FA", &["2fa"]),
            ("3DSecure", &["3d", "secure"]),
        ]);
    }

    #[test]
    fn reads_mixed_case_brands_by_the_same_rules() {
        // `IPv4` is not a recognizable acronym run, so the rules split it
        // like any other identifier.
        check(&[("IPv4Address", &["i", "pv4", "address"])]);
    }

    #[test]
    fn handles_letters_beyond_ascii() {
        check(&[
            ("注文Service", &["注文", "service"]),
            ("placeÉtat", &["place", "état"]),
            ("ÉTAT_CIVIL", &["état", "civil"]),
        ]);
    }
}
