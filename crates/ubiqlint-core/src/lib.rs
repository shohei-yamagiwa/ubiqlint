//! Language-agnostic core of ubiqlint.
//!
//! This crate loads dictionaries (`terms.yaml`) and configuration
//! (`ubiqlint.yaml`), splits identifiers into words, matches them against
//! the configured patterns, checks import boundaries and produces diagnostics.
//! It knows nothing about any particular programming language; language
//! adapters supply declarations and imports through [`LanguageAdapter`].

pub mod check;
pub mod config;
pub mod diagnostic;
pub mod dictionary;
pub mod identifier;
pub mod location;
pub mod pattern;
pub mod resolution;
pub mod source;
pub mod symbol;
pub mod words;

mod matching;
mod yaml;

pub use check::Checker;
pub use config::{CONFIG_FILE_NAME, Config, Excludes, ImportRule, Rule, Target, Vocabulary};
pub use diagnostic::Diagnostic;
pub use dictionary::{Behavior, DICTIONARY_FILE_NAME, Dictionary, NounForms, Term, VerbForms};
pub use location::{LineIndex, Location, Span};
pub use pattern::{Element, NounForm, Pattern, VerbForm};
pub use resolution::{DictionaryResolver, ResolutionError, ResolvedDictionary};
pub use source::{Import, SourceFile};
pub use symbol::{LanguageAdapter, Name, Symbol, SymbolId, SymbolKind, Symbols};
pub use words::Words;
