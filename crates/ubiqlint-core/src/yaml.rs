//! Helpers for turning a YAML tree into definitions while collecting every
//! problem found along the way, each with its location.

use saphyr::{LoadableYamlNode, MarkedYaml, Marker, Scalar, YamlData};

use crate::diagnostic::Diagnostic;
use crate::location::Location;
use crate::words::Words;

pub(crate) type Node<'input> = MarkedYaml<'input>;

/// Collects diagnostics while reading a document.
#[derive(Default)]
pub(crate) struct Diagnostics(Vec<Diagnostic>);

impl Diagnostics {
    pub(crate) fn error(&mut self, location: Location, message: impl Into<String>) {
        self.0.push(Diagnostic::new(location, message));
    }

    /// Returns `value` if no problem was reported, or every problem sorted by
    /// location otherwise.
    pub(crate) fn finish<T>(mut self, value: Option<T>) -> Result<T, Vec<Diagnostic>> {
        match value {
            Some(value) if self.0.is_empty() => Ok(value),
            _ => {
                debug_assert!(!self.0.is_empty(), "a read failed without a diagnostic");
                self.0.sort_by_key(|diagnostic| diagnostic.location);
                Err(self.0)
            }
        }
    }
}

fn marker_location(marker: &Marker) -> Location {
    Location {
        line: marker.line(),
        column: marker.col() + 1,
    }
}

pub(crate) fn location(node: &Node) -> Location {
    marker_location(&node.span.start)
}

/// Parses `source` as exactly one YAML document.
pub(crate) fn parse_document(source: &str) -> Result<Node<'_>, Vec<Diagnostic>> {
    let mut documents = MarkedYaml::load_from_str(source).map_err(|error| {
        vec![Diagnostic::new(
            marker_location(error.marker()),
            format!("invalid YAML: {}", error.info()),
        )]
    })?;
    match documents.len() {
        0 => Err(vec![Diagnostic::new(
            Location { line: 1, column: 1 },
            "the file contains no YAML document",
        )]),
        1 => Ok(documents.remove(0)),
        count => Err(vec![Diagnostic::new(
            location(&documents[1]),
            format!("expected a single YAML document, found {count}"),
        )]),
    }
}

/// Describes the kind of value a node holds, for error messages.
fn describe(node: &Node) -> &'static str {
    match &node.data {
        YamlData::Value(Scalar::Null) => "null",
        YamlData::Value(Scalar::Boolean(_)) => "a boolean",
        YamlData::Value(Scalar::Integer(_)) => "an integer",
        YamlData::Value(Scalar::FloatingPoint(_)) => "a floating-point number",
        YamlData::Value(Scalar::String(_)) => "a string",
        YamlData::Representation(..) => "a scalar",
        YamlData::Sequence(_) => "a sequence",
        YamlData::Mapping(_) => "a mapping",
        YamlData::Tagged(..) => "a tagged value",
        YamlData::Alias(_) => "an alias",
        YamlData::BadValue => "an invalid value",
    }
}

fn as_str<'a>(node: &'a Node) -> Option<&'a str> {
    match &node.data {
        YamlData::Value(Scalar::String(text)) => Some(text),
        _ => None,
    }
}

/// Reads a string. `what` names the value in error messages.
pub(crate) fn string<'a>(diags: &mut Diagnostics, node: &'a Node, what: &str) -> Option<&'a str> {
    let text = as_str(node);
    if text.is_none() {
        diags.error(
            location(node),
            format!("expected {what} to be a string, found {}", describe(node)),
        );
    }
    text
}

/// Reads a string that contains something other than whitespace.
pub(crate) fn non_empty_string<'a>(
    diags: &mut Diagnostics,
    node: &'a Node,
    what: &str,
) -> Option<&'a str> {
    let text = string(diags, node, what)?;
    if text.trim().is_empty() {
        diags.error(location(node), format!("{what} must not be empty"));
        return None;
    }
    Some(text)
}

/// Reads a word sequence such as `line item`.
pub(crate) fn words(diags: &mut Diagnostics, node: &Node, what: &str) -> Option<Words> {
    let text = string(diags, node, what)?;
    Words::parse(text)
        .map_err(|message| diags.error(location(node), format!("invalid {what}: {message}")))
        .ok()
}

