//! Checking declarations and imports against the rules that apply to a file.
//!
//! An identifier is split into words by [`identifier::split`] and matched
//! against the patterns of every rule whose target and `include` globs apply
//! to the symbol. A `{verb}` element is currently bound to the term of the first
//! `{noun}` element matched in the same identifier or, if there is none, to the term
//! of the innermost enclosing type, and may then only use a form of one of
//! that term's defined behaviors. The policy for identifiers with multiple
//! nouns is provisional; see section 8 of `docs/plan.md`.
//!
//! Imports from external namespaces may not introduce a dictionary term's
//! canonical name or use a wildcard. Internal namespace prefixes and the
//! paths to protect are explicitly configured in import rules.

use std::collections::HashSet;
use std::path::Path;

use crate::config::{Config, Rule};
use crate::diagnostic::Diagnostic;
use crate::dictionary::Dictionary;
use crate::identifier;
use crate::matching::{MatchMode, Matcher};
use crate::pattern::{Element, NounForm, Pattern, VerbForm};
use crate::source::{Import, SourceFile};
use crate::symbol::{Name, SymbolKind, Symbols};

/// Checks declarations and imports using one dictionary and one configuration.
pub struct Checker<'a> {
    config: &'a Config,
    dictionary_path: &'a Path,
    matcher: Matcher<'a>,
    /// Canonical type spellings, compared exactly rather than split as words.
    term_names: HashSet<&'a str>,
}

/// Prepared once per symbol and shared by all its applicable rules.
struct Identifier<'a> {
    name: &'a Name,
    words: Vec<String>,
    enclosing_type: Option<&'a str>,
    enclosing_term: Option<usize>,
}

/// Returns the placeholder a verb form is written as, such as `{verb:pp}`.
fn verb_placeholder(form: VerbForm) -> String {
    if form == VerbForm::Base {
        "{verb}".to_owned()
    } else {
        format!("{{verb:{}}}", form.name())
    }
}

impl<'a> Checker<'a> {
    /// Creates a checker for `dictionary` and `config`. `dictionary_path`
    /// identifies the definition file in diagnostics and editing hints.
    pub fn new(dictionary: &'a Dictionary, dictionary_path: &'a Path, config: &'a Config) -> Self {
        Self {
            config,
            dictionary_path,
            matcher: Matcher::new(dictionary, config),
            term_names: dictionary
                .terms
                .iter()
                .map(|term| term.name.as_str())
                .collect(),
        }
    }

    /// Checks declarations and imports, returning diagnostics in source order.
    /// Paths are relative to the configuration file's directory. An excluded
    /// file is never checked.
    pub fn check_file(&self, file: &SourceFile, path: &Path) -> Vec<Diagnostic> {
        if self.config.excludes().matches(path) {
            return Vec::new();
        }
        let mut diagnostics = self.check(&file.symbols, path);
        diagnostics.extend(self.check_imports(&file.imports, path));
        diagnostics.sort_by_key(|diagnostic| diagnostic.location);
        diagnostics
    }

    /// Checks imports against each boundary rule that includes the file.
    fn check_imports(&self, imports: &[Import], path: &Path) -> Vec<Diagnostic> {
        let rules: Vec<_> = self
            .config
            .import_rules
            .iter()
            .filter(|rule| rule.includes(path))
            .collect();
        let mut diagnostics = Vec::new();
        if rules.is_empty() {
            return diagnostics;
        }
        for import in imports {
            let qualified = import.path.join(".");
            for rule in &rules {
                if rule.is_internal(import.namespace()) {
                    continue;
                }
                let message = if import.wildcard {
                    format!(
                        "external wildcard import `{qualified}.*` is forbidden by rule `{}`: \
                         import non-conflicting names explicitly, or use fully qualified names",
                        rule.id
                    )
                } else if let Some(name) =
                    import.name().filter(|name| self.term_names.contains(name))
                {
                    format!(
                        "external import `{qualified}` conflicts with dictionary term `{name}` \
                         in `{}` under rule `{}`: use the fully qualified name `{qualified}` instead",
                        self.dictionary_path.display(),
                        rule.id
                    )
                } else {
                    continue;
                };
                diagnostics.push(Diagnostic {
                    location: import.span.start,
                    message,
                    span: Some(import.span),
                    rule_id: Some(rule.id.clone()),
                });
            }
        }
        diagnostics
    }

