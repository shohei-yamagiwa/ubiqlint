//! Editing dictionaries in place: each edit changes only the lines it is
//! about, keeps comments and formatting elsewhere, and is refused rather
//! than producing a broken dictionary.

use ubiqlint_core::edit::{self, BehaviorChanges, Edit, EditError, FieldChange, NewTerm, Outcome};
use ubiqlint_core::{Dictionary, TermChanges, VerbForms, Words};

const ORDER: &str = "\
# 注文コンテキストの用語
context: order
terms:
  # 集約
  - name: Order
    kind: entity
    ja: 注文
    description: 顧客が確定させた購入の単位
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place   # 注文を確定する
        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }
      - verb: cancel
        forms: { base: cancel, s: cancels, past: canceled, pp: canceled, ing: canceling }
  - name: LineItem
    forms: { singular: line item, plural: line items }
# 末尾のコメント
";

fn words(text: &str) -> Words {
    Words::parse(text).unwrap()
}

fn verb_forms(base: &str, s: &str, past: &str, pp: &str, ing: &str) -> VerbForms {
    VerbForms {
        base: words(base),
        s: words(s),
        past: words(past),
        pp: words(pp),
        ing: words(ing),
    }
}

fn new_term(name: &str, plural: &str) -> NewTerm {
    NewTerm {
        name: name.to_owned(),
        plural: words(plural),
        kind: None,
        ja: None,
        description: None,
    }
}

fn edit_term(name: &str, changes: TermChanges) -> Edit {
    Edit::EditTerm {
        name: name.to_owned(),
        changes,
    }
}

/// Applies `edit`, expecting a change that reads back as a valid dictionary.
fn edited(source: &str, edit: &Edit) -> String {
    match edit::apply(source, edit) {
        Ok(Outcome::Changed(text)) => {
            Dictionary::from_yaml_str(&text).expect("the edited dictionary is valid");
            text
        }
        other => panic!("expected a change, got {other:?}"),
    }
}

