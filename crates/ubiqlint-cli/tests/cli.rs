use std::fs;
use std::process::{Command, Output};

/// Runs `ubiqlint` from the crate directory so fixture paths stay short.
fn ubiqlint(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to run ubiqlint")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is not UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is not UTF-8")
}

#[test]
fn prints_version() {
    let output = ubiqlint(&["--version"]);

    assert!(output.status.success());
    assert_eq!(
        stdout(&output).trim(),
        format!("ubiqlint {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn validate_accepts_valid_files() {
    let output = ubiqlint(&[
        "validate",
        "tests/fixtures/valid/terms.yaml",
        "tests/fixtures/valid/ubiqlint.yaml",
    ]);

    assert!(output.status.success());
    assert_eq!(stdout(&output), "");
}

#[test]
fn validate_reports_every_problem_with_its_location() {
    let output = ubiqlint(&[
        "validate",
        "tests/fixtures/invalid/terms.yaml",
        "tests/fixtures/invalid/ubiqlint.yaml",
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "\
tests/fixtures/invalid/terms.yaml:5:5: error: unknown field `forbidden_verbs` in a term, \
expected one of `name`, `kind`, `ja`, `description`, `forms`, `behaviors`
tests/fixtures/invalid/terms.yaml:8:16: error: missing required field `pp` in `forms` of a behavior
tests/fixtures/invalid/terms.yaml:8:16: error: missing required field `ing` in `forms` of a behavior
tests/fixtures/invalid/ubiqlint.yaml:4:13: error: unknown target `class`, \
expected one of `type`, `method`, `field`, `parameter`, `variable`
tests/fixtures/invalid/ubiqlint.yaml:6:9: error: unknown placeholder `{role}`: \
expected `{noun}`, `{verb}`, `{any}` or a vocabulary defined in `vocabularies`
"
    );
}

#[test]
fn validate_keeps_each_problem_on_one_line() {
    let output = ubiqlint(&["validate", "tests/fixtures/multiline/terms.yaml"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "tests/fixtures/multiline/terms.yaml:4:24: error: invalid `singular`: \
         `order\\nitem` is not a valid word sequence: use lowercase letters and digits, \
         with a single space between words (e.g. `line item`)\n"
    );
}

#[test]
fn validate_rejects_files_of_unknown_kind() {
    let output = ubiqlint(&["validate", "Cargo.toml"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "Cargo.toml: error: cannot tell what kind of file this is: \
         name a dictionary `terms.yaml` and a configuration file `ubiqlint.yaml`\n"
    );
}

#[test]
fn validate_reports_unreadable_files() {
    let output = ubiqlint(&["validate", "tests/fixtures/missing/terms.yaml"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output)
            .starts_with("tests/fixtures/missing/terms.yaml: error: cannot read the file: "),
        "unexpected output: {}",
        stdout(&output)
    );
}

#[test]
fn validate_requires_a_file() {
    let output = ubiqlint(&["validate"]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn check_accepts_a_project_without_violations() {
    let output = ubiqlint(&["check", "--config", "tests/fixtures/check/ok/ubiqlint.yaml"]);

    assert!(output.status.success());
    assert_eq!(stdout(&output), "");
}

#[test]
fn check_uses_default_files_in_the_current_directory() {
    let fixture = "tests/fixtures/check/bad";
    let explicit = ubiqlint(&["check", "--config", &format!("{fixture}/ubiqlint.yaml")]);
    let output = Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
        .arg("check")
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture))
        .output()
        .expect("failed to run ubiqlint");

    // The default config has an empty parent path. It must still load the
    // dictionary and source files, reporting the same violations relative to cwd.
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        stdout(&explicit).replace(&format!("{fixture}/"), "")
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn check_reports_every_violation_with_its_location() {
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/bad/ubiqlint.yaml",
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "\
tests/fixtures/check/bad/src/main/java/com/example/order/domain/Broken.java:3:22: error: syntax error: missing `}`
tests/fixtures/check/bad/src/main/java/com/example/order/domain/Order.java:7:17: error: `updateOrder` does not match pattern `{verb}{noun}` of rule `domain-method`: `update` is not a defined behavior of `Order` (defined behaviors: `place`, `cancel`)
tests/fixtures/check/bad/src/main/java/com/example/order/domain/PurchaseService.java:3:14: error: `PurchaseService` does not match any pattern of rule `domain-type`: expected one of `{noun}`, `{noun}{role}`, `{noun}{verb:pp}`. If `PurchaseService` is a new term or behavior of the ubiquitous language, define it in `tests/fixtures/check/bad/terms.yaml` first
"
    );
}

#[test]
fn check_uses_another_dictionary_when_asked() {
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/bad/ubiqlint.yaml",
        "--dictionary",
        "tests/fixtures/check/bad/extra-terms.yaml",
    ]);

    // `updateOrder` is allowed by the extra dictionary; the rest remains.
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "\
tests/fixtures/check/bad/src/main/java/com/example/order/domain/Broken.java:3:22: error: syntax error: missing `}`
tests/fixtures/check/bad/src/main/java/com/example/order/domain/PurchaseService.java:3:14: error: `PurchaseService` does not match any pattern of rule `domain-type`: expected one of `{noun}`, `{noun}{role}`, `{noun}{verb:pp}`. If `PurchaseService` is a new term or behavior of the ubiquitous language, define it in `tests/fixtures/check/bad/extra-terms.yaml` first
"
    );
}

#[test]
fn check_reports_import_and_naming_violations_together() {
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/imports/ubiqlint.yaml",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let prefix = "tests/fixtures/check/imports/src/domain";
    assert_eq!(stdout(&output), format!("\
{prefix}/Bad.java:3:1: error: external import `com.vendor.Order` conflicts with dictionary term `Order` in `tests/fixtures/check/imports/terms.yaml` under rule `domain-imports`: use the fully qualified name `com.vendor.Order` instead
{prefix}/Bad.java:4:1: error: external wildcard import `com.vendor.*` is forbidden by rule `domain-imports`: import non-conflicting names explicitly, or use fully qualified names
{prefix}/Bad.java:5:1: error: external import `com.vendor.Container.LineItem` conflicts with dictionary term `LineItem` in `tests/fixtures/check/imports/terms.yaml` under rule `domain-imports`: use the fully qualified name `com.vendor.Container.LineItem` instead
{prefix}/Bad.java:6:1: error: external wildcard import `com.vendor.Order.*` is forbidden by rule `domain-imports`: import non-conflicting names explicitly, or use fully qualified names
{prefix}/Bad.java:7:1: error: external import `com.example.ordering.Order` conflicts with dictionary term `Order` in `tests/fixtures/check/imports/terms.yaml` under rule `domain-imports`: use the fully qualified name `com.example.ordering.Order` instead
{prefix}/Bad.java:9:7: error: `Purchase` does not match any pattern of rule `domain-type`: expected one of `{{noun}}`. If `Purchase` is a new term or behavior of the ubiquitous language, define it in `tests/fixtures/check/imports/terms.yaml` first
{prefix}/Broken.java:2:14: error: syntax error: missing `}}`
"));
    assert_eq!(stderr(&output), "");
}

#[test]
fn import_collisions_use_the_dictionary_override() {
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/imports/ubiqlint.yaml",
        "--dictionary",
        "tests/fixtures/check/imports/empty-terms.yaml",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert!(!out.contains("conflicts with dictionary term"), "{out}");
    assert_eq!(out.matches("external wildcard import").count(), 2);
    assert!(!out.contains("Allowed.java:3:"), "{out}");
    assert!(!out.contains("OrderDto.java"), "{out}");
    assert!(!out.contains("Generated.java"), "{out}");
}

#[test]
fn validate_accepts_import_rules() {
    let output = ubiqlint(&["validate", "tests/fixtures/check/imports/ubiqlint.yaml"]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(stdout(&output), "");
}

#[test]
fn check_reports_an_unreadable_config() {
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/missing/ubiqlint.yaml",
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).starts_with(
            "tests/fixtures/check/missing/ubiqlint.yaml: error: cannot read the file: "
        ),
        "unexpected output: {}",
        stdout(&output)
    );
}

#[test]
fn check_reports_config_and_dictionary_errors_in_one_run() {
    let output = ubiqlint(&["check", "--config", "tests/fixtures/invalid/ubiqlint.yaml"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "\
tests/fixtures/invalid/ubiqlint.yaml:4:13: error: unknown target `class`, \
expected one of `type`, `method`, `field`, `parameter`, `variable`
tests/fixtures/invalid/ubiqlint.yaml:6:9: error: unknown placeholder `{role}`: \
expected `{noun}`, `{verb}`, `{any}` or a vocabulary defined in `vocabularies`
tests/fixtures/invalid/terms.yaml:5:5: error: unknown field `forbidden_verbs` in a term, \
expected one of `name`, `kind`, `ja`, `description`, `forms`, `behaviors`
tests/fixtures/invalid/terms.yaml:8:16: error: missing required field `pp` in `forms` of a behavior
tests/fixtures/invalid/terms.yaml:8:16: error: missing required field `ing` in `forms` of a behavior
"
    );
}

#[test]
fn check_accepts_the_sample_project() {
    let output = ubiqlint(&[
        "check",
        "--config",
        "../../examples/order-service/ubiqlint.yaml",
    ]);

    assert!(
        output.status.success(),
        "unexpected output: {}",
        stdout(&output)
    );
    assert_eq!(stdout(&output), "");
}

#[test]
fn check_never_reads_files_matched_by_exclude() {
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/excluded/ubiqlint.yaml",
    ]);

    assert!(
        output.status.success(),
        "unexpected output: {}",
        stdout(&output)
    );
    assert_eq!(stdout(&output), "");
}

#[test]
fn check_exclude_file_glob_does_not_swallow_directories() {
    // `src/*` excludes `src/Main.java`, but must not prune the directory
    // `src/domain`, so `src/domain/Bad.java` is still checked.
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/exclude-glob/ubiqlint.yaml",
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "tests/fixtures/check/exclude-glob/src/domain/Bad.java:2:17: error: `update` does not \
         match pattern `{verb}` of rule `domain-method`: `{verb}` is not bound to a term: the \
         enclosing type `Bad` is not in the dictionary `tests/fixtures/check/exclude-glob/terms.yaml`\n"
    );
}

#[test]
fn check_respects_gitignore() {
    // This fixture lives inside this repository, so it covers the in-repo
    // case. The tarball case (no `.git` anywhere above) is covered by
    // `check_applies_gitignore_outside_a_repository`.
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/gitignored/ubiqlint.yaml",
    ]);

    assert!(
        output.status.success(),
        "unexpected output: {}",
        stdout(&output)
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
}

