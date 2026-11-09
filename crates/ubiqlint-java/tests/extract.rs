use ubiqlint_core::{LanguageAdapter, Location, Span, Symbol, SymbolKind, Symbols};
use ubiqlint_java::{JavaAdapter, extract};

fn symbols(source: &str) -> Symbols {
    extract(source).unwrap_or_else(|errors| panic!("unexpected syntax errors: {errors:?}"))
}

/// Lists the symbols one per line as `kind name`, indented by depth.
fn outline(source: &str) -> String {
    let symbols = symbols(source);
    let mut lines = String::new();
    for symbol in &symbols {
        let depth = symbols.ancestors(symbol).count();
        let name = symbol
            .name
            .as_ref()
            .map_or("(anonymous)", |name| name.text.as_str());
        lines += &format!("{}{} {name}\n", "  ".repeat(depth), symbol.kind);
    }
    lines
}

/// Removes the common indentation of the lines of an expected outline.
fn lines(text: &str) -> String {
    let text = text.trim_start_matches('\n').trim_end();
    let indent = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    text.lines()
        .map(|line| format!("{}\n", &line[indent..]))
        .collect()
}

fn find<'a>(symbols: &'a Symbols, name: &str) -> &'a Symbol {
    symbols
        .iter()
        .find(|symbol| symbol.name.as_ref().is_some_and(|n| n.text == name))
        .unwrap_or_else(|| panic!("no symbol named {name}"))
}

fn span(start: (usize, usize), end: (usize, usize)) -> Span {
    Span {
        start: Location {
            line: start.0,
            column: start.1,
        },
        end: Location {
            line: end.0,
            column: end.1,
        },
    }
}

#[test]
fn handles_java_files() {
    assert_eq!(JavaAdapter.extensions(), ["java"]);
    let symbols = JavaAdapter.extract("class Order {}").unwrap();
    assert_eq!(symbols.len(), 1);
}

#[test]
fn extracts_every_kind_of_type() {
    let source = r#"
        package com.example.order;

        import java.util.List;

        public class Order {}
        interface OrderRepository {}
        enum OrderStatus {}
        record Money(long amount) {}
        @interface DomainEvent {}
        sealed interface Shape permits Circle {}
        final class Circle implements Shape {}
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
            type OrderRepository
            type OrderStatus
            type Money
              field amount
            type DomainEvent
            type Shape
            type Circle
            "
        )
    );
}

#[test]
fn nests_types_in_types_and_methods() {
    let source = r#"
        class Order {
            static class LineItem {
                enum Status { OPEN }
            }
            void place() {
                record Snapshot(int count) {}
                class Helper {}
            }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
              type LineItem
                type Status
                  field OPEN
              method place
                type Snapshot
                  field count
                type Helper
            "
        )
    );
}

#[test]
fn extracts_members_of_classes() {
    let source = r#"
        class Order {
            private static final int MAX_ITEMS = 10;
            private final List<LineItem> lineItems, removedItems = List.of();
            private int[] quantities;

            Order(OrderId id) {}

            void place(Customer customer, @Valid Address shippingAddress) {}

            <T> List<T> lineItems(int... indexes) { return null; }

            static {}
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
              field MAX_ITEMS
              field lineItems
              field removedItems
              field quantities
              constructor Order
                parameter id
              method place
                parameter customer
                parameter shippingAddress
              method lineItems
                parameter indexes
            "
        )
    );
}

#[test]
fn extracts_members_of_interfaces_and_annotations() {
    let source = r#"
        interface OrderRepository {
            int PAGE_SIZE = 20, MAX_PAGES = 5;
            Order find(OrderId id);
            default void save(Order order) {}
            interface Listener {}
        }
        @interface DomainEvent {
            String name() default "";
            int VERSION = 1;
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type OrderRepository
              field PAGE_SIZE
              field MAX_PAGES
              method find
                parameter id
              method save
                parameter order
              type Listener
            type DomainEvent
              method name
              field VERSION
            "
        )
    );
}

#[test]
fn extracts_enum_constants_and_their_bodies() {
    let source = r#"
        enum OrderStatus implements Labeled {
            PLACED("placed"),
            CANCELED("canceled") {
                @Override String label() { return "x"; }
            };

            private final String code;

            OrderStatus(String code) { this.code = code; }

            String label() { return code; }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type OrderStatus
              field PLACED
              field CANCELED
                type (anonymous)
                  method label
              field code
              constructor OrderStatus
                parameter code
              method label
            "
        )
    );
}