fn rejected(source: &str, edit: &Edit) -> Vec<String> {
    match edit::apply(source, edit) {
        Err(EditError::Rejected(problems)) => problems,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

fn layout_error(source: &str, edit: &Edit) -> String {
    match edit::apply(source, edit) {
        Err(EditError::Layout(message)) => message,
        other => panic!("expected a layout error, got {other:?}"),
    }
}

#[test]
fn adds_a_term_after_the_last_one() {
    let edit = Edit::AddTerm(NewTerm {
        kind: Some("entity".to_owned()),
        ja: Some("出荷".to_owned()),
        description: Some("倉庫から発送する単位: 注文ごと".to_owned()),
        ..new_term("Shipment", "shipments")
    });
    assert_eq!(
        edited(ORDER, &edit),
        ORDER.replace(
            "# 末尾のコメント",
            concat!(
                "  - name: Shipment\n",
                "    kind: entity\n",
                "    ja: 出荷\n",
                "    description: \"倉庫から発送する単位: 注文ごと\"\n",
                "    forms: { singular: shipment, plural: shipments }\n",
                "# 末尾のコメント",
            )
        )
    );
}

#[test]
fn rejects_terms_that_clash_with_existing_ones() {
    let edit = Edit::AddTerm(NewTerm {
        kind: Some("  ".to_owned()),
        ..new_term("ORDER", "line items")
    });
    assert_eq!(
        rejected(ORDER, &edit),
        [
            "term `ORDER` clashes with term `Order` (line 5): term names are compared ignoring case",
            "the form `order` is already used by term `Order` (line 5)",
            "the form `line items` is already used by term `LineItem` (line 15)",
            "`kind` must not be empty",
        ]
    );
    assert_eq!(
        rejected(ORDER, &Edit::AddTerm(new_term("Order", "orders"))),
        [
            "term `Order` is already defined (line 5)",
            "the form `order` is already used by term `Order` (line 5)",
            "the form `orders` is already used by term `Order` (line 5)",
        ]
    );
    assert_eq!(
        rejected(ORDER, &Edit::AddTerm(new_term("Line Item", "line items"))),
        [
            "invalid term name `Line Item`: use ASCII letters and digits, starting with a letter \
             (e.g. `LineItem`)",
            "the form `line items` is already used by term `LineItem` (line 15)",
        ]
    );
}

#[test]
fn renames_a_term_with_its_forms() {
    let edit = edit_term(
        "LineItem",
        TermChanges {
            name: Some("OrderLine".to_owned()),
            plural: Some(words("order lines")),
            ..TermChanges::default()
        },
    );
    assert_eq!(
        edited(ORDER, &edit),
        ORDER.replace(
            "  - name: LineItem\n    forms: { singular: line item, plural: line items }",
            "  - name: OrderLine\n    forms: { singular: order line, plural: order lines }"
        )
    );
}

#[test]
fn renaming_a_term_requires_a_plural_when_the_singular_changes() {
    let rename = |name: &str| {
        edit_term(
            "LineItem",
            TermChanges {
                name: Some(name.to_owned()),
                ..TermChanges::default()
            },
        )
    };
    assert_eq!(
        rejected(ORDER, &rename("OrderLine")),
        [
            "renaming `LineItem` to `OrderLine` changes its singular form to `order line`; \
             give its plural form as well"
        ]
    );
    // `LINEItem` still splits into `line item`, so the plural can stay.
    assert_eq!(
        edited(ORDER, &rename("LINEItem")),
        ORDER.replace("name: LineItem", "name: LINEItem")
    );
    assert_eq!(
        rejected(ORDER, &rename("order")),
        [
            "term `order` clashes with term `Order` (line 5): term names are compared ignoring case",
            "renaming `LineItem` to `order` changes its singular form to `order`; give its \
             plural form as well",
            "the form `order` is already used by term `Order` (line 5)",
        ]
    );
}

#[test]
fn inserts_missing_fields_in_schema_order() {
    let edit = edit_term(
        "LineItem",
        TermChanges {
            description: FieldChange::Set("注文に含まれる1行".to_owned()),
            kind: FieldChange::Set("value_object".to_owned()),
            ..TermChanges::default()
        },
    );
    assert_eq!(
        edited(ORDER, &edit),
        ORDER.replace(
            "  - name: LineItem\n",
            "  - name: LineItem\n    kind: value_object\n    description: 注文に含まれる1行\n"
        )
    );
    let edit = edit_term(
        "Order",
        TermChanges {
            ja: FieldChange::Unset,
            description: FieldChange::Set("確定した購入".to_owned()),
            ..TermChanges::default()
        },
    );
    assert_eq!(
        edited(ORDER, &edit),
        ORDER
            .replace("    ja: 注文\n", "")
            .replace("顧客が確定させた購入の単位", "確定した購入")
    );
}

#[test]
fn leaves_the_source_alone_when_nothing_changes() {
    let edit = edit_term(
        "Order",
        TermChanges {
            kind: FieldChange::Set("entity".to_owned()),
            ..TermChanges::default()
        },
    );
    assert_eq!(edit::apply(ORDER, &edit), Ok(Outcome::Unchanged));
    let edit = edit_term(
        "LineItem",
        TermChanges {
            kind: FieldChange::Unset,
            ..TermChanges::default()
        },
    );
    assert_eq!(edit::apply(ORDER, &edit), Ok(Outcome::Unchanged));
    assert_eq!(
        rejected(ORDER, &edit_term("Order", TermChanges::default())),
        ["no changes were given"]
    );
}

#[test]
fn removes_terms_with_their_lines() {
    let edit = Edit::RemoveTerm {
        name: "Order".to_owned(),
    };
    // The comment above the term is left alone: it may describe a group.
    assert_eq!(
        edited(ORDER, &edit),
        "\
# 注文コンテキストの用語
context: order
terms:
  # 集約
  - name: LineItem
    forms: { singular: line item, plural: line items }
# 末尾のコメント
"
    );
    let edit = Edit::RemoveTerm {
        name: "LineItem".to_owned(),
    };
    let without_line_item = edited(ORDER, &edit);
    assert_eq!(
        without_line_item,
        ORDER.replace(
            "  - name: LineItem\n    forms: { singular: line item, plural: line items }\n",
            ""
        )
    );
    // `terms` is required, so the last term leaves an empty sequence.
    let edit = Edit::RemoveTerm {
        name: "Order".to_owned(),
    };
    assert_eq!(
        edited(&without_line_item, &edit),
        "# 注文コンテキストの用語\ncontext: order\nterms: []\n# 末尾のコメント\n"
    );
}

#[test]
fn reports_unknown_terms_and_behaviors() {
    let edit = Edit::RemoveTerm {
        name: "order".to_owned(),
    };
    assert_eq!(
        rejected(ORDER, &edit),
        ["no term `order` in the dictionary (defined terms: `Order`, `LineItem`)"]
    );
    let edit = Edit::RemoveBehavior {
        term: "Order".to_owned(),
        verb: words("ship"),
    };
    assert_eq!(
        rejected(ORDER, &edit),
        ["no behavior `ship` on term `Order` (defined behaviors: `place`, `cancel`)"]
    );
    let edit = Edit::RemoveBehavior {
        term: "LineItem".to_owned(),
        verb: words("ship"),
    };
    assert_eq!(
        rejected(ORDER, &edit),
        ["no behavior `ship` on term `LineItem` (no behaviors are defined)"]
    );
    let empty = "context: order\nterms: []\n";
    let edit = Edit::RemoveTerm {
        name: "Order".to_owned(),
    };
    assert_eq!(
        rejected(empty, &edit),
        ["no term `Order` in the dictionary (no terms are defined)"]
    );
}

#[test]
fn adds_behaviors_after_the_existing_ones() {
    let edit = Edit::AddBehavior {
        term: "Order".to_owned(),
        forms: verb_forms("ship", "ships", "shipped", "shipped", "shipping"),
    };
    assert_eq!(
        edited(ORDER, &edit),
        ORDER.replace(
            "ing: canceling }\n",
            "ing: canceling }\n      - verb: ship\n        \
             forms: { base: ship, s: ships, past: shipped, pp: shipped, ing: shipping }\n"
        )
    );
    // A term without behaviors gets the field after its forms.
    let edit = Edit::AddBehavior {
        term: "LineItem".to_owned(),
        forms: verb_forms(
            "pick up",
            "picks up",
            "picked up",
            "picked up",
            "picking up",
        ),
    };
    assert_eq!(
        edited(ORDER, &edit),
        ORDER.replace(
            "plural: line items }\n",
            "plural: line items }\n    behaviors:\n      - verb: pick up\n        \
             forms: { base: pick up, s: picks up, past: picked up, pp: picked up, \
             ing: picking up }\n"
        )
    );
    let edit = Edit::AddBehavior {
        term: "Order".to_owned(),
        forms: verb_forms("place", "places", "placed", "placed", "placing"),
    };
    assert_eq!(
        rejected(ORDER, &edit),
        ["behavior `place` is already defined on term `Order` (line 11)"]
    );
}

#[test]
fn edits_behaviors() {
    let edit = Edit::EditBehavior {
        term: "Order".to_owned(),
        verb: words("cancel"),
        changes: BehaviorChanges {
            past: Some(words("cancelled")),
            pp: Some(words("cancelled")),
            ing: Some(words("cancelling")),
            ..BehaviorChanges::default()
        },
    };
    assert_eq!(
        edited(ORDER, &edit),
        ORDER.replace(
            "past: canceled, pp: canceled, ing: canceling",
            "past: cancelled, pp: cancelled, ing: cancelling"
        )
    );
    let edit = Edit::EditBehavior {
        term: "Order".to_owned(),
        verb: words("place"),
        changes: BehaviorChanges {
            verb: Some(words("submit")),
            s: Some(words("submits")),
            past: Some(words("submitted")),
            pp: Some(words("submitted")),
            ing: Some(words("submitting")),
        },
    };
    assert_eq!(
        edited(ORDER, &edit),
        ORDER.replace(
            "- verb: place   # 注文を確定する\n        \
             forms: { base: place, s: places, past: placed, pp: placed, ing: placing }",
            "- verb: submit   # 注文を確定する\n        \
             forms: { base: submit, s: submits, past: submitted, pp: submitted, ing: submitting }"
        )
    );
}

#[test]
fn changing_a_verb_requires_its_other_forms() {
    let edit = Edit::EditBehavior {
        term: "Order".to_owned(),
        verb: words("place"),
        changes: BehaviorChanges {
            verb: Some(words("cancel")),
            past: Some(words("canceled")),
            ..BehaviorChanges::default()
        },
    };
    assert_eq!(
        rejected(ORDER, &edit),
        [
            "behavior `cancel` is already defined on term `Order` (line 13)",
            "changing the verb `place` to `cancel` changes every form; give `s`, `pp`, `ing` \
             as well",
        ]
    );
}

#[test]
fn removes_behaviors_and_the_last_one_takes_the_field_with_it() {
    let edit = Edit::RemoveBehavior {
        term: "Order".to_owned(),
        verb: words("place"),
    };
    let without_place = edited(ORDER, &edit);
    assert_eq!(
        without_place,
        ORDER.replace(
            "      - verb: place   # 注文を確定する\n        \
             forms: { base: place, s: places, past: placed, pp: placed, ing: placing }\n",
            ""
        )
    );
    let edit = Edit::RemoveBehavior {
        term: "Order".to_owned(),
        verb: words("cancel"),
    };
    assert_eq!(
        edited(&without_place, &edit),
        "\
# 注文コンテキストの用語
context: order
terms:
  # 集約
  - name: Order
    kind: entity
    ja: 注文
    description: 顧客が確定させた購入の単位
    forms: { singular: order, plural: orders }
  - name: LineItem
    forms: { singular: line item, plural: line items }
# 末尾のコメント
"
    );
}

#[test]
fn follows_an_indentless_layout_with_block_forms() {
    let source = "\
context: order
terms:
- name: Order
  forms:
    singular: order
    plural: orders

- name: LineItem
  forms:
    singular: line item
    plural: line items
";
    let edit = Edit::AddTerm(new_term("Shipment", "shipments"));
    assert_eq!(
        edited(source, &edit),
        format!(
            "{source}\n- name: Shipment\n  forms:\n    singular: shipment\n    plural: shipments\n"
        )
    );
    let edit = Edit::AddBehavior {
        term: "Order".to_owned(),
        forms: verb_forms("place", "places", "placed", "placed", "placing"),
    };
    assert_eq!(
        edited(source, &edit),
        source.replace(
            "    plural: orders\n",
            "    plural: orders\n  behaviors:\n  - verb: place\n    forms:\n      base: place\n      \
             s: places\n      past: placed\n      pp: placed\n      ing: placing\n"
        )
    );
    // Removing a term between blank lines does not leave two in a row.
    let edit = Edit::RemoveTerm {
        name: "LineItem".to_owned(),
    };
    let three = format!(
        "{source}\n- name: Shipment\n  forms:\n    singular: shipment\n    plural: shipments\n"
    );
    assert_eq!(
        edited(&three, &edit),
        "\
context: order
terms:
- name: Order
  forms:
    singular: order
    plural: orders

- name: Shipment
  forms:
    singular: shipment
    plural: shipments
"
    );
}

#[test]
fn fills_an_empty_flow_sequence() {
    let source = "context: order\nterms: [] # まだない\n";
    let edit = Edit::AddTerm(new_term("Order", "orders"));
    let with_order = edited(source, &edit);
    assert_eq!(
        with_order,
        "context: order\nterms: # まだない\n  - name: Order\n    \
         forms: { singular: order, plural: orders }\n"
    );
    let source = "\
context: order
terms:
  - name: Order
    forms: {singular: order, plural: orders}
    behaviors: []
";
    let edit = Edit::AddBehavior {
        term: "Order".to_owned(),
        forms: verb_forms("place", "places", "placed", "placed", "placing"),
    };
    assert_eq!(
        edited(source, &edit),
        source.replace(
            "    behaviors: []\n",
            "    behaviors:\n      - verb: place\n        \
             forms: {base: place, s: places, past: placed, pp: placed, ing: placing}\n"
        )
    );
}

#[test]
fn keeps_crlf_line_endings_and_a_missing_final_newline() {
    let source = "context: order\r\nterms:\r\n  - name: Order\r\n    \
                  forms: { singular: order, plural: orders }";
    let edit = Edit::AddTerm(new_term("Shipment", "shipments"));
    assert_eq!(
        edited(source, &edit),
        format!(
            "{source}\r\n  - name: Shipment\r\n    forms: {{ singular: shipment, plural: shipments }}"
        )
    );
}

#[test]
fn replaces_block_scalars_and_quoted_values() {
    let source = "\
context: order
terms:
  - name: Order
    description: |
      複数行の
      説明
    forms: { singular: \"order\", plural: 'orders' }
";
    let edit = edit_term(
        "Order",
        TermChanges {
            plural: Some(words("orders2")),
            description: FieldChange::Set("1 行目\n2 行目".to_owned()),
            ..TermChanges::default()
        },
    );
    assert_eq!(
        edited(source, &edit),
        "\
context: order
terms:
  - name: Order
    description: \"1 行目\\n2 行目\"
    forms: { singular: \"order\", plural: orders2 }
"
    );
    // Values that YAML would read as something else are quoted.
    let edit = edit_term(
        "Order",
        TermChanges {
            kind: FieldChange::Set("null".to_owned()),
            ja: FieldChange::Set("- 注文 # 確定".to_owned()),
            ..TermChanges::default()
        },
    );
    assert_eq!(
        edited(source, &edit),
        source.replace(
            "  - name: Order\n",
            "  - name: Order\n    kind: \"null\"\n    ja: \"- 注文 # 確定\"\n"
        )
    );
}

#[test]
fn refuses_layouts_it_cannot_edit_in_place() {
    let source = "\
context: order
terms: [{ name: Order, forms: { singular: order, plural: orders } }]
";
    assert_eq!(
        layout_error(source, &Edit::AddTerm(new_term("Shipment", "shipments"))),
        "cannot edit the dictionary in place: `terms` is written in flow style (`[ ... ]`)"
    );
    let source = "\
context: order
terms:
  - { name: Order, forms: { singular: order, plural: orders } }
";
    let edit = edit_term(
        "Order",
        TermChanges {
            kind: FieldChange::Set("entity".to_owned()),
            ..TermChanges::default()
        },
    );
    assert_eq!(
        layout_error(source, &edit),
        "cannot edit the dictionary in place: `kind` would go in a term written in flow \
         style (`{ ... }`)"
    );
    // Values inside a flow mapping can still be replaced.
    let edit = edit_term(
        "Order",
        TermChanges {
            plural: Some(words("order list")),
            ..TermChanges::default()
        },
    );
    assert_eq!(
        edited(source, &edit),
        source.replace("plural: orders", "plural: order list")
    );
}

#[test]
fn refuses_to_edit_an_invalid_dictionary() {
    let source = "context: order\nterms:\n  - name: Order\n";
    let edit = Edit::AddTerm(new_term("Shipment", "shipments"));
    match edit::apply(source, &edit) {
        Err(EditError::InvalidDictionary(diagnostics)) => assert_eq!(
            diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["3:5: missing required field `forms` in a term"]
        ),
        other => panic!("expected an invalid dictionary, got {other:?}"),
    }
}

#[test]
fn removes_fields_written_as_block_scalars() {
    let source = "\
context: order
terms:
  -
    name: Order
    description: >-
      折り返される
      説明

    forms: { singular: order, plural: orders }
";
    let edit = edit_term(
        "Order",
        TermChanges {
            description: FieldChange::Unset,
            ..TermChanges::default()
        },
    );
    assert_eq!(
        edited(source, &edit),
        "\
context: order
terms:
  -
    name: Order

    forms: { singular: order, plural: orders }
"
    );
}

#[test]
fn refuses_to_remove_a_field_that_shares_the_line_of_its_dash() {
    let source = "\
context: order
terms:
  - kind: entity
    name: Order
    forms: { singular: order, plural: orders }
";
    let edit = edit_term(
        "Order",
        TermChanges {
            kind: FieldChange::Unset,
            ..TermChanges::default()
        },
    );
    assert_eq!(
        layout_error(source, &edit),
        "cannot edit the dictionary in place: `kind` does not start its own line"
    );
}

#[test]
fn refuses_an_edit_that_would_change_other_definitions() {
    // The blank line belongs to `kind` (`|+` keeps trailing line breaks), so
    // inserting `ja` right after its text would change its value.
    let source = "\
context: order
terms:
  - name: Order
    kind: |+
      entity

    forms: { singular: order, plural: orders }
";
    let edit = edit_term(
        "Order",
        TermChanges {
            ja: FieldChange::Set("注文".to_owned()),
            ..TermChanges::default()
        },
    );
    assert_eq!(
        layout_error(source, &edit),
        "cannot edit the dictionary in place: the edited file would not read back as the \
         intended dictionary"
    );
}

#[test]
fn edits_a_file_that_starts_with_a_byte_order_mark() {
    let source = "\u{feff}context: 注文\nterms:\n  - name: Order\n    ja: 注文\n    \
                  forms: { singular: order, plural: orders }\n";
    let edit = edit_term(
        "Order",
        TermChanges {
            plural: Some(words("order list")),
            ..TermChanges::default()
        },
    );
    assert_eq!(
        edited(source, &edit),
        source.replace("plural: orders", "plural: order list")
    );
}
