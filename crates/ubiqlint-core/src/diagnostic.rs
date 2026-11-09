//! Problems found in input files.

use std::fmt;

use crate::location::{Location, Span};

/// A problem found in a file, such as an invalid definition in a dictionary
/// or a syntax error in source code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub location: Location,
    pub message: String,
    /// The offending identifier, import or syntax, when a range is known.
    pub span: Option<Span>,
    /// The configured rule that produced the violation, if any.
    pub rule_id: Option<String>,
}

impl Diagnostic {
    /// A problem at a position, without a known range or configured rule.
    pub fn new(location: Location, message: impl Into<String>) -> Self {
        Self {
            location,
            message: message.into(),
            span: None,
            rule_id: None,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}
