//! Java language adapter for ubiqlint.
//!
//! Parses Java source files with `tree-sitter-java` and describes their
//! declarations and imports to `ubiqlint-core` as a [`SourceFile`]:
//!
//! | Declaration | Kind |
//! |---|---|
//! | Class, interface, enum, record, annotation type | [`Type`](SymbolKind::Type) |
//! | Anonymous class, including the body of an enum constant | [`Type`](SymbolKind::Type), without a name |
//! | Constructor, including a compact constructor of a record | [`Constructor`](SymbolKind::Constructor) |
//! | Method, annotation type element | [`Method`](SymbolKind::Method) |
//! | Field, interface constant, enum constant, record component | [`Field`](SymbolKind::Field) |
//! | Parameter of a method, constructor or lambda, exception parameter of `catch` | [`Parameter`](SymbolKind::Parameter) |
//! | Local variable, variable of an enhanced `for` or `try`-with-resources, pattern variable | [`Variable`](SymbolKind::Variable) |
//!
//! The parent of a symbol is the innermost declaration enclosing it. For
//! example, the parent of a lambda parameter is the method or variable whose
//! body or initializer contains the lambda. Unnamed variables (`_`) and
//! receiver parameters (`Order this`) are not symbols.
//!
//! As in the Java compiler, Unicode escapes are translated before parsing,
//! so `class \u004frder` declares `Order`. Names are given as translated,
//! while locations point into the source as written.

mod lexical;

use tree_sitter::{Node, Parser};
use ubiqlint_core::{
    Diagnostic, Import, LanguageAdapter, LineIndex, Location, Name, SourceFile, Span, SymbolId,
    SymbolKind, Symbols,
};

use crate::lexical::Translated;

/// The adapter for Java source files (`.java`).
#[derive(Clone, Copy, Debug, Default)]
pub struct JavaAdapter;

impl LanguageAdapter for JavaAdapter {
    fn extensions(&self) -> &[&str] {
        &["java"]
    }

    fn extract(&self, source: &str) -> Result<SourceFile, Vec<Diagnostic>> {
        extract(source)
    }
}

/// Extracts declarations and imports from a Java source file, or returns
/// its syntax errors, including malformed Unicode escapes.
pub fn extract(source: &str) -> Result<SourceFile, Vec<Diagnostic>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .expect("the Java grammar is compatible with tree-sitter");
    let lines = LineIndex::new(source);
    let translated = Translated::new(source).map_err(|malformed| {
        malformed
            .iter()
            .map(|escape| Diagnostic {
                location: lines.location(escape.offset),
                message:
                    "invalid Unicode escape: `\\u` must be followed by four hexadecimal digits"
                        .to_owned(),
                span: None,
                rule_id: None,
            })
            .collect::<Vec<_>>()
    })?;
    let source = Source { translated, lines };
    let tree = parser
        .parse(source.translated.text(), None)
        .expect("parsing without a timeout or cancellation always succeeds");
    let root = tree.root_node();
    if root.has_error() {
        return Err(syntax_errors(root, &source));
    }
    let mut extractor = Extractor {
        source,
        symbols: Symbols::new(),
        imports: Vec::new(),
    };
    extractor.walk(root);
    Ok(SourceFile {
        symbols: extractor.symbols,
        imports: extractor.imports,
    })
}

/// A source file as the parser sees it, with a way back to the file as
/// written.
struct Source<'a> {
    translated: Translated<'a>,
    lines: LineIndex<'a>,
}

impl Source<'_> {
    /// Returns the location in the file of an offset in the parsed text.
    fn location(&self, offset: usize) -> Location {
        self.lines.location(self.translated.original_offset(offset))
    }

    /// Returns the span in the file of a range in the parsed text.
    fn span(&self, start: usize, end: usize) -> Span {
        Span {
            start: self.location(start),
            end: self.location(end),
        }
    }

    /// Returns the parsed text of `node`.
    fn text(&self, node: Node) -> &str {
        &self.translated.text()[node.byte_range()]
    }
}

