//! Splicing changes into the YAML source of a dictionary.
//!
//! Each [`Step`] parses the current text, finds the nodes it changes and
//! replaces one scalar, or inserts or removes whole lines, leaving every
//! other byte of the file as it was. New lines follow the indentation of
//! their neighbors and the file's line endings. A layout that cannot be
//! changed this way, such as a flow sequence of terms, is refused rather
//! than reformatted.
//!
//! The source has already been read as a valid dictionary, so the shape of
//! the tree is known; unexpected shapes are reported as layout problems.

use saphyr::{LoadableYamlNode, MarkedYaml, Scalar, YamlData};

use crate::dictionary::{self, VerbForms};
use crate::edit::NewTerm;

type Node<'s> = MarkedYaml<'s>;

/// A term, or a behavior of a term, by its position in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Item {
    Term(usize),
    Behavior(usize, usize),
}

/// One change to the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Sets the scalar at `path` within an item, such as `forms.plural`.
    /// A missing field directly under a term is inserted.
    Set {
        item: Item,
        path: &'static [&'static str],
        value: String,
    },
    /// Removes a field of a term.
    Unset {
        term: usize,
        key: &'static str,
    },
    AppendTerm(NewTerm),
    RemoveTerm(usize),
    AppendBehavior {
        term: usize,
        forms: VerbForms,
    },
    RemoveBehavior {
        term: usize,
        behavior: usize,
    },
}

/// The fields of a term, in the order the schema lists them. New fields
/// are inserted in this order.
const TERM_FIELDS: [&str; 6] = ["name", "kind", "ja", "description", "forms", "behaviors"];

/// Applies one step to `source`, returning the new text or why the step
/// cannot be applied in place.
pub(super) fn apply(source: &str, step: &Step) -> Result<String, String> {
    let doc = Doc::parse(source)?;
    match step {
        Step::Set { item, path, value } => doc.set(*item, path, value),
        Step::Unset { term, key } => doc.unset(*term, key),
        Step::AppendTerm(term) => doc.append_term(term),
        Step::RemoveTerm(term) => doc.remove_term(*term),
        Step::AppendBehavior { term, forms } => doc.append_behavior(*term, forms),
        Step::RemoveBehavior { term, behavior } => doc.remove_behavior(*term, *behavior),
    }
}

/// A parsed dictionary source.
struct Doc<'s> {
    source: &'s str,
    root: Node<'s>,
    /// The byte offset of each character, followed by the length of the
    /// source. The parser counts positions in characters.
    offsets: Vec<usize>,
    /// The line ending used by the file.
    newline: &'static str,
}

/// How the `forms` of a term or behavior are written.
#[derive(Clone, Copy)]
enum FormsStyle {
    /// `{ singular: order, plural: orders }`, or without the inner spaces.
    Flow { padded: bool },
    /// One field per line, indented by `indent` from `forms`.
    Block { indent: usize },
}

impl<'s> Doc<'s> {
    fn parse(source: &'s str) -> Result<Self, String> {
        let mut documents =
            MarkedYaml::load_from_str(source).map_err(|error| format!("invalid YAML: {error}"))?;
        if documents.len() != 1 {
            return Err("expected a single YAML document".to_owned());
        }
        let offsets = source
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([source.len()])
            .collect();
        let newline = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        Ok(Self {
            source,
            root: documents.remove(0),
            offsets,
            newline,
        })
    }

    // Positions.

    fn start(&self, node: &Node) -> usize {
        self.offsets[node.span.start.index()]
    }

    fn end(&self, node: &Node) -> usize {
        self.offsets[node.span.end.index()]
    }

    /// The offset at which the line containing `offset` starts.
    fn line_start(&self, offset: usize) -> usize {
        self.source[..offset]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }

    /// The offset of the line ending after `offset`, or the end of the
    /// source on the last line.
    fn line_end(&self, offset: usize) -> usize {
        match self.source[offset..].find('\n') {
            Some(index) => {
                let end = offset + index;
                if end > offset && self.source.as_bytes()[end - 1] == b'\r' {
                    end - 1
                } else {
                    end
                }
            }
            None => self.source.len(),
        }
    }

