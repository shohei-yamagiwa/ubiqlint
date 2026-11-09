//! Dictionaries (`terms.yaml`): the terms of one bounded context and the
//! behaviors each term allows.

use std::collections::HashMap;

use crate::diagnostic::{Diagnostic, Location};
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
            items
                .iter()
                .filter_map(|item| read_term(diags, item))
                .collect::<Vec<_>>()
        });
    if let Some(terms) = &terms {
        check_unique_terms(diags, terms);
    }
    Some(Dictionary {
        context: context?.to_owned(),
        terms: terms?,
    })
}

fn read_term(diags: &mut Diagnostics, node: &Node) -> Option<Term> {
    let fields = Fields::new(
        diags,
        node,
        "a term",
        &["name", "kind", "ja", "description", "forms", "behaviors"],
    )?;
    let name = fields
        .required(diags, "name")
        .and_then(|node| read_term_name(diags, node));
    let kind = yaml::optional(&fields, "kind", |node| {
        yaml::non_empty_string(diags, node, "`kind`")
    });
    let ja = yaml::optional(&fields, "ja", |node| {
        yaml::non_empty_string(diags, node, "`ja`")
    });
    let description = yaml::optional(&fields, "description", |node| {
        yaml::non_empty_string(diags, node, "`description`")
    });
    let forms = fields
        .required(diags, "forms")
        .and_then(|node| read_noun_forms(diags, node));
    let behaviors = yaml::optional(&fields, "behaviors", |node| {
        yaml::sequence(diags, node, "`behaviors`").map(|items| {
            items
                .iter()
                .filter_map(|item| read_behavior(diags, item))
                .collect::<Vec<_>>()
        })
    });
    let name = name?;
    let behaviors = behaviors?.unwrap_or_default();
    check_unique_behaviors(diags, name, &behaviors);
    Some(Term {
        name: name.to_owned(),
        kind: kind?.map(str::to_owned),
        ja: ja?.map(str::to_owned),
        description: description?.map(str::to_owned),
        forms: forms?,
        behaviors,
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

fn read_noun_forms(diags: &mut Diagnostics, node: &Node) -> Option<NounForms> {
    let fields = Fields::new(diags, node, "`forms` of a term", &["singular", "plural"])?;
    let singular = fields
        .required(diags, "singular")
        .and_then(|node| yaml::words(diags, node, "`singular`"));
    let plural = fields
        .required(diags, "plural")
        .and_then(|node| yaml::words(diags, node, "`plural`"));
    Some(NounForms {
        singular: singular?,
        plural: plural?,
    })
}

fn read_behavior(diags: &mut Diagnostics, node: &Node) -> Option<Behavior> {
    let fields = Fields::new(diags, node, "a behavior", &["verb", "forms"])?;
    let verb = fields
        .required(diags, "verb")
        .and_then(|node| yaml::words(diags, node, "`verb`"));
    let forms = fields
        .required(diags, "forms")
        .and_then(|node| read_verb_forms(diags, node));
    let (verb, forms) = (verb?, forms?);
    if forms.base != verb {
        diags.error(
            yaml::location(node),
            format!(
                "`forms.base` (`{}`) must be the same as `verb` (`{verb}`)",
                forms.base
            ),
        );
    }
    Some(Behavior {
        verb,
        forms,
        location: yaml::location(node),
    })
}

fn read_verb_forms(diags: &mut Diagnostics, node: &Node) -> Option<VerbForms> {
    let fields = Fields::new(
        diags,
        node,
        "`forms` of a behavior",
        &["base", "s", "past", "pp", "ing"],
    )?;
    let mut form = |name: &str| {
        fields
            .required(diags, name)
            .and_then(|node| yaml::words(diags, node, &format!("`{name}`")))
    };
    let base = form("base");
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

/// Reports terms whose names or noun forms clash with an earlier term.
fn check_unique_terms(diags: &mut Diagnostics, terms: &[Term]) {
    let mut names: HashMap<String, &Term> = HashMap::new();
    let mut forms: HashMap<&Words, &Term> = HashMap::new();
    for term in terms {
        if let Some(first) = names.get(&term.name.to_ascii_lowercase()) {
            diags.error(
                term.location,
                format!(
                    "duplicate term `{}` (first defined at line {})",
                    term.name, first.location.line
                ),
            );
            continue;
        }
        names.insert(term.name.to_ascii_lowercase(), term);

        let mut own_forms = vec![&term.forms.singular];
        if term.forms.plural != term.forms.singular {
            own_forms.push(&term.forms.plural);
        }
        for form in own_forms {
            match forms.get(form) {
                Some(other) => diags.error(
                    term.location,
                    format!(
                        "the form `{form}` of term `{}` is already used by term `{}` (line {})",
                        term.name, other.name, other.location.line
                    ),
                ),
                None => {
                    forms.insert(form, term);
                }
            }
        }
    }
}

/// Reports behaviors defined more than once on the same term.
fn check_unique_behaviors(diags: &mut Diagnostics, term: &str, behaviors: &[Behavior]) {
    let mut verbs: HashMap<&Words, &Behavior> = HashMap::new();
    for behavior in behaviors {
        match verbs.get(&behavior.verb) {
            Some(first) => diags.error(
                behavior.location,
                format!(
                    "duplicate behavior `{}` on term `{term}` (first defined at line {})",
                    behavior.verb, first.location.line
                ),
            ),
            None => {
                verbs.insert(&behavior.verb, behavior);
            }
        }
    }
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
            ["6:9: `forms.base` (`placing`) must be the same as `verb` (`place`)"]
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
    forms: { singular: purchase, plural: purchases }
  - name: Purchase
    forms: { singular: purchase order, plural: orders }
";
        assert_eq!(
            errors(source),
            [
                "8:9: duplicate behavior `place` on term `Order` (first defined at line 6)",
                "10:5: duplicate term `order` (first defined at line 3)",
                "12:5: the form `orders` of term `Purchase` is already used by term `Order` (line 3)",
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
