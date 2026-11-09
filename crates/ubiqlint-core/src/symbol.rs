//! Declarations that language adapters extract from source files.
//!
//! The core knows nothing about any particular programming language. An
//! adapter parses a source file and describes its declarations as
//! [`Symbols`]: what kind each declaration is, its name, where it is, and
//! which declaration directly encloses it. The core then finds, for
//! example, the innermost type enclosing a method by following parents,
//! without interpreting any syntax.

use std::fmt;
use std::slice;

use crate::diagnostic::Diagnostic;
use crate::location::Span;

/// Extracts symbols from the source files of one programming language.
pub trait LanguageAdapter {
    /// Returns the file extensions this adapter handles, without the leading
    /// dot, such as `["java"]`.
    fn extensions(&self) -> &[&str];

    /// Extracts the declarations in `source`.
    ///
    /// Returns the syntax errors instead if `source` cannot be parsed, so
    /// that a broken file is not checked against what the parser guessed.
    fn extract(&self, source: &str) -> Result<Symbols, Vec<Diagnostic>>;
}

/// Identifies a symbol within the [`Symbols`] of one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(usize);

/// The kind of a declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// Classes, interfaces, enums, records and similar declarations,
    /// including anonymous ones.
    Type,
    /// Constructors. Their names are given by their types, so they are never
    /// checked, but they enclose parameters and local variables.
    Constructor,
    Method,
    Field,
    Parameter,
    /// Local variables.
    Variable,
}

impl SymbolKind {
    /// Returns a lowercase name for messages, such as `type`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Constructor => "constructor",
            Self::Method => "method",
            Self::Field => "field",
            Self::Parameter => "parameter",
            Self::Variable => "variable",
        }
    }
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The name of a declaration as written in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub text: String,
    /// The span of the identifier, used to report violations.
    pub span: Span,
}

/// A declaration in a source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub id: SymbolId,
    pub kind: SymbolKind,
    /// `None` for declarations without a name, such as anonymous classes.
    pub name: Option<Name>,
    /// The span of the whole declaration.
    pub span: Span,
    /// The declaration directly enclosing this one, or `None` at the top
    /// level.
    pub parent: Option<SymbolId>,
}

/// The symbols of one source file.
///
/// A symbol can only be added after its parent, so parents always come
/// before their children and following parents always ends.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Symbols {
    symbols: Vec<Symbol>,
}

impl Symbols {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a symbol and returns its ID.
    ///
    /// # Panics
    ///
    /// Panics if `parent` is not a symbol of this collection.
    pub fn push(
        &mut self,
        kind: SymbolKind,
        name: Option<Name>,
        span: Span,
        parent: Option<SymbolId>,
    ) -> SymbolId {
        if let Some(SymbolId(index)) = parent {
            assert!(
                index < self.symbols.len(),
                "parent {index} is not a symbol of this collection"
            );
        }
        let id = SymbolId(self.symbols.len());
        self.symbols.push(Symbol {
            id,
            kind,
            name,
            span,
            parent,
        });
        id
    }

    /// Returns the symbol with the given ID, or `None` if it belongs to
    /// another collection.
    pub fn get(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(id.0)
    }

    /// Returns the symbol directly enclosing `symbol`.
    pub fn parent(&self, symbol: &Symbol) -> Option<&Symbol> {
        symbol.parent.and_then(|id| self.get(id))
    }

    /// Returns the symbols enclosing `symbol`, from the innermost outwards.
    pub fn ancestors<'a>(&'a self, symbol: &'a Symbol) -> impl Iterator<Item = &'a Symbol> {
        let mut current = symbol;
        std::iter::from_fn(move || {
            current = self.parent(current)?;
            Some(current)
        })
    }

    /// Returns the symbols in the order they were added.
    pub fn iter(&self) -> slice::Iter<'_, Symbol> {
        self.symbols.iter()
    }

    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
}

impl<'a> IntoIterator for &'a Symbols {
    type Item = &'a Symbol;
    type IntoIter = slice::Iter<'a, Symbol>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Location;

    fn span(line: usize) -> Span {
        Span {
            start: Location { line, column: 1 },
            end: Location { line, column: 2 },
        }
    }

    fn name(text: &str, line: usize) -> Option<Name> {
        Some(Name {
            text: text.to_owned(),
            span: span(line),
        })
    }

    #[test]
    fn follows_parents_outwards() {
        let mut symbols = Symbols::new();
        let order = symbols.push(SymbolKind::Type, name("Order", 1), span(1), None);
        let anonymous = symbols.push(SymbolKind::Type, None, span(2), Some(order));
        let run = symbols.push(SymbolKind::Method, name("run", 3), span(3), Some(anonymous));

        assert_eq!(symbols.len(), 3);
        let run = symbols.get(run).unwrap();
        assert_eq!(symbols.parent(run).map(|symbol| symbol.id), Some(anonymous));
        let ancestors: Vec<SymbolId> = symbols.ancestors(run).map(|symbol| symbol.id).collect();
        assert_eq!(ancestors, [anonymous, order]);
        let order = symbols.get(order).unwrap();
        assert_eq!(symbols.parent(order), None);
        assert_eq!(symbols.ancestors(order).count(), 0);
    }

    #[test]
    fn iterates_in_the_order_added() {
        let mut symbols = Symbols::new();
        assert!(symbols.is_empty());
        let order = symbols.push(SymbolKind::Type, name("Order", 1), span(1), None);
        symbols.push(SymbolKind::Field, name("id", 2), span(2), Some(order));
        let names: Vec<&str> = symbols
            .iter()
            .map(|symbol| symbol.name.as_ref().unwrap().text.as_str())
            .collect();
        assert_eq!(names, ["Order", "id"]);
        assert_eq!((&symbols).into_iter().count(), 2);
    }

    #[test]
    fn returns_none_for_ids_of_other_collections() {
        let mut other = Symbols::new();
        other.push(SymbolKind::Type, name("Order", 1), span(1), None);
        let id = other.push(SymbolKind::Type, name("LineItem", 2), span(2), None);
        let mut symbols = Symbols::new();
        symbols.push(SymbolKind::Type, name("Order", 1), span(1), None);
        assert_eq!(symbols.get(id), None);
    }

    #[test]
    #[should_panic(expected = "parent 1 is not a symbol of this collection")]
    fn rejects_unknown_parents() {
        let mut other = Symbols::new();
        other.push(SymbolKind::Type, name("Order", 1), span(1), None);
        let parent = other.push(SymbolKind::Type, name("LineItem", 2), span(2), None);
        Symbols::new().push(SymbolKind::Method, name("place", 3), span(3), Some(parent));
    }

    #[test]
    fn names_kinds_in_lowercase() {
        assert_eq!(SymbolKind::Type.to_string(), "type");
        assert_eq!(SymbolKind::Constructor.name(), "constructor");
        assert_eq!(SymbolKind::Variable.name(), "variable");
    }
}