#[test]
fn extracts_record_components_and_compact_constructors() {
    let source = r#"
        record Money(long amount, Currency currency, String... notes) {
            Money {
                long rounded = amount;
            }
            Money(long amount) { this(amount, null); }
            static Money zero() { return null; }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Money
              field amount
              field currency
              field notes
              constructor Money
                variable rounded
              constructor Money
                parameter amount
              method zero
            "
        )
    );
}

#[test]
fn extracts_parameters() {
    let source = r#"
        class Order {
            void place(final Customer customer, String[] notes, Item items[], Object... extras) {}
            void cancel(Order this, Reason reason) {}
            void pay() {
                try {
                } catch (PaymentException | IllegalStateException failure) {
                } catch (final RuntimeException unexpected) {
                }
            }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
              method place
                parameter customer
                parameter notes
                parameter items
                parameter extras
              method cancel
                parameter reason
              method pay
                parameter failure
                parameter unexpected
            "
        )
    );
}

#[test]
fn extracts_lambda_parameters() {
    let source = r#"
        class Order {
            void place() {
                Runnable none = () -> {};
                items.forEach(item -> {});
                items.sort((left, right) -> 0);
                items.removeIf((LineItem item) -> {
                    int quantity = item.quantity();
                    return quantity == 0;
                });
            }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
              method place
                variable none
                parameter item
                parameter left
                parameter right
                parameter item
                variable quantity
            "
        )
    );
}

#[test]
fn extracts_local_variables() {
    let source = r#"
        class Order {
            void place() {
                int count = 0, total;
                var items = List.of();
                for (int i = 0, j = 0; i < 1; i++) {}
                for (final LineItem item : items) {}
                try (var reader = open(); Reader second = open(); existing) {}
                if (payment instanceof final CardPayment card) {}
                if (payment instanceof CashPayment(var amount, Currency currency)) {}
                switch (payment) {
                    case CardPayment card when card.valid() -> {}
                    case CashPayment(var amount, var currency) -> {}
                    default -> {}
                }
            }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
              method place
                variable count
                variable total
                variable items
                variable i
                variable j
                variable item
                variable reader
                variable second
                variable card
                variable amount
                variable currency
                variable card
                variable amount
                variable currency
            "
        )
    );
}

#[test]
fn skips_unnamed_variables() {
    let source = r#"
        class Order {
            void place() {
                for (var _ : items) {}
                try {} catch (Exception _) {}
                if (payment instanceof CashPayment(var _, var currency)) {}
                int _ = 0;
                items.forEach(_ -> {});
                items.sort((_, other) -> 0);
            }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
              method place
                variable currency
                parameter other
            "
        )
    );
}

#[test]
fn makes_anonymous_classes_children_of_their_enclosing_declaration() {
    let source = r#"
        class Order {
            private final Runnable task = new Runnable() {
                public void run() {}
            };

            void place() {
                new Thread() {
                    private int attempts;
                    @Override public void run() {}
                }.start();
            }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
              field task
                type (anonymous)
                  method run
              method place
                type (anonymous)
                  field attempts
                  method run
            "
        )
    );
}

#[test]
fn makes_variables_in_initializers_children_of_the_type() {
    let source = r#"
        class Order {
            static { int loaded = 0; }
            { int created = 0; }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
              variable loaded
              variable created
            "
        )
    );
}

#[test]
fn ignores_identifiers_that_declare_nothing() {
    // Packages, imports, type parameters, annotations, labels, method
    // references and module declarations contain identifiers but declare
    // no symbols.
    let source = r#"
        package com.example.order;

        import static java.util.Objects.requireNonNull;

        @Entity(name = "orders")
        class Order<T extends Comparable<T>> {
            @Size(max = 3) List<T> items;

            void place() {
                outer:
                for (;;) { break outer; }
                items.forEach(System.out::println);
                String text = """
                    class NotAClass {}
                    """;
            }
        }
    "#;
    assert_eq!(
        outline(source),
        lines(
            "
            type Order
              field items
              method place
                variable text
            "
        )
    );
    assert_eq!(
        outline("module com.example.order { requires java.base; }"),
        ""
    );
}