    /// The offset at which the line after the one containing `offset`
    /// starts, or the end of the source on the last line.
    fn next_line(&self, offset: usize) -> usize {
        self.source[offset..]
            .find('\n')
            .map_or(self.source.len(), |index| offset + index + 1)
    }

    /// The column of `offset`, which must only have spaces and dashes
    /// before it on its line.
    fn column(&self, offset: usize) -> Result<usize, String> {
        let start = self.line_start(offset);
        if self.source[start..offset]
            .bytes()
            .all(|b| b == b' ' || b == b'-')
        {
            Ok(offset - start)
        } else {
            Err("a definition does not start its own line".to_owned())
        }
    }

    /// Tells whether only spaces come before `offset` on its line.
    fn starts_line(&self, offset: usize) -> bool {
        let start = self.line_start(offset);
        self.source[start..offset].bytes().all(|b| b == b' ')
    }

    /// Checks that only whitespace or a comment follows `offset` on its
    /// line.
    fn ends_line(&self, offset: usize) -> Result<(), String> {
        let rest = self.source[offset..self.line_end(offset)].trim_start_matches([' ', '\t']);
        if rest.is_empty() || rest.starts_with('#') {
            Ok(())
        } else {
            Err("a definition shares its line with another".to_owned())
        }
    }

    /// Tells whether the line starting at `start` holds only whitespace.
    fn is_blank_line(&self, start: usize) -> bool {
        start < self.source.len()
            && self.source[start..self.line_end(start)]
                .trim_matches([' ', '\t'])
                .is_empty()
    }

    /// Tells whether `node` is a flow collection, written in brackets. A
    /// flow collection's span runs from its opening bracket to its closing
    /// one, while a block collection's ends where the next token starts.
    fn is_flow(&self, node: &Node) -> bool {
        let bytes = self.source.as_bytes();
        let close = match (&node.data, bytes.get(self.start(node))) {
            (YamlData::Sequence(_), Some(b'[')) => b']',
            (YamlData::Mapping(_), Some(b'{')) => b'}',
            _ => return false,
        };
        bytes.get(self.end(node)) == Some(&close)
    }

    /// The offset just after the last character of `node`, not counting
    /// trailing comments or blank lines.
    fn content_end(&self, node: &Node) -> Result<usize, String> {
        if self.is_flow(node) {
            return Ok(self.end(node) + 1);
        }
        let mut end = self.start(node);
        match &node.data {
            YamlData::Sequence(items) => {
                for item in items {
                    end = end.max(self.content_end(item)?);
                }
            }
            YamlData::Mapping(entries) => {
                for (key, value) in entries {
                    end = end
                        .max(self.content_end(key)?)
                        .max(self.content_end(value)?);
                }
            }
            // The span of a block scalar runs up to the next token, past
            // its trailing line breaks.
            _ => end += self.source[end..self.end(node)].trim_end().len(),
        }
        Ok(end)
    }

    // The tree.