    /// Checks every symbol of `symbols` against the rules that apply to
    /// `path`, given relative to the directory of the configuration file.
    ///
    /// A symbol must match at least one pattern of every rule whose target
    /// and `include` globs apply to it. The diagnostics are sorted by
    /// location, since symbols are registered in parse order, which is not
    /// always source order.
    pub fn check(&self, symbols: &Symbols, path: &Path) -> Vec<Diagnostic> {
        // Match the path against the globs once per file, not once per
        // symbol and rule.
        let applicable: Vec<&Rule> = self
            .config
            .rules
            .iter()
            .filter(|rule| rule.includes(path))
            .collect();
        let mut diagnostics = Vec::new();
        for symbol in symbols.iter() {
            let Some(name) = &symbol.name else {
                continue;
            };
            let mut rules = applicable
                .iter()
                .filter(|rule| rule.target.matches(symbol.kind))
                .peekable();
            if rules.peek().is_none() {
                continue;
            }
            // Anonymous classes are skipped, but an unknown named type
            // must not fall back to an outer type's term.
            let enclosing_type = symbols
                .ancestors(symbol)
                .filter(|ancestor| ancestor.kind == SymbolKind::Type)
                .find_map(|ancestor| ancestor.name.as_ref())
                .map(|name| name.text.as_str());
            let enclosing_term = enclosing_type.and_then(|name| {
                let words = identifier::split(name);
                self.matcher
                    .term_for_form(NounForm::Singular, &words)
                    .or_else(|| self.matcher.term_for_form(NounForm::Plural, &words))
            });
            let identifier = Identifier {
                name,
                words: identifier::split(&name.text),
                enclosing_type,
                enclosing_term,
            };
            for rule in rules {
                if let Some(diagnostic) = self.check_symbol(&identifier, rule) {
                    diagnostics.push(diagnostic);
                }
            }
        }
        diagnostics.sort_by_key(|diagnostic| diagnostic.location);
        diagnostics
    }

    /// Checks one symbol against one rule, returning the violation if the
    /// identifier matches none of the rule's patterns.
    fn check_symbol(&self, identifier: &Identifier, rule: &Rule) -> Option<Diagnostic> {
        let words = &identifier.words;
        if !words.is_empty()
            && rule.patterns.iter().any(|pattern| {
                self.matcher
                    .attempt(pattern, words, identifier.enclosing_term, MatchMode::Strict)
                    .is_some()
            })
        {
            return None;
        }
        let message = if words.is_empty() {
            // An identifier without words cannot match or become a term.
            format!(
                "`{}` contains no words, so it cannot match any pattern of rule `{}`",
                identifier.name.text, rule.id
            )
        } else {
            self.describe(identifier, rule)
        };
        Some(Diagnostic {
            location: identifier.name.span.start,
            message,
            span: Some(identifier.name.span),
            rule_id: Some(rule.id.clone()),
        })
    }