// A symlink committed to git becomes a plain file on Windows checkouts
// (core.symlinks=false), so the link is created at test time instead.
#[cfg(unix)]
#[test]
fn check_follows_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = std::env::temp_dir().join(format!("ubiqlint-check-symlink-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("real")).unwrap();
    fs::create_dir_all(dir.join("linked")).unwrap();
    fs::write(
        dir.join("ubiqlint.yaml"),
        "rules:\n  - id: domain-method\n    include: [\"linked/**\"]\n    target: method\n    patterns:\n      - \"{verb}\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("terms.yaml"),
        "context: order\nterms:\n  - name: Order\n    forms: { singular: order, plural: orders }\n    behaviors:\n      - verb: place\n        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }\n",
    )
    .unwrap();
    fs::write(
        dir.join("real/Bad.java"),
        "public class Bad {\n    public void update() {\n    }\n}\n",
    )
    .unwrap();
    symlink("../real/Bad.java", dir.join("linked/Bad.java")).unwrap();

    let config = dir.join("ubiqlint.yaml");
    let output = ubiqlint(&["check", "--config", config.to_str().unwrap()]);
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert!(
        out.contains(
            "linked/Bad.java:2:17: error: `update` does not match pattern `{verb}` of rule `domain-method`"
        ),
        "unexpected output: {out}"
    );
}

#[test]
fn check_reports_a_file_that_is_not_utf8_and_keeps_going() {
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/nonutf8/ubiqlint.yaml",
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "tests/fixtures/check/nonutf8/src/Legacy.java: error: cannot read the file: \
         stream did not contain valid UTF-8\n"
    );
}

