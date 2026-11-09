//! Pattern matching and dictionary lookups, independent of symbols and diagnostics.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::config::{Config, Vocabulary};
use crate::dictionary::{Dictionary, Term};
use crate::pattern::{Element, NounForm, Pattern, VerbForm};
use crate::words::Words;

/// Matches word sequences using one dictionary and the configured vocabularies.
pub(crate) struct Matcher<'a> {
    /// The dictionary shared by lookups and diagnostic descriptions.
    dictionary: &'a Dictionary,
    /// Configured word sequences, indexed by placeholder name.
    vocabularies: HashMap<&'a str, &'a Vocabulary>,
    /// The term each singular form belongs to, borrowing the dictionary's words.
    /// Dictionary validation keeps noun forms unique across terms, so one
    /// index per form is enough; `or_insert` is not a conflict-resolution policy.
    singular: HashMap<&'a [String], usize>,
    /// The corresponding index for plural forms, under the same uniqueness invariant.
    plural: HashMap<&'a [String], usize>,
    /// Every form of every behavior, borrowing the dictionary's words.
    /// A relaxed verb can span multiple words only if they are a known form.
    verb_forms: HashSet<&'a [String]>,
}

/// Whether verb bindings must satisfy the dictionary or only suggest a diagnosis.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchMode {
    /// Require every verb to be a defined behavior in the requested form.
    Strict,
    /// Relax verb validation to describe the nearest pattern on failure.
    Relaxed,
}

/// What a placeholder of a pattern matched.
enum Binding {
    /// A noun placeholder and the dictionary term it matched.
    Noun { term: usize },
    /// A verb placeholder, its requested form and its range of words.
    Verb { form: VerbForm, span: Range<usize> },
}

/// The placeholder bindings of one complete match, in pattern order.
#[derive(Default)]
pub(crate) struct Bindings(Vec<Binding>);

impl Bindings {
    /// Currently the first noun takes precedence over the enclosing type.
    /// The multiple-noun policy remains provisional (see `docs/plan.md` section 8).
    pub(crate) fn bound_term(&self, enclosing: Option<usize>) -> Option<usize> {
        self.0
            .iter()
            .find_map(|binding| match binding {
                Binding::Noun { term } => Some(*term),
                Binding::Verb { .. } => None,
            })
            .or(enclosing)
    }

    /// Returns verb forms and word ranges in pattern order.
    pub(crate) fn verbs(&self) -> impl Iterator<Item = (VerbForm, Range<usize>)> + '_ {
        self.0.iter().filter_map(|binding| match binding {
            Binding::Verb { form, span } => Some((*form, span.clone())),
            Binding::Noun { .. } => None,
        })
    }

    /// Counts the words consumed by verbs to rank relaxed matches.
    pub(crate) fn verb_words(&self) -> usize {
        self.verbs().map(|(_, span)| span.len()).sum()
    }
}

impl<'a> Matcher<'a> {
    /// Builds lookup indices over a validated dictionary and configuration.
    pub(crate) fn new(dictionary: &'a Dictionary, config: &'a Config) -> Self {
        let vocabularies = config
            .vocabularies
            .iter()
            .map(|vocabulary| (vocabulary.name.as_str(), vocabulary))
            .collect();
        let mut singular = HashMap::new();
        let mut plural = HashMap::new();
        for (index, term) in dictionary.terms.iter().enumerate() {
            singular
                .entry(term.forms.singular.as_slice())
                .or_insert(index);
            plural.entry(term.forms.plural.as_slice()).or_insert(index);
        }
        let mut verb_forms = HashSet::new();
        for term in &dictionary.terms {
            for behavior in &term.behaviors {
                for form in behavior.forms.all() {
                    verb_forms.insert(form.as_slice());
                }
            }
        }
        Self {
            dictionary,
            vocabularies,
            singular,
            plural,
            verb_forms,
        }
    }

    /// Returns the first complete match. Relaxed matching is used only to
    /// describe a violation, and still limits verbs to one word or a known form.
    pub(crate) fn attempt(
        &self,
        pattern: &Pattern,
        words: &[String],
        enclosing: Option<usize>,
        mode: MatchMode,
    ) -> Option<Bindings> {
        let mut attempt = Attempt {
            matcher: self,
            words,
            enclosing,
            mode,
            bindings: Bindings::default(),
        };
        attempt
            .match_elements(pattern.elements(), 0)
            .then_some(attempt.bindings)
    }