/// Reports each missing token and each innermost error in the tree.
///
/// An error that contains other errors is not reported itself. When
/// tree-sitter cannot recover, it may wrap everything from the start of the
/// file in one error, and reporting that would hide where the problem is.
fn syntax_errors(root: Node, source: &Source) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let mut stack = vec![root];
    let mut cursor = root.walk();
    let mut nested = Vec::new();
    while let Some(node) = stack.pop() {
        // Missing tokens such as `;` are anonymous, so look at every child.
        nested.clear();
        nested.extend(node.children(&mut cursor).filter(Node::has_error));
        let message = if node.is_missing() {
            if node.is_named() {
                format!("syntax error: missing {}", node.kind().replace('_', " "))
            } else {
                format!("syntax error: missing `{}`", node.kind())
            }
        } else if node.is_error() && nested.is_empty() {
            "syntax error".to_owned()
        } else {
            stack.extend(nested.drain(..).rev());
            continue;
        };
        errors.push(Diagnostic {
            location: source.location(node.start_byte()),
            message,
            span: Some(source.span(node.start_byte(), node.end_byte())),
            rule_id: None,
        });
    }
    if errors.is_empty() {
        // Not expected, but a file must never fail without a reason.
        errors.push(Diagnostic {
            location: source.location(root.start_byte()),
            message: "syntax error".to_owned(),
            span: Some(source.span(root.start_byte(), root.end_byte())),
            rule_id: None,
        });
    }
    errors.sort_by_key(|error| error.location);
    errors
}

struct Extractor<'a> {
    source: Source<'a>,
    symbols: Symbols,
    imports: Vec<Import>,
}