    /// Describes the nearest relaxed match, preferring the fewest verb words.
    /// Ties keep pattern order, just as strict matching does.
    fn describe(&self, identifier: &Identifier, rule: &Rule) -> String {
        let name = identifier.name;
        let words = &identifier.words;
        let nearest = rule
            .patterns
            .iter()
            .filter_map(|pattern| {
                self.matcher
                    .attempt(
                        pattern,
                        words,
                        identifier.enclosing_term,
                        MatchMode::Relaxed,
                    )
                    .map(|bindings| (pattern, bindings))
            })
            .min_by_key(|(_, bindings)| bindings.verb_words());
        let Some((pattern, bindings)) = nearest else {
            return self.no_pattern_message(name, rule);
        };
        let prefix = format!(
            "`{}` does not match pattern `{}` of rule `{}`",
            name.text,
            pattern.source(),
            rule.id
        );
        let term = bindings.bound_term(identifier.enclosing_term);
        let failing_verb = term.and_then(|term| {
            bindings
                .verbs()
                .find(|(form, span)| !self.matcher.term_allows(term, *form, &words[span.clone()]))
        });
        match (term, failing_verb) {
            (Some(term), Some((form, span))) => {
                let verb_words = &words[span];
                let verb = verb_words.join(" ");
                let term_name = &self.matcher.term(term).name;
                if let Some(expected) = self.matcher.term_behavior_form(term, verb_words, form) {
                    format!(
                        "{prefix}: `{verb}` is a defined behavior of `{term_name}` in a \
                         different form, but `{}` expects `{expected}`",
                        verb_placeholder(form),
                    )
                } else {
                    format!(
                        "{prefix}: `{verb}` is not a defined behavior of `{term_name}` ({})",
                        self.defined_behaviors(term),
                    )
                }
            }
            (None, _) if bindings.verbs().next().is_some() => {
                let reason = match identifier.enclosing_type {
                    Some(enclosing_type) => {
                        format!(
                            "the enclosing type `{enclosing_type}` is not in the dictionary `{}`",
                            self.dictionary_path.display()
                        )
                    }
                    None => "the identifier contains no `{noun}` and there is no enclosing type"
                        .to_owned(),
                };
                format!("{prefix}: `{{verb}}` is not bound to a term: {reason}")
            }
            _ => self.no_pattern_message(name, rule),
        }
    }

    /// Lists the behaviors defined for a term, for violation messages.
    fn defined_behaviors(&self, term: usize) -> String {
        let behaviors: Vec<String> = self
            .matcher
            .term(term)
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
                 it in `{}` first",
                name.text,
                self.dictionary_path.display()
            )
        } else {
            head
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::DICTIONARY_FILE_NAME;
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
                 not bound to a term: the enclosing type `Helper` is not in the dictionary `terms.yaml`",
            ]
        );
    }

    #[test]
    fn reports_an_unknown_noun_with_the_pattern_list() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
                 expects `placed`",
                "`cancelsOrder` does not match pattern `{verb}{noun}` of rule `domain-method`: \
                 `cancels` is a defined behavior of `Order` in a different form, but `{verb}` \
                 expects `cancel`",
            ]
        );
    }

    #[test]
    fn does_not_blame_the_whole_identifier_on_a_lone_verb_pattern() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
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
            "`__` contains no words, so it cannot match any pattern of rule `collection`"
        );
    }

    #[test]
    fn sorts_diagnostics_by_location() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
        // Registered out of source order: line 10 before line 4.
        let symbols = order_class(&[
            (SymbolKind::Method, "updateOrder", 10),
            (SymbolKind::Method, "update", 4),
        ]);

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        let lines: Vec<usize> = diagnostics.iter().map(|d| d.location.line).collect();
        assert_eq!(lines, [4, 10]);
    }

    #[test]
    fn skips_anonymous_classes_when_binding_the_enclosing_type() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
        let mut symbols = Symbols::new();
        let order = symbols.push(SymbolKind::Type, name("Order", 1, 13), span(1, 13), None);
        let anonymous = symbols.push(SymbolKind::Type, None, span(3, 20), Some(order));
        symbols.push(
            SymbolKind::Method,
            name("update", 5, 17),
            span(5, 17),
            Some(anonymous),
        );

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
    fn a_verb_past_the_end_of_the_words_does_not_overflow() {
        // A pattern with two verbs after the noun, checked against a
        // one-word identifier: the remaining length must not go negative
        // (debug panic / release hang).
        let (dictionary, _) = setup();
        let config = Config::from_yaml_str(
            "rules:\n  - id: event\n    include: [\"**/domain/**\"]\n    target: type\n    patterns: [\"{noun}{verb}{verb:pp}\"]\n",
        )
        .unwrap();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
        let mut symbols = Symbols::new();
        symbols.push(SymbolKind::Type, name("Order", 1, 13), span(1, 13), None);

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            ["`Order` does not match any pattern of rule `event`: \
              expected one of `{noun}{verb}{verb:pp}`. \
              If `Order` is a new term or behavior of the ubiquitous language, \
              define it in `terms.yaml` first"]
        );
    }

    /// A dictionary with a multi-word behavior, `check out`.
    const CHECKOUT_DICTIONARY: &str = "\
context: order
terms:
  - name: Order
    kind: entity
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place
        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }
      - verb: check out
        forms: { base: check out, s: checks out, past: checked out, pp: checked out, ing: checking out }
