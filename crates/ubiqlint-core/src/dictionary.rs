//! Dictionaries (`terms.yaml`): the terms of one bounded context and the
//! behaviors each term allows.

use std::collections::HashMap;

use crate::diagnostic::Diagnostic;
use crate::identifier;
use crate::location::Location;
use crate::pattern::VerbForm;
use crate::words::Words;
use crate::yaml::{self, Diagnostics, Fields, Node};

/// The file name of a dictionary.
pub const DICTIONARY_FILE_NAME: &str = "terms.yaml";

/// The terms of one bounded context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dictionary {
    /// The name of the bounded context.
    pub context: String,
    pub terms: Vec<Term>,
}

/// A noun of the ubiquitous language, such as `Order`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    /// The canonical spelling, such as `LineItem`.
    pub name: String,
    /// The kind of term, such as `entity` or `value_object`.
    pub kind: Option<String>,
    /// A Japanese label, such as `注文`.
    pub ja: Option<String>,
    pub description: Option<String>,
    pub forms: NounForms,
    /// The only behaviors allowed on this term.
    pub behaviors: Vec<Behavior>,
    pub location: Location,
}

/// The forms of a noun.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NounForms {
    pub singular: Words,
    pub plural: Words,
}

/// A verb allowed on a term, such as `place` on `Order`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Behavior {
    pub verb: Words,
    pub forms: VerbForms,
    pub location: Location,
}

/// The forms of a verb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerbForms {
    /// The base form, such as `place`.
    pub base: Words,
    /// The third-person singular present, such as `places`.
    pub s: Words,
    /// The past tense, such as `placed`.
    pub past: Words,
    /// The past participle, such as `placed`.
    pub pp: Words,
    /// The present participle, such as `placing`.
    pub ing: Words,
}

impl VerbForms {
    /// Returns the words of `form`.
    pub fn get(&self, form: VerbForm) -> &Words {
        match form {
            VerbForm::Base => &self.base,
            VerbForm::S => &self.s,
            VerbForm::Past => &self.past,
            VerbForm::Pp => &self.pp,
            VerbForm::Ing => &self.ing,
        }
    }

    /// Returns every form's words, in the order of [`VerbForm::ALL`].
    pub fn all(&self) -> [&Words; 5] {
        [&self.base, &self.s, &self.past, &self.pp, &self.ing]
    }
}

impl Dictionary {
    /// Reads a dictionary from YAML, reporting every problem found.
    pub fn from_yaml_str(source: &str) -> Result<Self, Vec<Diagnostic>> {
        let root = yaml::parse_document(source)?;
        let mut diags = Diagnostics::default();
        let dictionary = read_dictionary(&mut diags, &root);
        diags.finish(dictionary)
    }
}

fn read_dictionary(diags: &mut Diagnostics, node: &Node) -> Option<Dictionary> {
    let fields = Fields::new(diags, node, "the dictionary", &["context", "terms"])?;
    let context = fields
        .required(diags, "context")
        .and_then(|node| yaml::non_empty_string(diags, node, "`context`"));
    let terms = fields
        .required(diags, "terms")
        .and_then(|node| yaml::sequence(diags, node, "`terms`"))
        .map(|items| {
            let mut seen = Seen::default();
            items
                .iter()
                .filter_map(|item| read_term(diags, item, &mut seen))
                .collect::<Vec<_>>()
        });
    Some(Dictionary {
        context: context?.to_owned(),
        terms: terms?,
    })
}

/// Term names and noun forms read so far.
///
/// Each value is registered as soon as it is read, so that a clash is
/// reported even when other fields of the same term are invalid.
#[derive(Default)]
struct Seen {
    /// Lowercased term names, with the line that first defined each.
    names: HashMap<String, usize>,
    /// Noun forms, with the name of the term that uses each (if valid) and
    /// the line it appears on.
    forms: HashMap<Words, (Option<String>, usize)>,
}

impl Seen {
    /// Registers a term name, returning `false` if an earlier term already
    /// has it.
    fn add_name(&mut self, diags: &mut Diagnostics, name: &str, location: Location) -> bool {
        let key = name.to_ascii_lowercase();
        if let Some(line) = self.names.get(&key) {
            diags.error(
                location,
                format!("duplicate term `{name}` (first defined at line {line})"),
            );
            return false;
        }
        self.names.insert(key, location.line);
        true
    }