impl Extractor<'_> {
    /// Visits every node in source order, keeping track of the innermost
    /// enclosing symbol. Uses an explicit stack, since deeply nested
    /// expressions could otherwise overflow the call stack.
    fn walk(&mut self, root: Node) {
        let mut stack = vec![(root, None)];
        // Reused for every node, to avoid allocating for each one.
        let mut cursor = root.walk();
        let mut children = Vec::new();
        while let Some((node, parent)) = stack.pop() {
            let scope = self.visit(node, parent);
            // Push in reverse, so that popping visits children in source
            // order. Anonymous nodes are tokens such as `{`, which declare
            // nothing.
            children.extend(node.named_children(&mut cursor));
            stack.extend(children.drain(..).rev().map(|child| (child, scope)));
        }
    }

    /// Adds the symbols that `node` declares, and returns the symbol that
    /// encloses the children of `node`.
    fn visit(&mut self, node: Node, parent: Option<SymbolId>) -> Option<SymbolId> {
        match node.kind() {
            "import_declaration" => {
                self.imports.push(self.import(node));
                parent
            }
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration" => self.declare(SymbolKind::Type, node, parent),
            // The body of an anonymous class or of an enum constant.
            "class_body"
                if node.parent().is_some_and(|owner| {
                    matches!(owner.kind(), "object_creation_expression" | "enum_constant")
                }) =>
            {
                let span = self.span(node);
                Some(self.symbols.push(SymbolKind::Type, None, span, parent))
            }
            "constructor_declaration" | "compact_constructor_declaration" => {
                self.declare(SymbolKind::Constructor, node, parent)
            }
            "method_declaration" | "annotation_type_element_declaration" => {
                self.declare(SymbolKind::Method, node, parent)
            }
            "enum_constant" => self.declare(SymbolKind::Field, node, parent),
            "formal_parameter" => self.declare(parameter_kind(node), node, parent),
            "catch_formal_parameter" => self.declare(SymbolKind::Parameter, node, parent),
            "variable_declarator" => self.visit_declarator(node, parent),
            "resource" => self.declare(SymbolKind::Variable, node, parent),
            "type_pattern" | "record_pattern_component" => {
                let name = named_child_of_kind(node, "identifier");
                self.declare_as(SymbolKind::Variable, name, self.span(node), parent)
                    .or(parent)
            }
            "enhanced_for_statement" => {
                // `final Order order` in `for (final Order order : orders)`.
                // The loop itself is not part of the declaration.
                let start = node
                    .child_by_field_name("type")
                    .map(|ty| modifiers(node).unwrap_or(ty));
                self.declare_between(SymbolKind::Variable, node, start, parent);
                parent
            }
            "instanceof_expression" => {
                // `final Order order` in `o instanceof final Order order`.
                let start = child_after(node, "instanceof");
                self.declare_between(SymbolKind::Variable, node, start, parent);
                parent
            }
            "lambda_expression" => {
                // `order -> ...` declares a parameter without parentheses.
                if let Some(name) = node
                    .child_by_field_name("parameters")
                    .filter(|parameters| parameters.kind() == "identifier")
                {
                    self.declare_as(SymbolKind::Parameter, Some(name), self.span(name), parent);
                }
                parent
            }
            "inferred_parameters" => {
                // `(order, item) -> ...`
                let mut cursor = node.walk();
                for name in node.named_children(&mut cursor) {
                    self.declare_as(SymbolKind::Parameter, Some(name), self.span(name), parent);
                }
                parent
            }
            _ => parent,
        }
    }

    /// Reads the identifier leaves instead of splitting the declaration's
    /// text, since comments and whitespace may appear around every dot.
    /// Static imports introduce the final name just like ordinary imports;
    /// without a classpath it may be a nested type, a field or a method.
    fn import(&self, node: Node) -> Import {
        let mut path = Vec::new();
        let mut wildcard = false;
        let mut stack = vec![node];
        let mut cursor = node.walk();
        let mut children = Vec::new();
        while let Some(node) = stack.pop() {
            match node.kind() {
                "identifier" => path.push(self.source.text(node).to_owned()),
                "asterisk" => wildcard = true,
                "import_declaration" | "scoped_identifier" => {
                    children.extend(node.named_children(&mut cursor));
                    stack.extend(children.drain(..).rev());
                }
                _ => {}
            }
        }
        Import {
            path,
            wildcard,
            span: self.span(node),
        }
    }

    /// Visits `name = value` in a field, local variable or varargs parameter
    /// declaration. The symbol spans the whole declaration, so that every
    /// symbol declared together shares its modifiers and type.
    fn visit_declarator(&mut self, node: Node, parent: Option<SymbolId>) -> Option<SymbolId> {
        let Some(declaration) = node.parent() else {
            return parent;
        };
        let kind = match declaration.kind() {
            "field_declaration" | "constant_declaration" => SymbolKind::Field,
            "local_variable_declaration" => SymbolKind::Variable,
            "spread_parameter" => parameter_kind(declaration),
            _ => return parent,
        };
        let name = node.child_by_field_name("name");
        self.declare_as(kind, name, self.span(declaration), parent)
            .or(parent)
    }

    /// Declares the symbol named by the `name` field of `node`, spanning
    /// `node`. Returns the symbol, or `parent` if `node` has no name.
    fn declare(
        &mut self,
        kind: SymbolKind,
        node: Node,
        parent: Option<SymbolId>,
    ) -> Option<SymbolId> {
        let name = node.child_by_field_name("name");
        self.declare_as(kind, name, self.span(node), parent)
            .or(parent)
    }

    /// Declares the symbol named by the `name` field of `node`, spanning from
    /// `start` to the end of the name, including any array dimensions after
    /// it.
    fn declare_between(
        &mut self,
        kind: SymbolKind,
        node: Node,
        start: Option<Node>,
        parent: Option<SymbolId>,
    ) {
        let (Some(start), Some(name)) = (start, node.child_by_field_name("name")) else {
            return;
        };
        let end = node.child_by_field_name("dimensions").unwrap_or(name);
        let span = self.source.span(start.start_byte(), end.end_byte());
        self.declare_as(kind, Some(name), span, parent);
    }

    /// Declares a symbol named by the identifier `name`. Returns `None`,
    /// declaring nothing, if there is no name or it is `_`, which the grammar
    /// parses as an identifier in some places.
    fn declare_as(
        &mut self,
        kind: SymbolKind,
        name: Option<Node>,
        span: Span,
        parent: Option<SymbolId>,
    ) -> Option<SymbolId> {
        let name = name.filter(|name| name.kind() == "identifier")?;
        let text = self.source.text(name);
        if text == "_" {
            return None;
        }
        let name = Name {
            text: text.to_owned(),
            span: self.span(name),
        };
        Some(self.symbols.push(kind, Some(name), span, parent))
    }

    fn span(&self, node: Node) -> Span {
        self.source.span(node.start_byte(), node.end_byte())
    }
}

/// Tells whether a formal parameter declares a parameter or, in the header
/// of a record, a record component.
fn parameter_kind(parameter: Node) -> SymbolKind {
    let declarer = parameter
        .parent()
        .and_then(|parameters| parameters.parent());
    if declarer.is_some_and(|declarer| declarer.kind() == "record_declaration") {
        SymbolKind::Field
    } else {
        SymbolKind::Parameter
    }
}

fn named_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

/// Returns the modifiers of `node`, such as `final` or annotations.
fn modifiers(node: Node) -> Option<Node> {
    named_child_of_kind(node, "modifiers")
}

/// Returns the first child of `node` after the token `kind`, skipping
/// comments.
fn child_after<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .skip_while(|child| child.kind() != kind)
        .skip(1)
        .find(|child| !child.is_extra())
}
