//! End-to-end coverage of per-directory dictionary resolution.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const CONFIG: &str = "\
rules:
  - id: domain-type
    include: [\"**\"]
    target: type
    patterns: [\"{noun}\"]
  - id: domain-method
    include: [\"**\"]
    target: method
    patterns: [\"{verb}\"]
";

const ORDER: &str = "\
context: order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place
        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }
";

const OTHER_ORDER: &str = "\
context: other-order
terms:
  - name: Order
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: cancel
        forms: { base: cancel, s: cancels, past: canceled, pp: canceled, ing: canceling }
";

static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ubiqlint-resolution-cli-{}-{}",
            std::process::id(),
            NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
        ));
        let project = Self(path);
        project.write("ubiqlint.yaml", CONFIG);
        project
    }

    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    fn check(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
            .arg("check")
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

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

#[test]
fn resolves_sibling_contexts_without_a_root_dictionary() {
    let project = Project::new();
    project.write("sales/terms.yaml", ORDER);
    project.write("shipping/terms.yaml", OTHER_ORDER);
    project.write("sales/domain/Order.java", "class Order { void place() {} }");
    project.write(
        "shipping/domain/Order.java",
        "class Order { void cancel() {} }",
    );

    let output = project.check(&[]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(stdout(&output), "");
    assert!(output.stderr.is_empty());
}

#[test]
fn nearest_dictionary_replaces_parent_terms_and_behaviors() {
    let project = Project::new();
    project.write(
        "terms.yaml",
        &format!(
            "{ORDER}  - name: LineItem\n    forms: {{ singular: line item, plural: line items }}\n"
        ),
    );
    project.write("src/terms.yaml", OTHER_ORDER);
    project.write(
        "src/Order.java",
        "class Order { void place() {} void cancel() {} }",
    );
    project.write("src/domain/LineItem.java", "class LineItem {}");
    project.write("test/Order.java", "class Order { void place() {} }");

    let output = project.check(&[]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert_eq!(out.lines().count(), 2, "{out}");
    assert!(out.contains("src/Order.java:1:20: error: `place`"), "{out}");
    assert!(out.contains("(defined behaviors: `cancel`)"), "{out}");
    assert!(
        out.contains("src/domain/LineItem.java:1:7: error: `LineItem`"),
        "{out}"
    );
}

#[test]
fn reports_missing_dictionaries_per_source_and_never_searches_above_the_config() {
    let project = Project::new();
    project.write("terms.yaml", ORDER);
    project.write("nested/ubiqlint.yaml", CONFIG);
    project.write("nested/src/Order.java", "class Order { void place() {} }");
    project.write("nested/src/Other.java", "class Other {}");

    let output = project.check(&["--config", "nested/ubiqlint.yaml"]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert_eq!(out.lines().count(), 2, "{out}");
    for name in ["Order", "Other"] {
        assert!(
            out.contains(&format!(
                "nested/src/{name}.java: error: no `terms.yaml` found"
            )),
            "{out}"
        );
    }
    assert!(out.contains("use `--dictionary`"));
}

#[test]
fn reports_an_invalid_dictionary_once_and_keeps_checking_other_contexts() {
    let project = Project::new();
    project.write("terms.yaml", ORDER);
    project.write("bad/terms.yaml", "context: bad\nterms: wrong\n");
    project.write("bad/Order.java", "class Order { void place() {} }");
    project.write("bad/other/Order.java", "class Order { void place() {} }");
    project.write("ok/Order.java", "class Order { void update() {} }");

    let output = project.check(&[]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert_eq!(out.lines().count(), 2, "{out}");
    assert!(out.contains("bad/terms.yaml:2:8: error:"), "{out}");
    assert!(out.contains("ok/Order.java:1:20: error: `update`"), "{out}");
}

#[test]
fn an_invalid_root_dictionary_is_reported_once_and_does_not_block_nested_contexts() {
    let project = Project::new();
    project.write("terms.yaml", "context: bad\nterms: wrong\n");
    project.write("src/Order.java", "class Order { void place() {} }");
    project.write("src/Other.java", "class Other {}");
    project.write("src/nested/terms.yaml", OTHER_ORDER);
    project.write("src/nested/Order.java", "class Order { void update() {} }");

    let output = project.check(&[]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert_eq!(out.lines().count(), 2, "{out}");
    assert!(out.starts_with("terms.yaml:2:8: error:"), "{out}");
    assert!(
        out.contains("src/nested/Order.java:1:20: error: `update`"),
        "{out}"
    );
    assert!(out.contains("(defined behaviors: `cancel`)"), "{out}");
}

#[test]
fn explicit_dictionary_bypasses_all_automatic_dictionaries() {
    let project = Project::new();
    project.write("terms.yaml", "invalid: root\n");
    project.write("src/terms.yaml", "invalid: local\n");
    project.write("shared/custom.yaml", OTHER_ORDER);
    project.write("src/Order.java", "class Order { void cancel() {} }");

    let output = project.check(&["--dictionary", "shared/custom.yaml"]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(stdout(&output), "");

    // An explicit path is relative to cwd, including with a nested config.
    project.write("src/ubiqlint.yaml", CONFIG);
    let output = project.check(&[
        "--config",
        "src/ubiqlint.yaml",
        "--dictionary",
        "shared/custom.yaml",
    ]);
    assert!(output.status.success(), "{}", stdout(&output));

    let output = project.check(&["--dictionary", "missing.yaml"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).starts_with("missing.yaml: error: cannot read the file:"));
    assert_eq!(stdout(&output).lines().count(), 1);
}

#[test]
fn never_loads_dictionaries_only_used_by_ignored_or_excluded_sources() {
    let project = Project::new();
    project.write(
        "ubiqlint.yaml",
        &format!("exclude: [\"excluded/**\"]\n{CONFIG}"),
    );
    project.write(".gitignore", "ignored/\n");
    project.write("terms.yaml", ORDER);
    project.write("ok/Order.java", "class Order { void place() {} }");
    for directory in ["ignored", "excluded", "unused"] {
        project.write(&format!("{directory}/terms.yaml"), "invalid: dictionary\n");
    }
    for directory in ["ignored", "excluded"] {
        project.write(&format!("{directory}/Bad.java"), "not even Java");
    }

    let output = project.check(&[]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(stdout(&output), "");
    assert!(output.stderr.is_empty());
}

#[test]
fn dictionary_dependencies_are_loaded_even_if_the_dictionary_filename_is_ignored() {
    let project = Project::new();
    project.write(".gitignore", "terms.yaml\n");
    project.write("terms.yaml", ORDER);
    project.write("src/terms.yaml", OTHER_ORDER);
    project.write("src/Order.java", "class Order { void place() {} }");

    let output = project.check(&[]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.contains("(defined behaviors: `cancel`)"), "{out}");
}

#[test]
fn an_empty_project_does_not_require_a_dictionary() {
    let project = Project::new();
    let output = project.check(&[]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(output.stderr, b"warning: no source files were checked\n");
}

#[test]
fn files_outside_every_rule_need_no_dictionary_but_still_get_syntax_checks() {
    let project = Project::new();
    project.write(
        "ubiqlint.yaml",
        &CONFIG.replace("[\"**\"]", "[\"domain/**\"]"),
    );
    project.write("domain/terms.yaml", ORDER);
    project.write("domain/Order.java", "class Order { void place() {} }");
    project.write("infrastructure/Utility.java", "class Utility {}");
    project.write("infrastructure/terms.yaml", "invalid: unused\n");

    let output = project.check(&[]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(stdout(&output), "");
    assert!(output.stderr.is_empty());

    project.write("infrastructure/Utility.java", "class Utility {");
    let output = project.check(&[]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(
        out.contains("infrastructure/Utility.java:1:16: error: syntax error:"),
        "{out}"
    );
}

#[cfg(unix)]
#[test]
fn a_broken_dictionary_link_is_reported_once_without_falling_back() {
    use std::os::unix::fs::symlink;

    let project = Project::new();
    project.write("terms.yaml", ORDER);
    project.write("src/Order.java", "class Order { void place() {} }");
    project.write("src/sub/Order.java", "class Order { void place() {} }");
    symlink("missing", project.0.join("src/terms.yaml")).unwrap();

    let output = project.check(&[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output).lines().count(), 1);
    assert!(stdout(&output).starts_with("src/terms.yaml: error: cannot read the file:"));
}

#[cfg(unix)]
#[test]
fn symlinked_sources_resolve_dictionaries_at_the_visible_path() {
    use std::os::unix::fs::symlink;

    let project = Project::new();
    project.write("target/terms.yaml", ORDER);
    project.write("target/Order.java", "class Order { void place() {} }");
    project.write("visible/terms.yaml", OTHER_ORDER);
    symlink("../target/Order.java", project.0.join("visible/Order.java")).unwrap();

    let output = project.check(&[]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(
        out.starts_with("visible/Order.java:1:20: error: `place`"),
        "{out}"
    );
    assert!(out.contains("(defined behaviors: `cancel`)"), "{out}");
}
