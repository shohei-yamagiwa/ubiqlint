//! Problems found in input files.

use std::fmt;

use crate::location::Location;

/// A problem found in a file, such as an invalid definition in a dictionary
/// or a syntax error in source code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub location: Location,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}