    fn entry<'a>(&self, map: &'a Node<'s>, key: &str) -> Option<(&'a Node<'s>, &'a Node<'s>)> {
        let YamlData::Mapping(entries) = &map.data else {
            return None;
        };
        entries.iter().find(
            |(name, _)| matches!(&name.data, YamlData::Value(Scalar::String(text)) if text == key),
        )
    }

    fn terms(&self) -> Result<(&Node<'s>, &Node<'s>), String> {
        self.entry(&self.root, "terms")
            .ok_or_else(|| unexpected("`terms`"))
    }

    fn items<'a>(&self, sequence: &'a Node<'s>) -> Result<&'a [Node<'s>], String> {
        match &sequence.data {
            YamlData::Sequence(items) => Ok(items),
            _ => Err(unexpected("a sequence")),
        }
    }

    fn term(&self, index: usize) -> Result<&Node<'s>, String> {
        let (_, terms) = self.terms()?;
        self.items(terms)?
            .get(index)
            .ok_or_else(|| unexpected("a term"))
    }

    fn behavior(&self, term: usize, index: usize) -> Result<&Node<'s>, String> {
        let (_, behaviors) = self
            .entry(self.term(term)?, "behaviors")
            .ok_or_else(|| unexpected("`behaviors`"))?;
        self.items(behaviors)?
            .get(index)
            .ok_or_else(|| unexpected("a behavior"))
    }

    // Reading the layout.

    /// The offset of the `-` that starts a block sequence item.
    fn dash(&self, item: &Node) -> Result<usize, String> {
        let before = self.source[..self.start(item)].trim_end_matches([' ', '\t', '\r', '\n']);
        match before.len().checked_sub(1) {
            Some(dash) if before.as_bytes()[dash] == b'-' && self.starts_line(dash) => Ok(dash),
            _ => Err("a sequence item does not start with `-` on its own line".to_owned()),
        }
    }

    /// The columns of an item's `-` and of the keys of its mapping.
    fn item_columns(&self, item: &Node) -> Result<(usize, usize), String> {
        let dash = self.dash(item)?;
        let dash_column = self.column(dash)?;
        let key_column = if matches!(item.data, YamlData::Mapping(_)) && !self.is_flow(item) {
            self.column(self.start(item))?
        } else {
            dash_column + 2
        };
        Ok((dash_column, key_column.max(dash_column + 2)))
    }

    /// How far a sequence's `-` is indented from the key that holds it, and
    /// how far the keys of its items are indented from the `-`, taken from
    /// the terms so that new behaviors look like them.
    fn sequence_indents(&self) -> Result<(usize, usize), String> {
        let (key, terms) = self.terms()?;
        let key_column = self.column(self.start(key))?;
        match self.items(terms)?.first() {
            Some(first) if !self.is_flow(terms) => {
                let (dash, item) = self.item_columns(first)?;
                Ok((dash.saturating_sub(key_column), item - dash))
            }
            _ => Ok((2, 2)),
        }
    }

    /// The style of the first `forms` in the file, so that new forms look
    /// like the existing ones.
    fn forms_style(&self) -> FormsStyle {
        let model = self
            .terms()
            .ok()
            .and_then(|(_, terms)| self.items(terms).ok()?.first())
            .and_then(|term| self.entry(term, "forms"));
        let Some((key, forms)) = model else {
            return FormsStyle::Flow { padded: true };
        };
        if self.is_flow(forms) {
            let after = self.source.as_bytes().get(self.start(forms) + 1);
            return FormsStyle::Flow {
                padded: after == Some(&b' '),
            };
        }
        let first = match &forms.data {
            YamlData::Mapping(entries) => entries.iter().next().map(|(key, _)| key),
            _ => None,
        };
        let indent = first
            .and_then(|first| {
                let first = self.column(self.start(first)).ok()?;
                let key = self.column(self.start(key)).ok()?;
                first.checked_sub(key).filter(|indent| *indent > 0)
            })
            .unwrap_or(2);
        FormsStyle::Block { indent }
    }

    /// The range of the value of `key` to replace: from a block scalar's
    /// `|` or `>` indicator, or the scalar itself, to its last character.
    fn value_range(&self, key: &Node, value: &Node) -> Result<(usize, usize), String> {
        let bytes = self.source.as_bytes();
        let skip_spaces = |mut offset: usize| {
            while matches!(bytes.get(offset), Some(b' ' | b'\t')) {
                offset += 1;
            }
            offset
        };
        let colon = skip_spaces(self.end(key));
        if bytes.get(colon) != Some(&b':') {
            return Err("a key is not followed by `:`".to_owned());
        }
        let after = skip_spaces(colon + 1);
        let start = if matches!(bytes.get(after), Some(b'|' | b'>')) {
            after
        } else {
            self.start(value)
        };
        Ok((start, self.content_end(value)?))
    }

    // Writing.

    fn splice(&self, start: usize, end: usize, text: &str) -> String {
        let mut result = String::with_capacity(self.source.len() + text.len());
        result.push_str(&self.source[..start]);
        result.push_str(text);
        result.push_str(&self.source[end..]);
        result
    }

    /// Inserts `lines` as new lines after the line containing `offset`.
    fn insert_lines_after(&self, offset: usize, lines: &[String]) -> String {
        let at = self.line_end(offset);
        let mut text = String::new();
        for line in lines {
            text.push_str(self.newline);
            text.push_str(line);
        }
        self.splice(at, at, &text)
    }

    /// Removes the whole lines from the one containing `start` to the one
    /// containing `end`. A blank line that would be left next to another
    /// is removed too.
    fn remove_lines(&self, start: usize, end: usize) -> String {
        let start = self.line_start(start);
        let mut end = self.next_line(end);
        if start > 0 && self.is_blank_line(self.line_start(start - 1)) && self.is_blank_line(end) {
            end = self.next_line(end);
        }
        self.splice(start, end, "")
    }

    // Steps.

    fn item(&self, item: Item) -> Result<&Node<'s>, String> {
        match item {
            Item::Term(term) => self.term(term),
            Item::Behavior(term, behavior) => self.behavior(term, behavior),
        }
    }

    fn set(&self, item: Item, path: &[&str], value: &str) -> Result<String, String> {
        let (last, parents) = path.split_last().expect("a path names a field");
        let mut map = self.item(item)?;
        for parent in parents {
            let (_, nested) = self
                .entry(map, parent)
                .ok_or_else(|| unexpected(&format!("`{parent}`")))?;
            map = nested;
        }
        match self.entry(map, last) {
            Some((key, old)) => {
                let (start, end) = self.value_range(key, old)?;
                Ok(self.splice(start, end, &scalar(value)))
            }
            None if parents.is_empty() && matches!(item, Item::Term(_)) => {
                let line = format!("{last}: {}", scalar(value));
                self.insert_field(map, last, |column| vec![indent(column, &line)])
            }
            None => Err(unexpected(&format!("`{last}`"))),
        }
    }

    /// Inserts a field into the block mapping of a term, after the fields
    /// that come before it in [`TERM_FIELDS`]. `lines` renders the field for
    /// the column of the term's keys.
    fn insert_field(
        &self,
        term: &Node<'s>,
        field: &str,
        lines: impl FnOnce(usize) -> Vec<String>,
    ) -> Result<String, String> {
        if self.is_flow(term) {
            return Err(format!(
                "`{field}` would go in a term written in flow style (`{{ ... }}`)"
            ));
        }
        let YamlData::Mapping(entries) = &term.data else {
            return Err(unexpected("a term"));
        };
        let position = |name: &str| TERM_FIELDS.iter().position(|field| *field == name);
        let before = position(field).expect("a known field");
        let anchor = entries
            .iter()
            .filter(|(key, _)| {
                matches!(&key.data, YamlData::Value(Scalar::String(name))
                    if position(name).is_some_and(|index| index < before))
            })
            .max_by_key(|(key, _)| self.start(key))
            .map(|(_, value)| value)
            .ok_or_else(|| unexpected("`name`"))?;
        let end = self.content_end(anchor)?;
        self.ends_line(end)?;
        let column = self.column(self.start(term))?;
        Ok(self.insert_lines_after(end, &lines(column)))
    }

    fn unset(&self, term: usize, key: &str) -> Result<String, String> {
        let map = self.term(term)?;
        let (key_node, value) = self
            .entry(map, key)
            .ok_or_else(|| unexpected(&format!("`{key}`")))?;
        self.remove_field(key, key_node, value)
    }

    /// Removes a field of a block mapping, with its lines.
    fn remove_field(&self, name: &str, key: &Node, value: &Node) -> Result<String, String> {
        let start = self.start(key);
        if !self.starts_line(start) {
            return Err(format!("`{name}` does not start its own line"));
        }
        let end = self.content_end(value)?;
        self.ends_line(end)?;
        Ok(self.remove_lines(start, end))
    }

    fn append_term(&self, term: &NewTerm) -> Result<String, String> {
        if self.is_flow(&self.root) {
            return Err("the dictionary is written in flow style (`{ ... }`)".to_owned());
        }
        let (key, terms) = self.terms()?;
        let (seq_indent, item_indent) = self.sequence_indents()?;
        let singular = dictionary::singular_of(&term.name)?;
        let mut fields = vec![format!("name: {}", scalar(&term.name))];
        for (field, value) in [
            ("kind", &term.kind),
            ("ja", &term.ja),
            ("description", &term.description),
        ] {
            if let Some(value) = value {
                fields.push(format!("{field}: {}", scalar(value)));
            }
        }
        let forms = [
            ("singular", singular.to_string()),
            ("plural", term.plural.to_string()),
        ];
        let style = self.forms_style();
        self.append_item(key, terms, "terms", seq_indent, item_indent, |key_column| {
            let mut lines: Vec<String> = fields.iter().map(|f| indent(key_column, f)).collect();
            lines.extend(forms_lines(style, key_column, &forms));
            lines
        })
    }

    fn append_behavior(&self, term: usize, forms: &VerbForms) -> Result<String, String> {
        let map = self.term(term)?;
        let (seq_indent, item_indent) = self.sequence_indents()?;
        let style = self.forms_style();
        let verb = format!("verb: {}", scalar(&forms.base.to_string()));
        let names = ["base", "s", "past", "pp", "ing"];
        let forms: Vec<(&str, String)> = names
            .into_iter()
            .zip(forms.all())
            .map(|(name, words)| (name, words.to_string()))
            .collect();
        let item = |key_column: usize| {
            let mut lines = vec![indent(key_column, &verb)];
            lines.extend(forms_lines(style, key_column, &forms));
            lines
        };
        match self.entry(map, "behaviors") {
            Some((key, behaviors)) => {
                self.append_item(key, behaviors, "behaviors", seq_indent, item_indent, item)
            }
            None => self.insert_field(map, "behaviors", |column| {
                let dash = column + seq_indent;
                let mut lines = vec![indent(column, "behaviors:")];
                lines.extend(item_lines(dash, dash + item_indent, item));
                lines
            }),
        }
    }

    /// Appends an item after the last one in the block sequence `sequence`,
    /// the value of `key`. An empty `[]` becomes a block sequence indented
    /// by `seq_indent`, with keys `item_indent` further in. `item` renders
    /// the fields of the item for the column of its keys.
    fn append_item(
        &self,
        key: &Node,
        sequence: &Node,
        name: &str,
        seq_indent: usize,
        item_indent: usize,
        item: impl FnOnce(usize) -> Vec<String>,
    ) -> Result<String, String> {
        let items = self.items(sequence)?;
        if self.is_flow(sequence) {
            if !items.is_empty() {
                return Err(format!("`{name}` is written in flow style (`[ ... ]`)"));
            }
            // `terms: [] # comment` becomes `terms: # comment` followed by
            // the new item.
            let key_start = self.start(key);
            if !self.starts_line(key_start) {
                return Err(format!("`{name}` does not start its own line"));
            }
            let dash = self.column(key_start)? + seq_indent;
            let close = self.end(sequence) + 1;
            let line_end = self.line_end(close);
            let mut text = format!(":{}", &self.source[close..line_end]);
            for line in item_lines(dash, dash + item_indent, item) {
                text.push_str(self.newline);
                text.push_str(&line);
            }
            return Ok(self.splice(self.end(key), line_end, &text));
        }
        let last = items.last().ok_or_else(|| unexpected("an item"))?;
        let (dash, key_column) = self.item_columns(last)?;
        let end = self.content_end(last)?;
        self.ends_line(end)?;
        let mut lines = Vec::new();
        // Keep items apart with a blank line if the existing ones are.
        if items.len() > 1 {
            let dash_line = self.line_start(self.dash(last)?);
            if dash_line > 0 && self.is_blank_line(self.line_start(dash_line - 1)) {
                lines.push(String::new());
            }
        }
        lines.extend(item_lines(dash, key_column, item));
        Ok(self.insert_lines_after(end, &lines))
    }

    fn remove_term(&self, term: usize) -> Result<String, String> {
        let (key, terms) = self.terms()?;
        self.remove_item(key, terms, "terms", term, |doc, key, terms| {
            // The last term leaves an empty sequence, as `terms` is required.
            let end = doc.line_end(doc.content_end(terms)?);
            Ok(doc.splice(doc.end(key), end, ": []"))
        })
    }

    fn remove_behavior(&self, term: usize, behavior: usize) -> Result<String, String> {
        let (key, behaviors) = self
            .entry(self.term(term)?, "behaviors")
            .ok_or_else(|| unexpected("`behaviors`"))?;
        self.remove_item(
            key,
            behaviors,
            "behaviors",
            behavior,
            |doc, key, behaviors| {
                // The last behavior takes the optional `behaviors` with it.
                doc.remove_field("behaviors", key, behaviors)
            },
        )
    }

    /// Removes the item at `index` from the block sequence `sequence`, the
    /// value of `key`, with its lines. Removing the only item is left to
    /// `remove_last`.
    fn remove_item(
        &self,
        key: &Node,
        sequence: &Node,
        name: &str,
        index: usize,
        remove_last: impl FnOnce(&Self, &Node, &Node) -> Result<String, String>,
    ) -> Result<String, String> {
        let items = self.items(sequence)?;
        if self.is_flow(sequence) {
            return Err(format!("`{name}` is written in flow style (`[ ... ]`)"));
        }
        if items.len() == 1 {
            return remove_last(self, key, sequence);
        }
        let item = items.get(index).ok_or_else(|| unexpected("an item"))?;
        let dash = self.dash(item)?;
        let end = self.content_end(item)?;
        self.ends_line(end)?;
        Ok(self.remove_lines(dash, end))
    }
}

