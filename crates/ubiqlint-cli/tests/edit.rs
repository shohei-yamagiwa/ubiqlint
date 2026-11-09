//! End-to-end coverage of the commands that edit dictionaries.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const ORDER: &str = "\
# 注文コンテキスト
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }  # 集約
    behaviors:
      - verb: place
        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }
";

static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ubiqlint-edit-cli-{}-{}",
            std::process::id(),
            NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
        ));
        let project = Self(path);
        project.write("terms.yaml", ORDER);
        project
    }

    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.0.join(path)).unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("failed to run ubiqlint")
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is not UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is not UTF-8")
}

#[test]
fn adds_a_term_and_keeps_the_rest_of_the_file() {
    let project = Project::new();
    let output = project.run(&[
        "term",
        "add",
        "Shipment",
        "--plural",
        "shipments",
        "--ja",
        "出荷",
    ]);

    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(stdout(&output), "terms.yaml: added term `Shipment`\n");
    assert_eq!(
        project.read("terms.yaml"),
        format!(
            "{ORDER}  - name: Shipment\n    ja: 出荷\n    \
             forms: {{ singular: shipment, plural: shipments }}\n"
        )
    );
    // Nothing is left behind next to the dictionary.
    let entries: Vec<_> = fs::read_dir(&project.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, ["terms.yaml"]);
}

#[test]
fn edits_the_dictionary_given_by_path() {
    let project = Project::new();
    project.write("src/order/terms.yaml", ORDER);
    let output = project.run(&[
        "behavior",
        "edit",
        "Order",
        "place",
        "--verb",
        "submit",
        "--s",
        "submits",
        "--past",
        "submitted",
        "--pp",
        "submitted",
        "--ing",
        "submitting",
        "--dictionary",
        "src/order/terms.yaml",
    ]);

    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(
        stdout(&output),
        "src/order/terms.yaml: updated behavior `place` of term `Order` (renamed to `submit`)\n"
    );
    assert_eq!(
        project.read("src/order/terms.yaml"),
        ORDER.replace("verb: place", "verb: submit").replace(
            "base: place, s: places, past: placed, pp: placed, ing: placing",
            "base: submit, s: submits, past: submitted, pp: submitted, ing: submitting"
        )
    );
    assert_eq!(project.read("terms.yaml"), ORDER);
}

#[test]
fn reports_every_reason_to_refuse_an_edit_and_leaves_the_file_alone() {
    let project = Project::new();
    let output = project.run(&["term", "add", "ORDER", "--plural", "orders"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "\
terms.yaml: error: term `ORDER` clashes with term `Order` (line 4): term names are compared \
ignoring case
terms.yaml: error: the form `order` is already used by term `Order` (line 4)
terms.yaml: error: the form `orders` is already used by term `Order` (line 4)
"
    );
    assert_eq!(project.read("terms.yaml"), ORDER);

    let output = project.run(&["behavior", "remove", "Order", "ship"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "terms.yaml: error: no behavior `ship` on term `Order` (defined behaviors: `place`)\n"
    );
}

#[test]
fn rejects_malformed_arguments_as_usage_errors() {
    let project = Project::new();
    let output = project.run(&["term", "add", "Purchase", "--plural", "Purchases"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).starts_with(
            "error: invalid value 'Purchases' for '--plural <WORDS>': `Purchases` is not a \
             valid word sequence"
        ),
        "{}",
        stderr(&output)
    );
    let output = project.run(&["term", "add", "Line Item", "--plural", "line items"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("invalid term name `Line Item`"),
        "{}",
        stderr(&output)
    );
    // An edit must change something, and cannot both set and remove a field.
    assert_eq!(
        project.run(&["term", "edit", "Order"]).status.code(),
        Some(2)
    );
    let output = project.run(&["term", "edit", "Order", "--kind", "entity", "--unset-kind"]);
    assert_eq!(output.status.code(), Some(2));
    let output = project.run(&["term", "edit", "Order", "--kind", " "]);
    assert_eq!(output.status.code(), Some(2));
    // Every form of a new behavior is required.
    let output = project.run(&["behavior", "add", "Order", "cancel", "--s", "cancels"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(project.read("terms.yaml"), ORDER);
}

#[test]
fn reports_an_edit_that_changes_nothing() {
    let project = Project::new();
    let output = project.run(&["term", "edit", "Order", "--plural", "orders"]);

    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "terms.yaml: no changes: term `Order` already has these values\n"
    );
    assert_eq!(project.read("terms.yaml"), ORDER);
}

#[test]
fn refuses_to_edit_an_invalid_or_missing_dictionary() {
    let project = Project::new();
    let invalid = "context: order\nterms:\n  - name: Order\n";
    project.write("terms.yaml", invalid);
    let output = project.run(&["term", "remove", "Order"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "\
terms.yaml:3:5: error: missing required field `forms` in a term
terms.yaml: error: cannot edit an invalid dictionary; fix the problems above first
"
    );
    assert_eq!(project.read("terms.yaml"), invalid);

    let output = project.run(&[
        "term",
        "remove",
        "Order",
        "--dictionary",
        "missing/terms.yaml",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).starts_with("missing/terms.yaml: error: cannot read the file: "),
        "{}",
        stdout(&output)
    );
    assert!(!project.0.join("missing").exists());
}

#[test]
fn defining_a_behavior_resolves_a_violation() {
    let project = Project::new();
    project.write(
        "ubiqlint.yaml",
        "\
rules:
  - id: domain-method
    include: [\"**\"]
    target: method
    patterns: [\"{verb}\"]
",
    );
    project.write(
        "src/Order.java",
        "class Order {\n    void place() {}\n    void ship() {}\n}\n",
    );
    let output = project.run(&["check"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("`ship` is not a defined behavior of `Order`"),
        "{}",
        stdout(&output)
    );

    let output = project.run(&[
        "behavior", "add", "Order", "ship", "--s", "ships", "--past", "shipped", "--pp", "shipped",
        "--ing", "shipping",
    ]);
    assert!(output.status.success(), "{}", stdout(&output));
    let output = project.run(&["check"]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(stdout(&output), "");
}

#[cfg(unix)]
#[test]
fn keeps_symlinks_and_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let project = Project::new();
    project.write("shared/terms.yaml", ORDER);
    let shared = project.0.join("shared/terms.yaml");
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o640)).unwrap();
    fs::create_dir_all(project.0.join("order")).unwrap();
    symlink("../shared/terms.yaml", project.0.join("order/terms.yaml")).unwrap();

    let output = project.run(&[
        "term",
        "remove",
        "Order",
        "--dictionary",
        "order/terms.yaml",
    ]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert!(
        fs::symlink_metadata(project.0.join("order/terms.yaml"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        project.read("shared/terms.yaml"),
        "# 注文コンテキスト\ncontext: order\nterms: []\n"
    );
    assert_eq!(
        fs::metadata(&shared).unwrap().permissions().mode() & 0o777,
        0o640
    );
}
