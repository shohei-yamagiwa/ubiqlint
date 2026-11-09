//! The language-independent facts extracted from one source file.

use crate::location::Span;
use crate::symbol::Symbols;

/// A parsed source file. Imports are kept apart from declarations because
/// they introduce names without declaring symbols or enclosing other ones.
#[derive(Debug, Default)]
pub struct SourceFile {
    pub symbols: Symbols,
    pub imports: Vec<Import>,
}

/// An import of one name or every name in a namespace.
///
/// Adapters split the qualified path into its components, translating any
/// language-specific escapes and omitting comments, separators and `*`.
/// A named import ends in the name it introduces; a wildcard import ends
/// in its namespace (which may itself be a type).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub path: Vec<String>,
    pub wildcard: bool,
    /// The whole import declaration, in the source as written.
    pub span: Span,
}

impl Import {
    /// The namespace that supplies the imported name(s).
    pub fn namespace(&self) -> &[String] {
        if self.wildcard {
            &self.path
        } else {
            &self.path[..self.path.len().saturating_sub(1)]
        }
    }

    /// The name introduced by a named import, or `None` for a wildcard.
    pub fn name(&self) -> Option<&str> {
        if self.wildcard {
            None
        } else {
            self.path.last().map(String::as_str)
        }
    }
}
