//! Naming patterns such as `{verb}{noun}` or `{noun}{verb:pp}Event`.

/// A parsed naming pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    source: String,
    elements: Vec<Element>,
}

/// One part of a pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Element {
    /// `{noun}` or `{noun:<form>}`: a term from the dictionary.
    Noun(NounForm),
    /// `{verb}` or `{verb:<form>}`: a behavior of the term it is bound to.
    Verb(VerbForm),
    /// `{<name>}`: a word sequence from a vocabulary defined in the configuration.
    Vocabulary(String),
    /// `{any}`: any sequence of words.
    Any,
    /// Literal text such as `get` or `Event`, compared case-insensitively
    /// word by word.
    Literal(String),
}

/// A form of a noun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NounForm {
    Singular,
    Plural,
}

/// A form of a verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerbForm {
    Base,
    S,
    Past,
    Pp,
    Ing,
}

/// Placeholder names that cannot be used as vocabulary names.
pub const RESERVED_NAMES: [&str; 3] = ["noun", "verb", "any"];

const NOUN_FORMS: [(&str, NounForm); 2] = [
    ("singular", NounForm::Singular),
    ("plural", NounForm::Plural),
];

const VERB_FORMS: [(&str, VerbForm); 5] = [
    ("base", VerbForm::Base),
    ("s", VerbForm::S),
    ("past", VerbForm::Past),
    ("pp", VerbForm::Pp),
    ("ing", VerbForm::Ing),
];

impl Pattern {
    /// Parses a pattern. `is_vocabulary` tells whether a name refers to a
    /// vocabulary defined in the configuration.
    ///
    /// On failure, returns a message describing the problem.
    pub fn parse(source: &str, is_vocabulary: impl Fn(&str) -> bool) -> Result<Self, String> {
        if source.is_empty() {
            return Err("a pattern must not be empty".to_owned());
        }
        let mut elements = Vec::new();
        let mut rest = source;
        while !rest.is_empty() {
            if let Some(after_brace) = rest.strip_prefix('{') {
                let end = after_brace
                    .find(['{', '}'])
                    .filter(|&end| after_brace.as_bytes()[end] == b'}')
                    .ok_or_else(|| format!("unclosed `{{` in pattern `{source}`"))?;
                elements.push(parse_placeholder(&after_brace[..end], &is_vocabulary)?);
                rest = &after_brace[end + 1..];
            } else {
                let end = rest.find(['{', '}']).unwrap_or(rest.len());
                let literal = &rest[..end];
                if literal.is_empty() {
                    return Err(format!("unmatched `}}` in pattern `{source}`"));
                }
                if let Some(c) = literal
                    .chars()
                    .find(|&c| !(c.is_ascii_alphanumeric() || c == '_'))
                {
                    return Err(format!(
                        "invalid character `{c}` in pattern `{source}`: \
                         text outside placeholders may contain only ASCII letters, digits and `_`"
                    ));
                }
                elements.push(Element::Literal(literal.to_owned()));
                rest = &rest[end..];
            }
        }
        Ok(Self {
            source: source.to_owned(),
            elements,
        })
    }

    /// Returns the pattern as written.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the parts of the pattern in order.
    pub fn elements(&self) -> &[Element] {
        &self.elements
    }
}

fn parse_placeholder(
    content: &str,
    is_vocabulary: impl Fn(&str) -> bool,
) -> Result<Element, String> {
    let (name, form) = match content.split_once(':') {
        Some((name, form)) => (name, Some(form)),
        None => (content, None),
    };
    match name {
        "noun" => form
            .map_or(Ok(NounForm::Singular), |form| {
                lookup_form(name, form, &NOUN_FORMS)
            })
            .map(Element::Noun),
        "verb" => form
            .map_or(Ok(VerbForm::Base), |form| {
                lookup_form(name, form, &VERB_FORMS)
            })
            .map(Element::Verb),
        _ if name == "any" || is_vocabulary(name) => match form {
            Some(_) => Err(format!("`{{{name}}}` does not take a form")),
            None if name == "any" => Ok(Element::Any),
            None => Ok(Element::Vocabulary(name.to_owned())),
        },
        _ => Err(format!(
            "unknown placeholder `{{{content}}}`: expected `{{noun}}`, `{{verb}}`, `{{any}}` \
             or a vocabulary defined in `vocabularies`"
        )),
    }
}