/// Renders the lines of a sequence item with its `-` at `dash` and its keys
/// at `key_column`.
fn item_lines(
    dash: usize,
    key_column: usize,
    item: impl FnOnce(usize) -> Vec<String>,
) -> Vec<String> {
    let mut lines = item(key_column);
    if let Some(first) = lines.first_mut() {
        // The first field shares the line with the `-`.
        let field = first.trim_start().to_owned();
        *first = format!(
            "{}-{}{field}",
            " ".repeat(dash),
            " ".repeat(key_column - dash - 1)
        );
    }
    lines
}

/// Renders `forms` as a `forms` field at `column`.
fn forms_lines(style: FormsStyle, column: usize, forms: &[(&str, String)]) -> Vec<String> {
    match style {
        FormsStyle::Flow { padded } => {
            let fields: Vec<String> = forms
                .iter()
                .map(|(name, words)| format!("{name}: {}", scalar(words)))
                .collect();
            let fields = fields.join(", ");
            let line = if padded {
                format!("forms: {{ {fields} }}")
            } else {
                format!("forms: {{{fields}}}")
            };
            vec![indent(column, &line)]
        }
        FormsStyle::Block { indent: step } => {
            let mut lines = vec![indent(column, "forms:")];
            lines.extend(
                forms.iter().map(|(name, words)| {
                    indent(column + step, &format!("{name}: {}", scalar(words)))
                }),
            );
            lines
        }
    }
}

