//! Language-agnostic core of ubiqlint.
//!
//! This crate loads dictionaries (`terms.yaml`) and configuration
//! (`ubiqlint.yaml`), splits identifiers into words, matches them against
//! the configured patterns and produces diagnostics. It knows nothing about
//! any particular programming language; language adapters supply symbols.