";

    const VERB_CONFIG: &str = "\
rules:
  - id: domain-type
    include: [\"**/domain/**\"]
    target: type
    patterns:
      - \"{noun}\"
      - \"{noun}{verb:pp}\"

  - id: domain-method
    include: [\"**/domain/**\"]
    target: method
    patterns:
      - \"{verb}\"
      - \"{verb}{noun}\"
";

    #[test]
    fn relaxed_matching_does_not_swallow_unrelated_words() {
        let dictionary = Dictionary::from_yaml_str(CHECKOUT_DICTIONARY).unwrap();
        let config = Config::from_yaml_str(VERB_CONFIG).unwrap();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
        let mut symbols = Symbols::new();
        symbols.push(
            SymbolKind::Type,
            name("OrderShippingAddress", 3, 13),
            span(3, 13),
            None,
        );

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            [
                "`OrderShippingAddress` does not match any pattern of rule `domain-type`: \
              expected one of `{noun}`, `{noun}{verb:pp}`. \
              If `OrderShippingAddress` is a new term or behavior of the ubiquitous language, \
              define it in `terms.yaml` first"
            ]
        );
    }

    #[test]
    fn multi_word_behaviors_get_form_mismatch_messages() {
        let dictionary = Dictionary::from_yaml_str(CHECKOUT_DICTIONARY).unwrap();
        let config = Config::from_yaml_str(VERB_CONFIG).unwrap();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
        let symbols = order_class(&[
            (SymbolKind::Method, "checksOut", 4),
            (SymbolKind::Method, "checkedOutOrder", 7),
        ]);

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            [
                "`checksOut` does not match pattern `{verb}` of rule `domain-method`: \
                 `checks out` is a defined behavior of `Order` in a different form, but `{verb}` \
                 expects `check out`",
                "`checkedOutOrder` does not match pattern `{verb}{noun}` of rule `domain-method`: \
                 `checked out` is a defined behavior of `Order` in a different form, but `{verb}` \
                 expects `check out`",
            ]
        );
    }

    #[test]
    fn does_not_skip_a_named_type_that_is_not_a_term() {
        let (dictionary, config) = setup();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
        let mut symbols = Symbols::new();
        let order = symbols.push(SymbolKind::Type, name("Order", 1, 13), span(1, 13), None);
        let builder = symbols.push(
            SymbolKind::Type,
            name("Builder", 10, 20),
            span(10, 20),
            Some(order),
        );
        symbols.push(
            SymbolKind::Method,
            name("place", 11, 17),
            span(11, 17),
            Some(builder),
        );

        let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
        assert_eq!(
            messages(&diagnostics),
            [
                "`Builder` does not match any pattern of rule `domain-type`: \
                 expected one of `{noun}`, `{noun}{role}`, `{noun}{verb:pp}`. \
                 If `Builder` is a new term or behavior of the ubiquitous language, \
                 define it in `terms.yaml` first",
                "`place` does not match pattern `{verb}` of rule `domain-method`: `{verb}` is \
                 not bound to a term: the enclosing type `Builder` is not in the dictionary `terms.yaml`",
            ]
        );
    }

    #[test]
    fn backtracks_when_a_longer_noun_has_the_required_behavior() {
        let dictionary = Dictionary::from_yaml_str(&format!(
            "{DICTIONARY}{}",
            r#"  - name: OrderLine
    forms: { singular: order line, plural: order lines }
    behaviors:
      - verb: ship
        forms: { base: ship, s: ships, past: shipped, pp: shipped, ing: shipping }
"#
        ))
        .unwrap();
        let config = Config::from_yaml_str(
            r#"
rules:
  - id: method
    include: ["**"]
    target: method
    patterns: ["{noun}{any}{verb}"]
"#,
        )
        .unwrap();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
        // `Order` matches first, but cannot `ship`. Its noun and verb
        // bindings must be undone before trying the longer `OrderLine`.
        let symbols = order_class(&[(SymbolKind::Method, "orderLineExtraShip", 4)]);
        assert_eq!(checker.check(&symbols, Path::new(DOMAIN)), []);
    }

    #[test]
    fn backtracks_when_a_verb_or_vocabulary_needs_more_words() {
        let dictionary = Dictionary::from_yaml_str(CHECKOUT_DICTIONARY).unwrap();
        for (pattern, identifier) in [
            ("{verb}{any}{noun}", "checkOutExtraOrder"),
            ("{role}{verb}{noun}", "serviceExtraPlaceOrder"),
        ] {
            let config = Config::from_yaml_str(&format!(
                r#"
vocabularies:
  role: [service, service extra]
rules:
  - id: method
    include: ["**"]
    target: method
    patterns: ["{pattern}"]
"#
            ))
            .unwrap();
            let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
            let symbols = order_class(&[(SymbolKind::Method, identifier, 4)]);
            assert_eq!(
                checker.check(&symbols, Path::new(DOMAIN)),
                [],
                "{identifier}"
            );
        }
    }

    #[test]
    fn records_current_binding_of_verbs_to_the_first_of_multiple_nouns() {
        // This records the current implementation, not a settled specification.
        // The multiple-noun policy remains open in docs/plan.md section 8;
        // update this regression test when that policy is decided.
        let (dictionary, _) = setup();
        let config = Config::from_yaml_str(
            r#"
rules:
  - id: method
    include: ["**"]
    target: method
    patterns: ["{noun}{verb}{noun}{verb}"]
"#,
        )
        .unwrap();
        let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
        let symbols = order_class(&[
            (SymbolKind::Method, "orderPlaceLineItemCancel", 4),
            (SymbolKind::Method, "lineItemPlaceOrderCancel", 7),
        ]);
        assert_eq!(
            messages(&checker.check(&symbols, Path::new(DOMAIN))),
            [
                "`lineItemPlaceOrderCancel` does not match pattern `{noun}{verb}{noun}{verb}` of \
             rule `method`: `place` is not a defined behavior of `LineItem` (no behaviors are defined)"
            ]
        );
    }

    #[test]
    fn nearest_pattern_ties_keep_the_configured_order() {
        let (dictionary, _) = setup();
        let patterns = ["{verb}{noun}", "{any}{verb}"];
        for (first, second) in [(patterns[0], patterns[1]), (patterns[1], patterns[0])] {
            let config = Config::from_yaml_str(&format!(
                r#"
rules:
  - id: method
    include: ["**"]
    target: method
    patterns: ["{first}", "{second}"]
"#
            ))
            .unwrap();
            let checker = Checker::new(&dictionary, Path::new(DICTIONARY_FILE_NAME), &config);
            let symbols = order_class(&[(SymbolKind::Method, "updateOrder", 4)]);
            let diagnostics = checker.check(&symbols, Path::new(DOMAIN));
            assert_eq!(diagnostics.len(), 1);
            assert!(
                diagnostics[0]
                    .message
                    .contains(&format!("does not match pattern `{first}`"))
            );
        }
    }
}