#[test]
fn check_warns_when_no_source_files_were_checked() {
    let output = ubiqlint(&[
        "check",
        "--config",
        "tests/fixtures/check/nofiles/ubiqlint.yaml",
    ]);

    // Warnings go to stderr and never fail the run.
    assert!(output.status.success());
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "warning: no source files were checked\n");
}

/// Writes a minimal project: a valid `Order.java` under `src/`, plus a
/// violating `Bad.java` under `ignored/`, and a `.gitignore` for `ignored/`.
fn write_project_with_gitignore(dir: &std::path::Path) {
    fs::create_dir_all(dir.join("src/ignored")).unwrap();
    fs::write(
        dir.join("ubiqlint.yaml"),
        "rules:\n  - id: domain-method\n    include: [\"src/**\"]\n    target: method\n    patterns:\n      - \"{verb}\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("terms.yaml"),
        "context: order\nterms:\n  - name: Order\n    forms: { singular: order, plural: orders }\n    behaviors:\n      - verb: place\n        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }\n",
    )
    .unwrap();
    fs::write(
        dir.join("src/Order.java"),
        "public class Order {\n    public void place() {\n    }\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("src/ignored/Bad.java"),
        "public class Bad {\n    public void update() {\n    }\n}\n",
    )
    .unwrap();
    fs::write(dir.join(".gitignore"), "ignored/\n").unwrap();
}

