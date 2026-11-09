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
tests/fixtures/check/bad/src/main/java/com/example/order/domain/PurchaseService.java:3:14: error: `PurchaseService` does not match any pattern of rule `domain-type`: expected one of `{noun}`, `{noun}{role}`, `{noun}{verb:pp}`. If `PurchaseService` is a new term or behavior of the ubiquitous language, define it in `terms.yaml` first
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
tests/fixtures/check/bad/src/main/java/com/example/order/domain/PurchaseService.java:3:14: error: `PurchaseService` does not match any pattern of rule `domain-type`: expected one of `{noun}`, `{noun}{role}`, `{noun}{verb:pp}`. If `PurchaseService` is a new term or behavior of the ubiquitous language, define it in `terms.yaml` first
"
    );
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
fn check_respects_gitignore() {
    // The fixture has no `.git` directory, so this also covers sources
    // distributed as a tarball.
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

    assert!(output.status.success());
    assert_eq!(stdout(&output), "warning: no source files were checked\n");
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
