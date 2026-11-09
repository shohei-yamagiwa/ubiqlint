use std::path::Path;

use ubiqlint_core::{
    Checker, Config, Dictionary, Import, Location, Name, SourceFile, Span, SymbolKind,
};

const DICTIONARY: &str = "\
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
  - name: LineItem
    forms: { singular: line item, plural: line items }
";

const CONFIG: &str = "\
exclude: ['**/generated/**']
rules: []
import_rules:
  - id: domain-imports
    include: ['src/*/domain/**']
    internal_namespaces: [com.example.order, com.example.shared]
";

const DOMAIN: &str = "src/main/domain/Order.java";

fn span(line: usize) -> Span {
    Span {
        start: Location { line, column: 1 },
        end: Location { line, column: 30 },
    }
}

fn file(paths: &[&str]) -> SourceFile {
    SourceFile {
        imports: paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let wildcard = path.ends_with(".*");
                Import {
                    path: path
                        .strip_suffix(".*")
                        .unwrap_or(path)
                        .split('.')
                        .map(str::to_owned)
                        .collect(),
                    wildcard,
                    span: span(index + 1),
                }
            })
            .collect(),
        ..SourceFile::default()
    }
}

#[test]
fn rejects_only_exact_dictionary_names_from_external_namespaces() {
    let dictionary = Dictionary::from_yaml_str(DICTIONARY).unwrap();
    let config = Config::from_yaml_str(CONFIG).unwrap();
    let checker = Checker::new(&dictionary, Path::new("terms.yaml"), &config);
    let file = file(&[
        "com.vendor.Order",
        "com.vendor.LineItem",
        "com.vendor.Container.Order",
        "com.vendor.Order.Helper",
        "com.vendor.Orders",
        "com.vendor.ORDER",
        "com.vendor.OrderDto",
        "java.util.List",
    ]);
    let diagnostics = checker.check_file(&file, Path::new(DOMAIN));
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| d.location.line)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        diagnostics[0].message,
        "external import `com.vendor.Order` conflicts with dictionary term `Order` in `terms.yaml` under rule \
         `domain-imports`: use the fully qualified name `com.vendor.Order` instead"
    );
}

#[test]
fn allows_internal_imports_but_not_similar_namespace_prefixes() {
    let dictionary = Dictionary::from_yaml_str(DICTIONARY).unwrap();
    let config = Config::from_yaml_str(CONFIG).unwrap();
    let checker = Checker::new(&dictionary, Path::new("terms.yaml"), &config);
    let file = file(&[
        "com.example.order.Order",
        "com.example.order.model.LineItem",
        "com.example.shared.Order",
        "com.example.order.*",
        "com.example.order.Order.*",
        "com.example.ordering.Order",
        "com.example.Order.Order",
        // The imported name is not part of its supplying namespace.
        "com.example.order",
    ]);
    let diagnostics = checker.check_file(&file, Path::new(DOMAIN));
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| d.location.line)
            .collect::<Vec<_>>(),
        [6, 7]
    );
}

#[test]
fn forbids_external_wildcards_even_without_dictionary_terms() {
    let dictionary = Dictionary::from_yaml_str("context: empty\nterms: []\n").unwrap();
    let config = Config::from_yaml_str(CONFIG).unwrap();
    let checker = Checker::new(&dictionary, Path::new("terms.yaml"), &config);
    let file = file(&[
        "java.util.*",
        "com.vendor.Customer.*",
        "com.example.ordering.*",
    ]);
    let diagnostics = checker.check_file(&file, Path::new(DOMAIN));
    assert_eq!(diagnostics.len(), 3);
    assert_eq!(
        diagnostics[0].message,
        "external wildcard import `java.util.*` is forbidden by rule `domain-imports`: \
         import non-conflicting names explicitly, or use fully qualified names"
    );
}

#[test]
fn import_rules_are_opt_in_and_respect_paths_and_excludes() {
    let dictionary = Dictionary::from_yaml_str(DICTIONARY).unwrap();
    let config = Config::from_yaml_str(CONFIG).unwrap();
    let checker = Checker::new(&dictionary, Path::new("terms.yaml"), &config);
    let file = file(&["com.vendor.Order", "com.vendor.*"]);
    for path in [
        "src/main/dto/Order.java",
        "src/main/java/domain/Order.java",
        "src/main/domain/generated/Order.java",
    ] {
        assert!(
            checker.check_file(&file, Path::new(path)).is_empty(),
            "{path}"
        );
    }
    let config = Config::from_yaml_str("rules: []").unwrap();
    assert!(
        Checker::new(&dictionary, Path::new("terms.yaml"), &config)
            .check_file(&file, Path::new(DOMAIN))
            .is_empty()
    );
}

#[test]
fn every_matching_import_rule_must_be_satisfied() {
    let dictionary = Dictionary::from_yaml_str(DICTIONARY).unwrap();
    let extra = r#"  - id: strict-imports
    include: ['src/**']
    internal_namespaces: []
"#;
    let config = Config::from_yaml_str(&format!("{CONFIG}{extra}")).unwrap();
    let file = file(&["com.example.order.Order", "com.vendor.Order"]);
    let diagnostics = Checker::new(&dictionary, Path::new("terms.yaml"), &config)
        .check_file(&file, Path::new(DOMAIN));
    assert_eq!(diagnostics.len(), 3);
    assert!(diagnostics[0].message.contains("strict-imports"));
    assert!(diagnostics[1].message.contains("domain-imports"));
    assert!(diagnostics[2].message.contains("strict-imports"));
}

#[test]
fn combines_import_and_naming_violations_in_source_order() {
    let dictionary = Dictionary::from_yaml_str(DICTIONARY).unwrap();
    let config = Config::from_yaml_str(&CONFIG.replace(
        "rules: []",
        "\
rules:
  - id: domain-type
    include: ['**']
    target: type
    patterns: ['{noun}']",
    ))
    .unwrap();
    let mut file = file(&["com.vendor.Order"]);
    file.symbols.push(
        SymbolKind::Type,
        Some(Name {
            text: "Purchase".to_owned(),
            span: span(5),
        }),
        span(5),
        None,
    );
    let diagnostics = Checker::new(&dictionary, Path::new("terms.yaml"), &config)
        .check_file(&file, Path::new(DOMAIN));
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| d.location.line)
            .collect::<Vec<_>>(),
        [1, 5]
    );
    assert!(diagnostics[1].message.contains("domain-type"));
}

#[test]
fn an_imported_name_is_never_mistaken_for_an_internal_namespace() {
    let dictionary = Dictionary::from_yaml_str(DICTIONARY).unwrap();
    let config = Config::from_yaml_str(
        "\
rules: []
import_rules:
  - id: boundary
    include: ['**']
    internal_namespaces: [com.vendor.Order]
",
    )
    .unwrap();
    let file = file(&[
        "com.vendor.Order",
        "com.vendor.Order.*",
        "com.vendor.Order.LineItem",
    ]);
    let diagnostics = Checker::new(&dictionary, Path::new("terms.yaml"), &config)
        .check_file(&file, Path::new(DOMAIN));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].location.line, 1);
}