#[test]
fn locates_names_and_declarations() {
    let source = "package p;\n\
                  \n\
                  /** An order. */\n\
                  @Entity\n\
                  public class Order {\n\
                  \tprivate int a, b = 1;\n\
                  \tvoid place(final Customer 顧客) {\n\
                  \t\tfor (final Item item : items) {}\n\
                  \t\tif (o instanceof final Card card) {}\n\
                  \t\titems.forEach(x -> {});\n\
                  \t}\n\
                  }\n";
    let symbols = symbols(source);

    let order = find(&symbols, "Order");
    assert_eq!(order.name.as_ref().unwrap().span, span((5, 14), (5, 19)));
    // The declaration includes annotations and modifiers, but not comments.
    assert_eq!(order.span, span((4, 1), (12, 2)));

    // Variables declared together share the declaration.
    let a = find(&symbols, "a");
    let b = find(&symbols, "b");
    assert_eq!(a.name.as_ref().unwrap().span, span((6, 14), (6, 15)));
    assert_eq!(b.name.as_ref().unwrap().span, span((6, 17), (6, 18)));
    assert_eq!(a.span, span((6, 2), (6, 23)));
    assert_eq!(b.span, a.span);

    // Columns count characters, so a tab or a Japanese character is one.
    let customer = find(&symbols, "顧客");
    assert_eq!(customer.kind, SymbolKind::Parameter);
    assert_eq!(customer.name.as_ref().unwrap().span, span((7, 28), (7, 30)));
    assert_eq!(customer.span, span((7, 13), (7, 30)));

    // The variable of a loop or a pattern spans its modifiers, type and name.
    assert_eq!(find(&symbols, "item").span, span((8, 8), (8, 23)));
    assert_eq!(find(&symbols, "card").span, span((9, 20), (9, 35)));
    assert_eq!(find(&symbols, "x").span, span((10, 17), (10, 18)));
}

#[test]
fn links_children_to_parents() {
    let source = r#"
        class Order {
            void place() {
                Runnable task = () -> { int attempts = 0; };
            }
        }
    "#;
    let symbols = symbols(source);
    let attempts = find(&symbols, "attempts");
    let ancestors: Vec<(SymbolKind, &str)> = symbols
        .ancestors(attempts)
        .map(|symbol| (symbol.kind, symbol.name.as_ref().unwrap().text.as_str()))
        .collect();
    assert_eq!(
        ancestors,
        [
            (SymbolKind::Variable, "task"),
            (SymbolKind::Method, "place"),
            (SymbolKind::Type, "Order"),
        ]
    );
}

#[test]
fn handles_deeply_nested_expressions() {
    // A recursive walk overflows a 2 MiB test thread at about 5,000 terms
    // in a debug build and 20,000 in a release build.
    let terms = vec!["1"; 20_000].join(" + ");
    let source = format!("class Order {{ int total = {terms}; }}");
    assert_eq!(outline(&source), lines("type Order\n  field total"));
}

#[test]
fn reports_syntax_errors_instead_of_symbols() {
    let source =
        "class Order {\n    void place() {\n        int count = 0\n    }\n    void cancel( {}\n}\n";
    let errors = extract(source).unwrap_err();
    let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        errors,
        [
            "3:22: syntax error: missing `;`",
            "5:17: syntax error: missing `)`"
        ]
    );
}

#[test]
fn reports_errors_inside_larger_errors() {
    // tree-sitter wraps this whole file in an error that starts at 1:1, and
    // the actual problem, the `;` at 1:35, in another error inside it.
    let errors = extract("class A { void f() { int x = (1 + ; foo(; } }").unwrap_err();
    let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(errors, ["1:35: syntax error"]);
}

#[test]
fn reports_unexpected_text() {
    let errors = extract("class Order { # }").unwrap_err();
    let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(errors, ["1:15: syntax error"]);
}

#[test]
fn accepts_empty_files() {
    assert!(symbols("").is_empty());
    assert!(symbols("// nothing here\n").is_empty());
}

#[test]
fn keeps_modifiers_in_spans_across_comments() {
    let source = "class A { void f() {\n\
                  if (o instanceof final /* c */ Card card) {}\n\
                  if (o instanceof /* c */ final Card cash) {}\n\
                  for (final /* c */ Item item : items) {}\n\
                  } }";
    let symbols = symbols(source);
    assert_eq!(find(&symbols, "card").span, span((2, 18), (2, 41)));
    assert_eq!(find(&symbols, "cash").span, span((3, 26), (3, 41)));
    assert_eq!(find(&symbols, "item").span, span((4, 6), (4, 29)));
}