fn lookup_form<T: Copy>(name: &str, form: &str, forms: &[(&str, T)]) -> Result<T, String> {
    forms
        .iter()
        .find(|(candidate, _)| *candidate == form)
        .map(|(_, value)| *value)
        .ok_or_else(|| {
            let expected: Vec<&str> = forms.iter().map(|(candidate, _)| *candidate).collect();
            format!(
                "unknown form `{form}` for `{{{name}}}`, expected one of {}",
                crate::yaml::quoted_list(&expected)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Result<Vec<Element>, String> {
        Pattern::parse(source, |name| name == "role").map(|pattern| pattern.elements)
    }

    fn literal(text: &str) -> Element {
        Element::Literal(text.to_owned())
    }

    #[test]
    fn parses_placeholders_and_literals() {
        use Element::*;
        assert_eq!(parse("{noun}"), Ok(vec![Noun(NounForm::Singular)]));
        assert_eq!(parse("{noun:plural}"), Ok(vec![Noun(NounForm::Plural)]));
        assert_eq!(
            parse("{verb}{noun}"),
            Ok(vec![Verb(VerbForm::Base), Noun(NounForm::Singular)])
        );
        assert_eq!(
            parse("{noun}{verb:pp}Event"),
            Ok(vec![
                Noun(NounForm::Singular),
                Verb(VerbForm::Pp),
                literal("Event")
            ])
        );
        assert_eq!(
            parse("{noun}{role}"),
            Ok(vec![
                Noun(NounForm::Singular),
                Vocabulary("role".to_owned())
            ])
        );
        assert_eq!(parse("get{any}"), Ok(vec![literal("get"), Any]));
        assert_eq!(
            parse("MAX_{noun:plural}"),
            Ok(vec![literal("MAX_"), Noun(NounForm::Plural)])
        );
    }

    #[test]
    fn parses_every_verb_form() {
        for (name, form) in VERB_FORMS {
            assert_eq!(
                parse(&format!("{{verb:{name}}}")),
                Ok(vec![Element::Verb(form)])
            );
        }
    }

    #[test]
    fn keeps_the_source() {
        let pattern = Pattern::parse("{verb}{noun}", |_| false).unwrap();
        assert_eq!(pattern.source(), "{verb}{noun}");
    }

    #[test]
    fn reports_malformed_patterns() {
        let cases = [
            ("", "a pattern must not be empty"),
            ("{noun", "unclosed `{` in pattern `{noun`"),
            ("{noun{verb}", "unclosed `{` in pattern `{noun{verb}`"),
            ("noun}", "unmatched `}` in pattern `noun}`"),
            ("{noun}}", "unmatched `}` in pattern `{noun}}`"),
            (
                "get-{any}",
                "invalid character `-` in pattern `get-{any}`: \
                 text outside placeholders may contain only ASCII letters, digits and `_`",
            ),
        ];
        for (source, message) in cases {
            assert_eq!(parse(source), Err(message.to_owned()), "pattern {source:?}");
        }
    }

    #[test]
    fn reports_unknown_placeholders_and_forms() {
        let cases = [
            (
                "{noun}{service}",
                "unknown placeholder `{service}`: expected `{noun}`, `{verb}`, `{any}` \
                 or a vocabulary defined in `vocabularies`",
            ),
            (
                "{}",
                "unknown placeholder `{}`: expected `{noun}`, `{verb}`, `{any}` \
                 or a vocabulary defined in `vocabularies`",
            ),
            (
                "{noun:past}",
                "unknown form `past` for `{noun}`, expected one of `singular`, `plural`",
            ),
            (
                "{verb:participle}",
                "unknown form `participle` for `{verb}`, expected one of \
                 `base`, `s`, `past`, `pp`, `ing`",
            ),
            ("{any:plural}", "`{any}` does not take a form"),
            ("{role:plural}", "`{role}` does not take a form"),
        ];
        for (source, message) in cases {
            assert_eq!(parse(source), Err(message.to_owned()), "pattern {source:?}");
        }
    }
}
