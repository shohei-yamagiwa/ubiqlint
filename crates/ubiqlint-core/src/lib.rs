//! Language-agnostic core of ubiqlint.
//!
//! This crate loads dictionaries (`terms.yaml`) and configuration
//! (`ubiqlint.yaml`), splits identifiers into words, matches them against
//! the configured patterns and produces diagnostics. It knows nothing about
//! any particular programming language; language adapters supply symbols.

pub mod config;
pub mod diagnostic;
pub mod dictionary;
pub mod pattern;
pub mod words;

mod yaml;

pub use config::{CONFIG_FILE_NAME, Config, Rule, Target, Vocabulary};
pub use diagnostic::{Diagnostic, Location};
pub use dictionary::{Behavior, DICTIONARY_FILE_NAME, Dictionary, NounForms, Term, VerbForms};
pub use pattern::{Element, NounForm, Pattern, VerbForm};
pub use words::Words;