    /// Registers the noun forms of one term, reporting any form an earlier
    /// term already uses.
    fn add_forms(
        &mut self,
        diags: &mut Diagnostics,
        owner: Option<&str>,
        forms: [Option<&(Words, Location)>; 2],
    ) {
        let mut own: Vec<&Words> = Vec::new();
        for (form, location) in forms.into_iter().flatten() {
            // A term may use the same words for its singular and plural, as
            // `data` does.
            if own.contains(&form) {
                continue;
            }
            own.push(form);
            match self.forms.get(form) {
                Some((other, line)) => {
                    let other = other.as_ref().map_or_else(
                        || "another term".to_owned(),
                        |name| format!("term `{name}`"),
                    );
                    diags.error(
                        *location,
                        format!("the form `{form}` is already used by {other} (line {line})"),
                    );
                }
                None => {
                    self.forms
                        .insert(form.clone(), (owner.map(str::to_owned), location.line));
                }
            }
        }
    }
}

fn read_term(diags: &mut Diagnostics, node: &Node, seen: &mut Seen) -> Option<Term> {
    let fields = Fields::new(
        diags,
        node,
        "a term",
        &["name", "kind", "ja", "description", "forms", "behaviors"],
    )?;
    let name_node = fields.required(diags, "name");
    let name = name_node.and_then(|node| read_term_name(diags, node));
    // A term that repeats an earlier name is reported once, for its name,
    // rather than again for each of its forms.
    let is_new_name = match (name, name_node) {
        (Some(name), Some(node)) => seen.add_name(diags, name, yaml::location(node)),
        _ => true,
    };
    let kind = yaml::optional(&fields, "kind", |node| {
        yaml::non_empty_string(diags, node, "`kind`")
    });
    let ja = yaml::optional(&fields, "ja", |node| {
        yaml::non_empty_string(diags, node, "`ja`")
    });
    let description = yaml::optional(&fields, "description", |node| {
        yaml::non_empty_string(diags, node, "`description`")
    });
    let forms = fields.required(diags, "forms").and_then(|node| {
        let (singular, plural) = read_noun_forms(diags, node)?;
        if let (Some(name), Some((singular, location))) = (name, &singular) {
            let words = identifier::split(name);
            if singular.as_slice() != words.as_slice() {
                diags.error(
                    *location,
                    format!(
                        "`singular` (`{singular}`) must be the words of `name` \
                         (`{name}` splits into `{}`)",
                        words.join(" ")
                    ),
                );
            }
        }
        if is_new_name {
            seen.add_forms(diags, name, [singular.as_ref(), plural.as_ref()]);
        }
        Some(NounForms {
            singular: singular?.0,
            plural: plural?.0,
        })
    });
    let behaviors = yaml::optional(&fields, "behaviors", |node| {
        yaml::sequence(diags, node, "`behaviors`").map(|items| {
            let mut verbs = HashMap::new();
            items
                .iter()
                .filter_map(|item| read_behavior(diags, item, &mut verbs))
                .collect::<Vec<_>>()
        })
    });
    Some(Term {
        name: name?.to_owned(),
        kind: kind?.map(str::to_owned),
        ja: ja?.map(str::to_owned),
        description: description?.map(str::to_owned),
        forms: forms?,
        behaviors: behaviors?.unwrap_or_default(),
        location: yaml::location(node),
    })
}

fn read_term_name<'a>(diags: &mut Diagnostics, node: &'a Node) -> Option<&'a str> {
    let name = yaml::string(diags, node, "`name`")?;
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric());
    if !valid {
        diags.error(
            yaml::location(node),
            format!(
                "invalid term name `{name}`: use ASCII letters and digits, \
                 starting with a letter (e.g. `LineItem`)"
            ),
        );
        return None;
    }
    Some(name)
}

type LocatedWords = (Words, Location);

/// Reads the singular and plural forms, each with its location. Returns
/// `None` only if `node` is not a mapping.
fn read_noun_forms(
    diags: &mut Diagnostics,
    node: &Node,
) -> Option<(Option<LocatedWords>, Option<LocatedWords>)> {
    let fields = Fields::new(diags, node, "`forms` of a term", &["singular", "plural"])?;
    let mut form = |name: &str| {
        fields.required(diags, name).and_then(|node| {
            yaml::words(diags, node, &format!("`{name}`"))
                .map(|words| (words, yaml::location(node)))
        })
    };
    let singular = form("singular");
    let plural = form("plural");
    Some((singular, plural))
}