// Outside a git repository (a tarball), the tree's own `.gitignore`
// applies. The temp dir is normally outside any repository.
#[test]
fn check_applies_gitignore_outside_a_repository() {
    let dir =
        std::env::temp_dir().join(format!("ubiqlint-gitignore-tarball-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    write_project_with_gitignore(&dir);

    let config = dir.join("ubiqlint.yaml");
    let output = ubiqlint(&["check", "--config", config.to_str().unwrap()]);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "unexpected output: {} / {}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
}

// A `.gitignore` above the repository (e.g. `*` in a parent directory) must
// not apply to files inside a project that has its own `.git`.
#[test]
fn check_ignores_gitignore_above_the_repository() {
    let base =
        std::env::temp_dir().join(format!("ubiqlint-gitignore-parent-{}", std::process::id()));
    let dir = base.join("proj");
    let _ = fs::remove_dir_all(&base);
    write_project_with_gitignore(&dir);
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(base.join(".gitignore"), "*\n").unwrap();

    let config = dir.join("ubiqlint.yaml");
    let output = ubiqlint(&["check", "--config", config.to_str().unwrap()]);
    let _ = fs::remove_dir_all(&base);

    // `src/Order.java` is still checked, so no warning and no violations.
    assert!(
        output.status.success(),
        "unexpected output: {} / {}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
}

// A dangling symlink to a source file is a walk error on a checkable file:
// reported with the failed path first, and the path is not repeated inside
// the message.
#[cfg(unix)]
#[test]
fn check_reports_walk_errors_on_source_files_without_repeating_the_path() {
    use std::os::unix::fs::symlink;

    let dir = std::env::temp_dir().join(format!("ubiqlint-check-dangling-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("ubiqlint.yaml"),
        "rules:\n  - id: domain-method\n    include: [\"src/**\"]\n    target: method\n    patterns:\n      - \"{verb}\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("terms.yaml"),
        "context: order\nterms:\n  - name: Order\n    forms: { singular: order, plural: orders }\n    behaviors:\n      - verb: place\n        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }\n",
    )
    .unwrap();
    fs::write(
        dir.join("src/Order.java"),
        "public class Order {\n    public void place() {\n    }\n}\n",
    )
    .unwrap();
    symlink("nowhere", dir.join("src/Gone.java")).unwrap();

    let config = dir.join("ubiqlint.yaml");
    let output = ubiqlint(&["check", "--config", config.to_str().unwrap()]);
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    let gone = dir.join("src/Gone.java");
    assert!(
        out.contains(&format!(
            "{}: error: cannot walk: No such file or directory",
            gone.display()
        )),
        "unexpected output: {out}"
    );
    assert!(
        !out.contains("IO error for operation"),
        "path repeated inside the message: {out}"
    );
}

// Symlink loops and dangling links can only be created at test time.
#[cfg(unix)]
#[test]
fn check_ignores_walk_errors_under_excluded_paths_and_non_sources() {
    use std::os::unix::fs::symlink;

    let dir = std::env::temp_dir().join(format!("ubiqlint-check-walkerr-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join("legacy")).unwrap();
    fs::write(
        dir.join("ubiqlint.yaml"),
        "exclude: [\"legacy/**\"]\nrules:\n  - id: domain-method\n    include: [\"src/**\"]\n    target: method\n    patterns:\n      - \"{verb}\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("terms.yaml"),
        "context: order\nterms:\n  - name: Order\n    forms: { singular: order, plural: orders }\n    behaviors:\n      - verb: place\n        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }\n",
    )
    .unwrap();
    fs::write(
        dir.join("src/Order.java"),
        "public class Order {\n    public void place() {\n    }\n}\n",
    )
    .unwrap();
    // A symlink loop inside the excluded directory, and a dangling
    // non-source link in the checked tree: neither may fail the run.
    symlink(".", dir.join("legacy/loop")).unwrap();
    symlink("nowhere", dir.join("src/README")).unwrap();

    let config = dir.join("ubiqlint.yaml");
    let output = ubiqlint(&["check", "--config", config.to_str().unwrap()]);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "unexpected output: {}",
        stdout(&output)
    );
    assert_eq!(stdout(&output), "");
}

#[test]
fn check_exits_1_instead_of_panicking_when_the_output_pipe_is_closed() {
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
        .args([
            "check",
            "--config",
            "tests/fixtures/check/bad/ubiqlint.yaml",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to run ubiqlint");
    // Close the read end immediately, so the child's writes hit EPIPE.
    drop(child.stdout.take());
    let status = child.wait().expect("failed to wait for ubiqlint");
    assert_eq!(status.code(), Some(1));
}
