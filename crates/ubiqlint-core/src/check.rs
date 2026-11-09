//! Checking the symbols of a file against the rules that apply to it.
//!
//! An identifier is split into words by [`identifier::split`] and matched
//! against the patterns of every rule whose target and `include` globs apply
//! to the symbol. A `{verb}` element is bound to the term of the `{noun}`
//! element matched in the same identifier or, if there is none, to the term
//! of the innermost enclosing type, and may then only use a form of one of
//! that term's defined behaviors.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use crate::config::{Config, Rule, Vocabulary};
use crate::diagnostic::Diagnostic;
use crate::dictionary::{DICTIONARY_FILE_NAME, Dictionary};
use crate::identifier;
use crate::pattern::{Element, NounForm, Pattern, VerbForm};
use crate::symbol::{Name, Symbol, SymbolKind, Symbols};
use crate::words::Words;

/// Checks symbols against rules, using one dictionary and one configuration.
pub struct Checker<'a> {
    dictionary: &'a Dictionary,
    config: &'a Config,
    vocabularies: HashMap<&'a str, &'a Vocabulary>,
    /// The term each noun form belongs to, by form. Dictionary validation
    /// keeps the forms unique, so one map per form is enough.
    singular: HashMap<Vec<String>, usize>,
    plural: HashMap<Vec<String>, usize>,
}

/// Returns the placeholder a verb form is written as, such as `{verb:pp}`.
fn verb_placeholder(form: VerbForm) -> &'static str {
    match form {
        VerbForm::Base => "{verb}",
        VerbForm::S => "{verb:s}",
        VerbForm::Past => "{verb:past}",
        VerbForm::Pp => "{verb:pp}",
        VerbForm::Ing => "{verb:ing}",
    }
}

/// What a placeholder of a pattern matched.
enum Binding {
    /// A `{noun}` element and the term its words belong to.
    Noun { term: usize },
    /// A `{verb}` element, its form and the words it matched.
    Verb { form: VerbForm, span: Range<usize> },
}

impl<'a> Checker<'a> {
    /// Creates a checker for `dictionary` and `config`.
    pub fn new(dictionary: &'a Dictionary, config: &'a Config) -> Self {
        let vocabularies = config
            .vocabularies
            .iter()
            .map(|vocabulary| (vocabulary.name.as_str(), vocabulary))
            .collect();
        let mut singular = HashMap::new();
        let mut plural = HashMap::new();
        for (index, term) in dictionary.terms.iter().enumerate() {
            singular
                .entry(term.forms.singular.as_slice().to_vec())
                .or_insert(index);
            plural
                .entry(term.forms.plural.as_slice().to_vec())
                .or_insert(index);
        }
        Self {
            dictionary,
            config,
            vocabularies,
            singular,
            plural,
        }
    }