fn indent(column: usize, text: &str) -> String {
    format!("{}{text}", " ".repeat(column))
}

fn unexpected(what: &str) -> String {
    format!("cannot find {what} in the file")
}

/// Renders `text` as a YAML scalar that reads back as the same string in
/// both block and flow context: as is when that is safe, or double-quoted.
pub(super) fn scalar(text: &str) -> String {
    if reads_as_itself(&format!("key: {text}"), text)
        && reads_as_itself(&format!("{{key: {text}}}"), text)
    {
        text.to_owned()
    } else {
        double_quoted(text)
    }
}

/// Tells whether `document`, a mapping with the single key `key`, maps it
/// to the string `text`.
fn reads_as_itself(document: &str, text: &str) -> bool {
    let Ok(documents) = MarkedYaml::load_from_str(document) else {
        return false;
    };
    let [root] = documents.as_slice() else {
        return false;
    };
    let YamlData::Mapping(entries) = &root.data else {
        return false;
    };
    entries.len() == 1
        && entries.iter().all(|(_, value)| {
            matches!(&value.data, YamlData::Value(Scalar::String(value)) if value == text)
        })
}

fn double_quoted(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c if c.is_control()
                || matches!(
                    c,
                    '\u{2028}' | '\u{2029}' | '\u{feff}' | '\u{fffe}' | '\u{ffff}'
                ) =>
            {
                quoted.push_str(&format!("\\u{:04X}", u32::from(c)));
            }
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}
