use std::path::Path;

use ubiqlint_core::{Checker, Config, Dictionary, LineIndex, Location};
use ubiqlint_java::extract;

fn paths(source: &str) -> Vec<String> {
    extract(source)
        .unwrap()
        .imports
        .iter()
        .map(|import| {
            let suffix = if import.wildcard { ".*" } else { "" };
            format!("{}{suffix}", import.path.join("."))
        })
        .collect()
}

#[test]
fn extracts_named_wildcard_nested_and_static_imports() {
    let source = "\
package com.example.order;
import java.util.List;
import com.vendor.Customer;
import com.vendor.Container.Order;
import com.vendor.*;
import static com.vendor.Container.LineItem;
import static com.vendor.Customer.create;
import static com.vendor.Customer.*;
class Order {}
";
    assert_eq!(
        paths(source),
        [
            "java.util.List",
            "com.vendor.Customer",
            "com.vendor.Container.Order",
            "com.vendor.*",
            "com.vendor.Container.LineItem",
            "com.vendor.Customer.create",
            "com.vendor.Customer.*",
        ]
    );
    // Imports introduce no declarations and never become symbol parents.
    let file = extract(source).unwrap();
    assert_eq!(file.symbols.len(), 1);
    assert!(file.symbols.iter().next().unwrap().parent.is_none());
}

#[test]
fn removes_comments_and_whitespace_and_keeps_contextual_identifiers() {
    let source = "\
import /* com.fake.Order */ com /* comment */ . vendor
    . module . record . Order;
import static com.vendor.Container /* .Order */ . /* .* */ *;
// import com.fake.LineItem;
class Order { String text = \"import com.fake.LineItem;\"; }
";
    assert_eq!(
        paths(source),
        ["com.vendor.module.record.Order", "com.vendor.Container.*"]
    );
}

#[test]
fn translates_escapes_and_locates_imports_in_the_original_source() {
    let source =
        r"/* 注文 */ \u0069mport com.\u0076endor.\u004frder;\u000aimport com.vendor.\u002a;";
    let file = extract(source).unwrap();
    assert_eq!(paths(source), ["com.vendor.Order", "com.vendor.*"]);
    let lines = LineIndex::new(source);
    let semicolon = source.find(';').unwrap();
    assert_eq!(
        file.imports[0].span,
        lines.span(source.find(r"\u0069").unwrap(), semicolon + 1)
    );
    assert_eq!(
        file.imports[1].span,
        lines.span(source.find("import").unwrap(), source.len())
    );
    // An escaped line feed does not add a line in the file as written.
    assert_eq!(file.imports[1].span.start.line, 1);
}

#[test]
fn locates_multiline_imports_across_every_line_ending() {
    let source = "package p;\r\n\timport com.vendor\r . Order;\nimport com.vendor.*;";
    let file = extract(source).unwrap();
    assert_eq!(file.imports[0].span.start, Location { line: 2, column: 2 });
    assert_eq!(
        file.imports[0].span.end,
        Location {
            line: 3,
            column: 10
        }
    );
    assert_eq!(file.imports[1].span.start, Location { line: 4, column: 1 });
}

#[test]
fn syntax_errors_prevent_returning_partial_imports() {
    for source in [
        "import com.vendor.Order; class Broken {",
        "import com..Order; class Order {}",
        r"import com.vendor.\u00G1; class Order {}",
    ] {
        assert!(!extract(source).unwrap_err().is_empty(), "{source}");
    }
}

#[test]
fn checks_the_name_introduced_by_static_imports_including_possible_nested_types() {
    let dictionary = Dictionary::from_yaml_str(
        "\
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
",
    )
    .unwrap();
    let config = Config::from_yaml_str(
        "\
rules: []
import_rules:
  - id: domain-imports
    include: ['**']
    internal_namespaces: [com.example.order]
",
    )
    .unwrap();
    let source = "\
import static com.vendor.Container.Order;
import static com.vendor.Order.create;
import static com.vendor.Order.*;
import static com.example.order.Container.Order;
import static com.example.order.Order.*;
class Customer { com.vendor.Order order; }
";
    let file = extract(source).unwrap();
    let diagnostics = Checker::new(&dictionary, Path::new("terms.yaml"), &config)
        .check_file(&file, Path::new("Order.java"));
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| d.location.line)
            .collect::<Vec<_>>(),
        [1, 3]
    );
}