    /// Checks every symbol of `symbols` against the rules that apply to
    /// `path`, given relative to the directory of the configuration file.
    ///
    /// A symbol must match at least one pattern of every rule whose target
    /// and `include` globs apply to it.
    pub fn check(&self, symbols: &Symbols, path: &Path) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for symbol in symbols.iter() {
            if symbol.name.is_none() {
                continue;
            }
            for rule in &self.config.rules {
                if rule.target.matches(symbol.kind) && rule.includes(path) {
                    if let Some(diagnostic) = self.check_symbol(symbols, symbol, rule) {
                        diagnostics.push(diagnostic);
                    }
                }
            }
        }
        diagnostics
    }

    /// Checks one symbol against one rule, returning the violation if the
    /// identifier matches none of the rule's patterns.
    fn check_symbol(&self, symbols: &Symbols, symbol: &Symbol, rule: &Rule) -> Option<Diagnostic> {
        let name = symbol
            .name
            .as_ref()
            .expect("only named symbols are checked");
        // An identifier that splits into no words, such as `__`, can match
        // no pattern at all, so it is a violation whenever a rule applies.
        let words = identifier::split(&name.text);
        let enclosing = self.enclosing_term(symbols, symbol);
        for pattern in &rule.patterns {
            if self.attempt(pattern, &words, enclosing, false).is_some() {
                return None;
            }
        }
        Some(self.describe(symbols, symbol, rule, &words, enclosing))
    }

    /// Builds the violation for an identifier that matched no pattern,
    /// describing the nearest one: the pattern that matches once the verb
    /// checks are relaxed, with the fewest verb words, so that `updateOrder`
    /// is described against `{verb}{noun}` and not against `{verb}`.
    fn describe(
        &self,
        symbols: &Symbols,
        symbol: &Symbol,
        rule: &Rule,
        words: &[String],
        enclosing: Option<usize>,
    ) -> Diagnostic {
        let name = symbol
            .name
            .as_ref()
            .expect("only named symbols are checked");
        let mut nearest: Option<(&Pattern, Vec<Binding>, usize)> = None;
        for pattern in &rule.patterns {
            if let Some(bindings) = self.attempt(pattern, words, enclosing, true) {
                let verb_words = bindings
                    .iter()
                    .map(|binding| match binding {
                        Binding::Verb { span, .. } => span.len(),
                        Binding::Noun { .. } => 0,
                    })
                    .sum();
                if nearest
                    .as_ref()
                    .is_none_or(|&(_, _, best)| verb_words < best)
                {
                    nearest = Some((pattern, bindings, verb_words));
                }
            }
        }
        let message = match nearest {
            Some((pattern, bindings, _)) => {
                let term = self.bound_term(&bindings, enclosing);
                let failing_verb = term.and_then(|term| {
                    bindings.iter().find_map(|binding| match binding {
                        Binding::Verb { form, span } => {
                            (!self.term_allows(term, *form, &words[span.clone()]))
                                .then(|| (*form, span.clone()))
                        }
                        Binding::Noun { .. } => None,
                    })
                });
                match (term, failing_verb) {
                    (Some(term), Some((form, span))) => {
                        let verb_words = &words[span.clone()];
                        let verb = verb_words.join(" ");
                        let term_name = &self.dictionary.terms[term].name;
                        if self.term_has_behavior(term, verb_words) {
                            // The words are a defined behavior in another form,
                            // so say which forms this pattern expects instead.
                            let expected = self
                                .term_forms(term, form)
                                .iter()
                                .map(|form| format!("`{form}`"))
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!(
                                "`{}` does not match pattern `{}` of rule `{}`: `{verb}` is a \
                                 defined behavior of `{term_name}` in a different form, but `{}` \
                                 expects one of {expected}",
                                name.text,
                                pattern.source(),
                                rule.id,
                                verb_placeholder(form),
                            )
                        } else {
                            format!(
                                "`{}` does not match pattern `{}` of rule `{}`: `{verb}` is not \
                                 a defined behavior of `{term_name}` ({})",
                                name.text,
                                pattern.source(),
                                rule.id,
                                self.defined_behaviors(term),
                            )
                        }
                    }
                    (None, _) if bindings.iter().any(|b| matches!(b, Binding::Verb { .. })) => {
                        match self.enclosing_type_name(symbols, symbol) {
                            Some(enclosing_type) => format!(
                                "`{}` does not match pattern `{}` of rule `{}`: `{{verb}}` is \
                                 not bound to a term: the enclosing type `{enclosing_type}` is \
                                 not in the dictionary",
                                name.text,
                                pattern.source(),
                                rule.id,
                            ),
                            None => format!(
                                "`{}` does not match pattern `{}` of rule `{}`: `{{verb}}` is \
                                 not bound to a term: the identifier contains no `{{noun}}` and \
                                 there is no enclosing type",
                                name.text,
                                pattern.source(),
                                rule.id,
                            ),
                        }
                    }
                    _ => self.no_pattern_message(name, rule),
                }
            }
            None => self.no_pattern_message(name, rule),
        };
        Diagnostic {
            location: name.span.start,
            message,
        }
    }

    /// Tries to match `words` against `pattern`. When `relax_verbs` is set,
    /// `{verb}` elements match any words without being checked against the
    /// dictionary, so that the nearest pattern can be described. Returns the
    /// bindings of the first match.
    fn attempt(
        &self,
        pattern: &Pattern,
        words: &[String],
        enclosing: Option<usize>,
        relax_verbs: bool,
    ) -> Option<Vec<Binding>> {
        let mut bindings = Vec::new();
        if self.match_elements(
            pattern.elements(),
            words,
            0,
            enclosing,
            relax_verbs,
            &mut bindings,
        ) {
            Some(bindings)
        } else {
            None
        }
    }

    /// Matches `elements` against `words` from `pos` on. Variable-length
    /// elements try every length, since a `{noun}` such as `line item` spans
    /// several words, and backtrack on failure.
    fn match_elements(
        &self,
        elements: &[Element],
        words: &[String],
        pos: usize,
        enclosing: Option<usize>,
        relax_verbs: bool,
        bindings: &mut Vec<Binding>,
    ) -> bool {
        let Some((first, rest)) = elements.split_first() else {
            return pos == words.len()
                && self.verbs_allowed(bindings, words, enclosing, relax_verbs);
        };
        match first {
            Element::Literal(literal) => {
                let end = pos + literal.len();
                words.get(pos..end) == Some(literal.as_slice())
                    && self.match_elements(rest, words, end, enclosing, relax_verbs, bindings)
            }
            Element::Noun(form) => (pos + 1..=words.len()).any(|end| {
                self.term_for_form(*form, &words[pos..end])
                    .is_some_and(|term| {
                        bindings.push(Binding::Noun { term });
                        if self.match_elements(rest, words, end, enclosing, relax_verbs, bindings) {
                            true
                        } else {
                            bindings.pop();
                            false
                        }
                    })
            }),
            Element::Vocabulary(name) => {
                self.vocabularies
                    .get(name.as_str())
                    .is_some_and(|vocabulary| {
                        vocabulary.words.iter().any(|entry| {
                            let end = pos + entry.as_slice().len();
                            words.get(pos..end) == Some(entry.as_slice())
                                && self.match_elements(
                                    rest,
                                    words,
                                    end,
                                    enclosing,
                                    relax_verbs,
                                    bindings,
                                )
                        })
                    })
            }
            // `{any}` needs at least one word, so `get{any}` does not match
            // `get` alone.
            Element::Any => (pos + 1..=words.len())
                .any(|end| self.match_elements(rest, words, end, enclosing, relax_verbs, bindings)),
            Element::Verb(form) => {
                // In relaxed matching a base-form `{verb}` is kept to a single
                // word, so that a pattern of only `{verb}` cannot swallow the
                // whole identifier and produce a misleading message.
                let max_len = if relax_verbs && *form == VerbForm::Base {
                    1
                } else {
                    words.len() - pos
                };
                (1..=max_len).any(|len| {
                    let end = pos + len;
                    bindings.push(Binding::Verb {
                        form: *form,
                        span: pos..end,
                    });
                    if self.match_elements(rest, words, end, enclosing, relax_verbs, bindings) {
                        true
                    } else {
                        bindings.pop();
                        false
                    }
                })
            }
        }
    }

    /// Tells whether every `{verb}` binding is a form of a defined behavior
    /// of the bound term: the term of the `{noun}` matched in the same
    /// identifier or, if there is none, of the innermost enclosing type.
    fn verbs_allowed(
        &self,
        bindings: &[Binding],
        words: &[String],
        enclosing: Option<usize>,
        relax_verbs: bool,
    ) -> bool {
        if relax_verbs || !bindings.iter().any(|b| matches!(b, Binding::Verb { .. })) {
            return true;
        }
        let Some(term) = self.bound_term(bindings, enclosing) else {
            return false;
        };
        bindings.iter().all(|binding| match binding {
            Binding::Verb { form, span } => self.term_allows(term, *form, &words[span.clone()]),
            Binding::Noun { .. } => true,
        })
    }

    /// Returns the term the `{verb}` elements are bound to.
    fn bound_term(&self, bindings: &[Binding], enclosing: Option<usize>) -> Option<usize> {
        bindings
            .iter()
            .find_map(|binding| match binding {
                Binding::Noun { term } => Some(*term),
                Binding::Verb { .. } => None,
            })
            .or(enclosing)
    }

    /// Returns the term whose singular or plural form is `words`.
    fn term_for_form(&self, form: NounForm, words: &[String]) -> Option<usize> {
        match form {
            NounForm::Singular => self.singular.get(words).copied(),
            NounForm::Plural => self.plural.get(words).copied(),
        }
    }

    /// Tells whether `words` are any form of a behavior of `term`.
    fn term_has_behavior(&self, term: usize, words: &[String]) -> bool {
        self.dictionary.terms[term]
            .behaviors
            .iter()
            .any(|behavior| {
                [
                    &behavior.forms.base,
                    &behavior.forms.s,
                    &behavior.forms.past,
                    &behavior.forms.pp,
                    &behavior.forms.ing,
                ]
                .iter()
                .any(|form| form.as_slice() == words)
            })
    }

    /// Returns the distinct words of the given form of the behaviors of
    /// `term`, in definition order.
    fn term_forms(&self, term: usize, form: VerbForm) -> Vec<Words> {
        let mut forms: Vec<Words> = Vec::new();
        for behavior in &self.dictionary.terms[term].behaviors {
            let words = match form {
                VerbForm::Base => &behavior.forms.base,
                VerbForm::S => &behavior.forms.s,
                VerbForm::Past => &behavior.forms.past,
                VerbForm::Pp => &behavior.forms.pp,
                VerbForm::Ing => &behavior.forms.ing,
            };
            if !forms.contains(words) {
                forms.push(words.clone());
            }
        }
        forms
    }

    /// Tells whether `words` are the given form of a behavior of `term`.
    fn term_allows(&self, term: usize, form: VerbForm, words: &[String]) -> bool {
        self.dictionary.terms[term]
            .behaviors
            .iter()
            .any(|behavior| {
                let allowed = match form {
                    VerbForm::Base => &behavior.forms.base,
                    VerbForm::S => &behavior.forms.s,
                    VerbForm::Past => &behavior.forms.past,
                    VerbForm::Pp => &behavior.forms.pp,
                    VerbForm::Ing => &behavior.forms.ing,
                };
                allowed.as_slice() == words
            })
    }

    /// Returns the term of the innermost enclosing type, if it is one.
    fn enclosing_term(&self, symbols: &Symbols, symbol: &Symbol) -> Option<usize> {
        self.enclosing_type(symbols, symbol).and_then(|enclosing| {
            let name = enclosing.name.as_ref().expect("the type is named");
            let words = identifier::split(&name.text);
            self.term_for_form(NounForm::Singular, &words)
                .or_else(|| self.term_for_form(NounForm::Plural, &words))
        })
    }

    /// Returns the innermost enclosing type with a name.
    fn enclosing_type<'s>(&self, symbols: &'s Symbols, symbol: &'s Symbol) -> Option<&'s Symbol> {
        symbols
            .ancestors(symbol)
            .find(|ancestor| ancestor.kind == SymbolKind::Type && ancestor.name.is_some())
    }

    fn enclosing_type_name<'s>(&self, symbols: &'s Symbols, symbol: &'s Symbol) -> Option<&'s str> {
        self.enclosing_type(symbols, symbol)
            .and_then(|enclosing| enclosing.name.as_ref().map(|name| name.text.as_str()))
    }

    /// Lists the behaviors defined for a term, for violation messages.
    fn defined_behaviors(&self, term: usize) -> String {
        let behaviors: Vec<String> = self.dictionary.terms[term]
            .behaviors
            .iter()
            .map(|behavior| format!("`{}`", behavior.verb))
            .collect();
        if behaviors.is_empty() {
            "no behaviors are defined".to_owned()
        } else {
            format!("defined behaviors: {}", behaviors.join(", "))
        }
    }

    fn no_pattern_message(&self, name: &Name, rule: &Rule) -> String {
        let patterns: Vec<&str> = rule
            .patterns
            .iter()
            .map(|pattern| pattern.source())
            .collect();
        let head = format!(
            "`{}` does not match any pattern of rule `{}`: expected one of {}",
            name.text,
            rule.id,
            crate::yaml::quoted_list(&patterns),
        );
        // Only rules whose patterns speak about terms and behaviors can be
        // satisfied by growing the dictionary; `get{any}` cannot.
        let uses_dictionary = rule
            .patterns
            .iter()
            .flat_map(Pattern::elements)
            .any(|element| matches!(element, Element::Noun(_) | Element::Verb(_)));
        if uses_dictionary {
            format!(
                "{head}. If `{}` is a new term or behavior of the ubiquitous language, define \
                 it in `{DICTIONARY_FILE_NAME}` first",
                name.text
            )
        } else {
            head
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{Location, Span};

    const DICTIONARY: &str = "\
context: order
terms:
  - name: Order
    kind: entity
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place
        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }
      - verb: cancel
        forms: { base: cancel, s: cancels, past: canceled, pp: canceled, ing: canceling }
  - name: LineItem
    kind: value_object
    forms: { singular: line item, plural: line items }
";

    const CONFIG: &str = "\
vocabularies:
  role: [repository, factory]

rules:
  - id: domain-type
    include: [\"**/domain/**\"]
    target: type
    patterns:
      - \"{noun}\"
      - \"{noun}{role}\"
      - \"{noun}{verb:pp}\"

  - id: domain-method
    include: [\"**/domain/**\"]
    target: method
    patterns:
      - \"{verb}\"
      - \"{verb}{noun}\"

  - id: collection
    include: [\"**/domain/**\"]
    target: field
    patterns:
      - \"{noun:plural}\"

  - id: dto
    include: [\"**/dto/**\"]
    target: method
    patterns:
      - \"get{any}\"
      - \"set{any}\"
";

    const DOMAIN: &str = "src/main/java/com/example/order/domain/Order.java";
    const DTO: &str = "src/main/java/com/example/order/dto/OrderDto.java";

    fn setup() -> (Dictionary, Config) {
        let dictionary = Dictionary::from_yaml_str(DICTIONARY).unwrap();
        let config = Config::from_yaml_str(CONFIG).unwrap();
        (dictionary, config)
    }

    fn span(line: usize, column: usize) -> Span {
        Span {
            start: Location { line, column },
            end: Location {
                line,
                column: column + 1,
            },
        }
    }

    fn name(text: &str, line: usize, column: usize) -> Option<Name> {
        Some(Name {
            text: text.to_owned(),
            span: span(line, column),
        })
    }

    /// Builds the symbols of `class Order { ... }` with the given members.
    fn order_class(members: &[(SymbolKind, &str, usize)]) -> Symbols {
        let mut symbols = Symbols::new();
        let order = symbols.push(SymbolKind::Type, name("Order", 1, 13), span(1, 13), None);
        for (kind, text, line) in members {
            symbols.push(*kind, name(text, *line, 17), span(*line, 17), Some(order));
        }
        symbols
    }

    fn messages(diagnostics: &[Diagnostic]) -> Vec<String> {
        diagnostics.iter().map(|d| d.message.clone()).collect()
    }

    #[test]
    fn accepts_identifiers_that_match_a_pattern() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let mut symbols = order_class(&[
            (SymbolKind::Method, "place", 4),
            (SymbolKind::Method, "cancel", 7),
            (SymbolKind::Method, "placeOrder", 10),
            (SymbolKind::Field, "lineItems", 3),
        ]);
        symbols.push(
            SymbolKind::Type,
            name("OrderRepository", 20, 20),
            span(20, 20),
            None,
        );
        symbols.push(
            SymbolKind::Type,
            name("OrderPlaced", 30, 20),
            span(30, 20),
            None,
        );

        assert_eq!(checker.check(&symbols, Path::new(DOMAIN)), []);
    }

    #[test]
    fn accepts_getters_and_setters_in_dto_files() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let mut symbols = Symbols::new();
        let dto = symbols.push(SymbolKind::Type, name("OrderDto", 1, 13), span(1, 13), None);
        for (text, line) in [("getId", 4), ("setId", 8)] {
            symbols.push(
                SymbolKind::Method,
                name(text, line, 17),
                span(line, 17),
                Some(dto),
            );
        }

        assert_eq!(checker.check(&symbols, Path::new(DTO)), []);
    }

    #[test]
    fn reports_a_verb_that_is_not_a_defined_behavior() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let symbols = order_class(&[(SymbolKind::Method, "updateOrder", 7)]);

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].location,
            Location {
                line: 7,
                column: 17
            }
        );
        assert_eq!(
            diagnostics[0].message,
            "`updateOrder` does not match pattern `{verb}{noun}` of rule `domain-method`: \
             `update` is not a defined behavior of `Order` (defined behaviors: `place`, `cancel`)"
        );
    }

    #[test]
    fn describes_a_bare_verb_against_the_bare_verb_pattern() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let symbols = order_class(&[(SymbolKind::Method, "update", 7)]);

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            [
                "`update` does not match pattern `{verb}` of rule `domain-method`: \
              `update` is not a defined behavior of `Order` (defined behaviors: `place`, `cancel`)"
            ]
        );
    }

    #[test]
    fn binds_a_verb_to_the_noun_in_the_identifier_first() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        // `place` is a behavior of the enclosing `Order`, but the `{noun}` of
        // the identifier is `LineItem`, which defines no behaviors.
        let symbols = order_class(&[(SymbolKind::Method, "placeLineItem", 7)]);

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            [
                "`placeLineItem` does not match pattern `{verb}{noun}` of rule `domain-method`: \
              `place` is not a defined behavior of `LineItem` (no behaviors are defined)"
            ]
        );
    }

    #[test]
    fn binds_a_verb_to_the_enclosing_type_when_the_identifier_has_no_noun() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let mut symbols = Symbols::new();
        let helper = symbols.push(SymbolKind::Type, name("Helper", 1, 13), span(1, 13), None);
        symbols.push(
            SymbolKind::Method,
            name("run", 4, 17),
            span(4, 17),
            Some(helper),
        );

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            [
                "`Helper` does not match any pattern of rule `domain-type`: \
                 expected one of `{noun}`, `{noun}{role}`, `{noun}{verb:pp}`. \
                 If `Helper` is a new term or behavior of the ubiquitous language, \
                 define it in `terms.yaml` first",
                "`run` does not match pattern `{verb}` of rule `domain-method`: `{verb}` is \
                 not bound to a term: the enclosing type `Helper` is not in the dictionary",
            ]
        );
    }

    #[test]
    fn reports_an_unknown_noun_with_the_pattern_list() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let mut symbols = Symbols::new();
        symbols.push(
            SymbolKind::Type,
            name("PurchaseService", 3, 14),
            span(3, 14),
            None,
        );

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].location,
            Location {
                line: 3,
                column: 14
            }
        );
        assert_eq!(
            diagnostics[0].message,
            "`PurchaseService` does not match any pattern of rule `domain-type`: \
             expected one of `{noun}`, `{noun}{role}`, `{noun}{verb:pp}`. \
             If `PurchaseService` is a new term or behavior of the ubiquitous language, \
             define it in `terms.yaml` first"
        );
    }

    #[test]
    fn reports_a_pp_that_is_not_a_defined_behavior() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let mut symbols = Symbols::new();
        symbols.push(
            SymbolKind::Type,
            name("OrderUpdated", 3, 13),
            span(3, 13),
            None,
        );

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            [
                "`OrderUpdated` does not match pattern `{noun}{verb:pp}` of rule `domain-type`: \
              `updated` is not a defined behavior of `Order` (defined behaviors: `place`, `cancel`)"
            ]
        );
    }

    #[test]
    fn any_needs_at_least_one_word() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let mut symbols = Symbols::new();
        let dto = symbols.push(SymbolKind::Type, name("OrderDto", 1, 13), span(1, 13), None);
        symbols.push(
            SymbolKind::Method,
            name("get", 4, 17),
            span(4, 17),
            Some(dto),
        );

        let diagnostics = checker.check(&symbols, Path::new(DTO));
        // The dto rule's patterns mention no `{noun}` or `{verb}`, so the
        // message does not suggest growing the dictionary.
        assert_eq!(
            messages(&diagnostics),
            ["`get` does not match any pattern of rule `dto`: \
              expected one of `get{any}`, `set{any}`"]
        );
    }

    #[test]
    fn matches_multi_word_nouns_in_singular_and_plural() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let mut symbols = Symbols::new();
        symbols.push(SymbolKind::Type, name("LineItem", 3, 13), span(3, 13), None);
        let order = symbols.push(SymbolKind::Type, name("Order", 10, 13), span(10, 13), None);
        symbols.push(
            SymbolKind::Field,
            name("lineItems", 11, 20),
            span(11, 20),
            Some(order),
        );
        symbols.push(
            SymbolKind::Field,
            name("items", 12, 20),
            span(12, 20),
            Some(order),
        );

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].location,
            Location {
                line: 12,
                column: 20
            }
        );
        assert_eq!(
            diagnostics[0].message,
            "`items` does not match any pattern of rule `collection`: \
             expected one of `{noun:plural}`. \
             If `items` is a new term or behavior of the ubiquitous language, \
             define it in `terms.yaml` first"
        );
    }

    #[test]
    fn skips_symbols_and_files_no_rule_applies_to() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        let mut symbols = order_class(&[(SymbolKind::Method, "updateOrder", 7)]);
        // Constructors are never checked, and neither are unnamed types.
        let order = symbols.iter().next().unwrap().id;
        symbols.push(
            SymbolKind::Constructor,
            name("Order", 3, 17),
            span(3, 17),
            Some(order),
        );
        symbols.push(SymbolKind::Type, None, span(4, 1), Some(order));

        // A file outside every `include` glob is not checked at all.
        assert_eq!(
            checker.check(&symbols, Path::new("src/main/java/com/example/Order.java")),
            []
        );
        // In a domain file the constructor and the anonymous type are skipped.
        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("`updateOrder`"));
    }

    #[test]
    fn distinguishes_a_defined_behavior_in_a_different_form() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        // `OrderPlace` fails `{noun}{verb:pp}`, but `place` is defined: the
        // message must say the pp form `placed` is what the pattern expects.
        let mut symbols = Symbols::new();
        symbols.push(
            SymbolKind::Type,
            name("OrderPlace", 3, 13),
            span(3, 13),
            None,
        );
        // `cancelsOrder` fails `{verb}{noun}`: `cancels` is the `s` form.
        let order = symbols.push(SymbolKind::Type, name("Order", 10, 13), span(10, 13), None);
        symbols.push(
            SymbolKind::Method,
            name("cancelsOrder", 11, 17),
            span(11, 17),
            Some(order),
        );

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            [
                "`OrderPlace` does not match pattern `{noun}{verb:pp}` of rule `domain-type`: \
                 `place` is a defined behavior of `Order` in a different form, but `{verb:pp}` \
                 expects one of `placed`, `canceled`",
                "`cancelsOrder` does not match pattern `{verb}{noun}` of rule `domain-method`: \
                 `cancels` is a defined behavior of `Order` in a different form, but `{verb}` \
                 expects one of `place`, `cancel`",
            ]
        );
    }

    #[test]
    fn does_not_blame_the_whole_identifier_on_a_lone_verb_pattern() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        // Neither `placeOrders` nor `toString` is an attempt at a behavior;
        // the message must not claim `place orders` or `to string` is one.
        let symbols = order_class(&[
            (SymbolKind::Method, "placeOrders", 4),
            (SymbolKind::Method, "toString", 7),
        ]);

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            [
                "`placeOrders` does not match any pattern of rule `domain-method`: \
                 expected one of `{verb}`, `{verb}{noun}`. \
                 If `placeOrders` is a new term or behavior of the ubiquitous language, \
                 define it in `terms.yaml` first",
                "`toString` does not match any pattern of rule `domain-method`: \
                 expected one of `{verb}`, `{verb}{noun}`. \
                 If `toString` is a new term or behavior of the ubiquitous language, \
                 define it in `terms.yaml` first",
            ]
        );
    }

    #[test]
    fn reports_an_identifier_that_splits_into_no_words() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, &config);
        // `__` splits into no words, so no pattern can match it.
        let symbols = order_class(&[(SymbolKind::Field, "__", 4)]);

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].location,
            Location {
                line: 4,
                column: 17
            }
        );
        assert_eq!(
            diagnostics[0].message,
            "`__` does not match any pattern of rule `collection`: \
             expected one of `{noun:plural}`. \
             If `__` is a new term or behavior of the ubiquitous language, \
             define it in `terms.yaml` first"
        );
    }
}