    /// Returns the dictionary term used by both matching and diagnostics.
    pub(crate) fn term(&self, index: usize) -> &Term {
        &self.dictionary.terms[index]
    }

    /// Returns the term whose singular or plural form is `words`.
    pub(crate) fn term_for_form(&self, form: NounForm, words: &[String]) -> Option<usize> {
        match form {
            NounForm::Singular => self.singular.get(words).copied(),
            NounForm::Plural => self.plural.get(words).copied(),
        }
    }

    /// Returns the words of `form` of the behavior of `term` that `words`
    /// are a form of, if any: the form the pattern expects instead.
    pub(crate) fn term_behavior_form(
        &self,
        term: usize,
        words: &[String],
        form: VerbForm,
    ) -> Option<&Words> {
        self.dictionary.terms[term]
            .behaviors
            .iter()
            .find(|behavior| behavior.forms.all().iter().any(|f| f.as_slice() == words))
            .map(|behavior| behavior.forms.get(form))
    }

    /// Tells whether `words` are the given form of a behavior of `term`.
    pub(crate) fn term_allows(&self, term: usize, form: VerbForm, words: &[String]) -> bool {
        self.dictionary.terms[term]
            .behaviors
            .iter()
            .any(|behavior| behavior.forms.get(form).as_slice() == words)
    }
}

/// State shared by all branches of one match. Keeping it here makes each
/// recursive call carry only the remaining pattern and the word position.
struct Attempt<'a> {
    matcher: &'a Matcher<'a>,
    words: &'a [String],
    enclosing: Option<usize>,
    mode: MatchMode,
    bindings: Bindings,
}

impl Attempt<'_> {
    /// Variable-length elements try each non-empty prefix and backtrack on failure.
    fn match_elements(&mut self, elements: &[Element], pos: usize) -> bool {
        let Some((first, rest)) = elements.split_first() else {
            return pos == self.words.len() && self.verbs_allowed();
        };
        match first {
            Element::Literal(literal) => {
                let end = pos + literal.len();
                self.words.get(pos..end) == Some(literal.as_slice())
                    && self.match_elements(rest, end)
            }
            Element::Noun(form) => (pos + 1..=self.words.len()).any(|end| {
                self.matcher
                    .term_for_form(*form, &self.words[pos..end])
                    .is_some_and(|term| self.with_binding(Binding::Noun { term }, rest, end))
            }),
            Element::Vocabulary(name) => {
                self.matcher
                    .vocabularies
                    .get(name.as_str())
                    .is_some_and(|vocabulary| {
                        vocabulary.words.iter().any(|entry| {
                            let end = pos + entry.as_slice().len();
                            self.words.get(pos..end) == Some(entry.as_slice())
                                && self.match_elements(rest, end)
                        })
                    })
            }
            // `{any}` needs at least one word: `get{any}` cannot match `get`.
            Element::Any => (pos + 1..=self.words.len()).any(|end| self.match_elements(rest, end)),
            Element::Verb(form) => (pos + 1..=self.words.len()).any(|end| {
                if self.mode == MatchMode::Relaxed
                    && end - pos > 1
                    && !self.matcher.verb_forms.contains(&self.words[pos..end])
                {
                    return false;
                }
                self.with_binding(
                    Binding::Verb {
                        form: *form,
                        span: pos..end,
                    },
                    rest,
                    end,
                )
            }),
        }
    }

    /// Both nouns and verbs must undo their binding when a branch fails.
    fn with_binding(&mut self, binding: Binding, rest: &[Element], end: usize) -> bool {
        self.bindings.0.push(binding);
        if self.match_elements(rest, end) {
            true
        } else {
            self.bindings.0.pop();
            false
        }
    }

    /// Checks every verb against its bound term, unless this is a relaxed match.
    fn verbs_allowed(&self) -> bool {
        if self.mode == MatchMode::Relaxed {
            return true;
        }
        // With no verb bindings this is vacuously true, even without a term.
        let term = self.bindings.bound_term(self.enclosing);
        self.bindings.verbs().all(|(form, span)| {
            term.is_some_and(|term| self.matcher.term_allows(term, form, &self.words[span]))
        })
    }
}