/// Reads a sequence.
pub(crate) fn sequence<'a, 'input>(
    diags: &mut Diagnostics,
    node: &'a Node<'input>,
    what: &str,
) -> Option<&'a [Node<'input>]> {
    match &node.data {
        YamlData::Sequence(items) => Some(items),
        _ => {
            diags.error(
                location(node),
                format!("expected {what} to be a sequence, found {}", describe(node)),
            );
            None
        }
    }
}

/// Reads a sequence with at least one item.
pub(crate) fn non_empty_sequence<'a, 'input>(
    diags: &mut Diagnostics,
    node: &'a Node<'input>,
    what: &str,
) -> Option<&'a [Node<'input>]> {
    let items = sequence(diags, node, what)?;
    if items.is_empty() {
        diags.error(location(node), format!("{what} must not be empty"));
        return None;
    }
    Some(items)
}

/// Reads a mapping whose keys are strings, returning its entries in order.
pub(crate) fn entries<'a, 'input>(
    diags: &mut Diagnostics,
    node: &'a Node<'input>,
    what: &str,
) -> Option<Vec<(&'a str, &'a Node<'input>, &'a Node<'input>)>> {
    let YamlData::Mapping(mapping) = &node.data else {
        diags.error(
            location(node),
            format!("expected {what} to be a mapping, found {}", describe(node)),
        );
        return None;
    };
    let mut entries = Vec::with_capacity(mapping.len());
    for (key, value) in mapping {
        match as_str(key) {
            Some(name) => entries.push((name, key, value)),
            None => diags.error(
                location(key),
                format!(
                    "expected keys in {what} to be strings, found {}",
                    describe(key)
                ),
            ),
        }
    }
    Some(entries)
}

/// A mapping with a fixed set of allowed fields.
pub(crate) struct Fields<'a, 'input> {
    what: &'static str,
    location: Location,
    entries: Vec<(&'a str, &'a Node<'input>)>,
}

impl<'a, 'input> Fields<'a, 'input> {
    /// Reads `node` as a mapping, reporting any field not in `allowed`.
    pub(crate) fn new(
        diags: &mut Diagnostics,
        node: &'a Node<'input>,
        what: &'static str,
        allowed: &[&str],
    ) -> Option<Self> {
        let mut fields = Vec::new();
        for (name, key, value) in entries(diags, node, what)? {
            if allowed.contains(&name) {
                fields.push((name, value));
            } else {
                diags.error(
                    location(key),
                    format!(
                        "unknown field `{name}` in {what}, expected one of {}",
                        quoted_list(allowed)
                    ),
                );
            }
        }
        Some(Self {
            what,
            location: location(node),
            entries: fields,
        })
    }

    /// Returns the value of `field`, if present.
    pub(crate) fn optional(&self, field: &str) -> Option<&'a Node<'input>> {
        self.entries
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, value)| *value)
    }

    /// Returns the value of `field`, reporting it as missing if absent.
    pub(crate) fn required(
        &self,
        diags: &mut Diagnostics,
        field: &str,
    ) -> Option<&'a Node<'input>> {
        let value = self.optional(field);
        if value.is_none() {
            diags.error(
                self.location,
                format!("missing required field `{field}` in {}", self.what),
            );
        }
        value
    }
}

/// Reads an optional field with `read`.
///
/// Returns `Some(None)` if the field is absent and `None` if it is present
/// but could not be read.
pub(crate) fn optional<'a, 'input, T>(
    fields: &Fields<'a, 'input>,
    field: &str,
    read: impl FnOnce(&'a Node<'input>) -> Option<T>,
) -> Option<Option<T>> {
    match fields.optional(field) {
        None => Some(None),
        Some(node) => read(node).map(Some),
    }
}

/// Collects the results of reading several values, returning `None` if any
/// read failed.
///
/// Unlike collecting into `Option<Vec<_>>`, this reads every value even after
/// a failure, so that every problem is reported.
pub(crate) fn all<T>(results: impl IntoIterator<Item = Option<T>>) -> Option<Vec<T>> {
    let results: Vec<Option<T>> = results.into_iter().collect();
    results.into_iter().collect()
}

/// Formats names as a list of code spans: `a`, `b`, `c`.
pub(crate) fn quoted_list(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