/// Reads a behavior. `verbs` holds the verbs already defined on the same
/// term, with the line of each.
fn read_behavior(
    diags: &mut Diagnostics,
    node: &Node,
    verbs: &mut HashMap<Words, usize>,
) -> Option<Behavior> {
    let fields = Fields::new(diags, node, "a behavior", &["verb", "forms"])?;
    let verb = fields.required(diags, "verb").and_then(|node| {
        let verb = yaml::words(diags, node, "`verb`")?;
        let location = yaml::location(node);
        match verbs.get(&verb) {
            Some(line) => diags.error(
                location,
                format!("duplicate behavior `{verb}` (first defined at line {line})"),
            ),
            None => {
                verbs.insert(verb.clone(), location.line);
            }
        }
        Some(verb)
    });
    let forms = fields
        .required(diags, "forms")
        .and_then(|node| read_verb_forms(diags, node, verb.as_ref()));
    Some(Behavior {
        verb: verb?,
        forms: forms?,
        location: yaml::location(node),
    })
}

/// Reads the forms of a verb, reporting a base form that differs from
/// `verb` when both could be read.
fn read_verb_forms(
    diags: &mut Diagnostics,
    node: &Node,
    verb: Option<&Words>,
) -> Option<VerbForms> {
    let fields = Fields::new(
        diags,
        node,
        "`forms` of a behavior",
        &["base", "s", "past", "pp", "ing"],
    )?;
    let base = fields.required(diags, "base").and_then(|node| {
        let base = yaml::words(diags, node, "`base`")?;
        if let Some(verb) = verb.filter(|verb| **verb != base) {
            diags.error(
                yaml::location(node),
                format!("`forms.base` (`{base}`) must be the same as `verb` (`{verb}`)"),
            );
        }
        Some(base)
    });
    let mut form = |name: &str| {
        fields
            .required(diags, name)
            .and_then(|node| yaml::words(diags, node, &format!("`{name}`")))
    };
    let s = form("s");
    let past = form("past");
    let pp = form("pp");
    let ing = form("ing");
    Some(VerbForms {
        base: base?,
        s: s?,
        past: past?,
        pp: pp?,
        ing: ing?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Formats the diagnostics for `source` as `line:column: message`.
    fn errors(source: &str) -> Vec<String> {
        Dictionary::from_yaml_str(source)
            .expect_err("the dictionary should be invalid")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    const ORDER: &str = "\
context: order
terms:
  - name: Order
    kind: entity
    ja: 注文
    description: 顧客が確定させた購入の単位
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place
        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }
      - verb: cancel
        forms: { base: cancel, s: cancels, past: canceled, pp: canceled, ing: canceling }
  - name: LineItem
    forms: { singular: line item, plural: line items }
";

    fn words(text: &str) -> Words {
        Words::parse(text).unwrap()
    }

    #[test]
    fn reads_a_valid_dictionary() {
        let dictionary = Dictionary::from_yaml_str(ORDER).unwrap();
        assert_eq!(dictionary.context, "order");
        assert_eq!(dictionary.terms.len(), 2);

        let order = &dictionary.terms[0];
        assert_eq!(order.name, "Order");
        assert_eq!(order.kind.as_deref(), Some("entity"));
        assert_eq!(order.ja.as_deref(), Some("注文"));
        assert_eq!(order.forms.plural, words("orders"));
        assert_eq!(order.location, Location { line: 3, column: 5 });

        let verbs: Vec<String> = order.behaviors.iter().map(|b| b.verb.to_string()).collect();
        assert_eq!(verbs, ["place", "cancel"]);
        assert_eq!(order.behaviors[1].forms.pp, words("canceled"));

        let line_item = &dictionary.terms[1];
        assert_eq!(line_item.forms.singular, words("line item"));
        assert_eq!(line_item.kind, None);
        assert!(line_item.behaviors.is_empty());
    }

    #[test]
    fn reports_unknown_fields() {
        let source = "\
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
    forbidden_verbs: { delete: cancel }
";
        assert_eq!(
            errors(source),
            [
                "5:5: unknown field `forbidden_verbs` in a term, expected one of \
              `name`, `kind`, `ja`, `description`, `forms`, `behaviors`"
            ]
        );
    }

    #[test]
    fn reports_missing_fields() {
        let source = "\
context: order
terms:
  - name: Order
";
        assert_eq!(
            errors(source),
            ["3:5: missing required field `forms` in a term"]
        );
    }

    #[test]
    fn reports_values_of_the_wrong_type() {
        let source = "\
context: [order]
terms: Order
";
        assert_eq!(
            errors(source),
            [
                "1:10: expected `context` to be a string, found a sequence",
                "2:8: expected `terms` to be a sequence, found a string",
            ]
        );
    }

    #[test]
    fn reports_invalid_names_and_forms() {
        let source = "\
context: order
terms:
  - name: Line Item
    forms: { singular: LineItem, plural: line items }
";
        assert_eq!(
            errors(source),
            [
                "3:11: invalid term name `Line Item`: use ASCII letters and digits, \
                 starting with a letter (e.g. `LineItem`)",
                "4:24: invalid `singular`: `LineItem` is not a valid word sequence: \
                 use lowercase letters and digits, with a single space between words \
                 (e.g. `line item`)",
            ]
        );
    }

    #[test]
    fn reports_a_base_form_that_differs_from_the_verb() {
        let source = "\
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place
        forms: { base: placing, s: places, past: placed, pp: placed, ing: placing }
";
        assert_eq!(
            errors(source),
            ["7:24: `forms.base` (`placing`) must be the same as `verb` (`place`)"]
        );
    }

    #[test]
    fn reports_missing_verb_forms() {
        let source = "\
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place
        forms: { base: place, s: places, past: placed }
";
        assert_eq!(
            errors(source),
            [
                "7:16: missing required field `pp` in `forms` of a behavior",
                "7:16: missing required field `ing` in `forms` of a behavior",
            ]
        );
    }

    #[test]
    fn reports_duplicate_terms_forms_and_behaviors() {
        let source = "\
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place
        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }
      - verb: place
        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }
  - name: order
    forms: { singular: order, plural: orders }
  - name: Purchase
    forms: { singular: purchase, plural: orders }
";
        assert_eq!(
            errors(source),
            [
                "8:15: duplicate behavior `place` (first defined at line 6)",
                "10:11: duplicate term `order` (first defined at line 3)",
                "13:42: the form `orders` is already used by term `Order` (line 4)",
            ]
        );
    }

    #[test]
    fn reports_a_singular_form_that_differs_from_the_name() {
        let source = "\
context: order
terms:
  - name: LineItem
    forms: { singular: line items, plural: line items }
  - name: SKU
    forms: { singular: sku, plural: skus }
  - name: ShippingAddress
    forms: { singular: shipping address, plural: shipping addresses }
";
        assert_eq!(
            errors(source),
            [
                "4:24: `singular` (`line items`) must be the words of `name` \
              (`LineItem` splits into `line item`)"
            ]
        );
    }

    #[test]
    fn reports_clashes_in_terms_with_other_invalid_fields() {
        let source = "\
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
  - name: Order
    forms: { singular: order }
  - name: Purchase
    kind: ''
    forms: { singular: purchase, plural: orders }
";
        assert_eq!(
            errors(source),
            [
                "5:11: duplicate term `Order` (first defined at line 3)",
                "6:12: missing required field `plural` in `forms` of a term",
                "8:11: `kind` must not be empty",
                "9:42: the form `orders` is already used by term `Order` (line 4)",
            ]
        );
    }

    #[test]
    fn reports_clashes_in_behaviors_with_incomplete_forms() {
        let source = "\
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place
        forms: { base: placing, s: places, past: placed, ing: placing }
      - verb: place
        forms: { base: place }
";
        assert_eq!(
            errors(source),
            [
                "7:16: missing required field `pp` in `forms` of a behavior",
                "7:24: `forms.base` (`placing`) must be the same as `verb` (`place`)",
                "8:15: duplicate behavior `place` (first defined at line 6)",
                "9:16: missing required field `s` in `forms` of a behavior",
                "9:16: missing required field `past` in `forms` of a behavior",
                "9:16: missing required field `pp` in `forms` of a behavior",
                "9:16: missing required field `ing` in `forms` of a behavior",
            ]
        );
    }

    #[test]
    fn allows_identical_singular_and_plural_forms() {
        let source = "\
context: catalog
terms:
  - name: Data
    forms: { singular: data, plural: data }
";
        assert!(Dictionary::from_yaml_str(source).is_ok());
    }

    #[test]
    fn reports_every_problem_at_once() {
        let source = "\
context: order
terms:
  - name: Order
    forms: { singular: order }
  - name: Shipment
    forms: { singular: shipment, plural: Shipments }
";
        assert_eq!(
            errors(source),
            [
                "4:12: missing required field `plural` in `forms` of a term",
                "6:42: invalid `plural`: `Shipments` is not a valid word sequence: \
                 use lowercase letters and digits, with a single space between words \
                 (e.g. `line item`)",
            ]
        );
    }

    #[test]
    fn reports_yaml_errors() {
        assert_eq!(
            errors("context: order\ncontext: shipping\n"),
            ["2:1: invalid YAML: duplicated key in mapping"]
        );
        assert_eq!(
            errors("terms: [\n"),
            ["2:1: invalid YAML: while parsing a node, did not find expected node content"]
        );
        assert_eq!(
            errors("# nothing here\n"),
            ["1:1: the file contains no YAML document"]
        );
        assert_eq!(
            errors("context: order\n---\ncontext: shipping\n"),
            ["3:1: expected a single YAML document, found 2"]
        );
    }
}
