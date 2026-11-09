use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ubiqlint_core::edit::{Edit, EditError};
use ubiqlint_core::{Dictionary, Words};
use ubiqlint_project::{
    CheckRun, Dictionaries, EditFileError, EditOutcome, FileReport, LoadError, Project, Skip,
};

static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);

/// A project in a temporary directory, removed when dropped.
struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ubiqlint-project-{}-{}",
            std::process::id(),
            NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        let project = Self(path);
        project.write("ubiqlint.yaml", CONFIG);
        project
    }

    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    fn open(&self) -> Project {
        Project::open(&self.0.join("ubiqlint.yaml")).unwrap()
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const CONFIG: &str = "\
exclude: [\"src/generated/**\"]
rules:
  - id: domain-method
    include: [\"src/**\"]
    target: method
    patterns:
      - \"{verb}\"
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

/// Checks `text` as the file at `relative` in a run of its own.
fn check_one(project: &Project, relative: &str, text: &str) -> FileReport {
    let mut dictionaries = project.nearest_dictionaries();
    let relative = Path::new(relative);
    let mut run = CheckRun::new(project, &mut dictionaries, [relative]);
    run.check(relative, text)
}

fn messages(report: FileReport) -> Vec<String> {
    match report {
        FileReport::Checked(diagnostics) => diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.to_string())
            .collect(),
        other => panic!("expected a checked file, got {other:?}"),
    }
}

#[test]
fn checks_the_given_text_rather_than_the_file_on_disk() {
    let temp = TempProject::new();
    temp.write("terms.yaml", ORDER);
    temp.write(
        "src/Order.java",
        "class Order { void place() {} void update() {} }",
    );
    let project = temp.open();

    // A draft that does not exist on disk yet.
    let draft = messages(check_one(
        &project,
        "src/Draft.java",
        "class Order { void cancel() {} }",
    ));
    assert_eq!(draft.len(), 1, "{draft:?}");
    assert!(draft[0].starts_with("1:20: `cancel`"), "{draft:?}");

    // The text given replaces what is on disk.
    let fixed = messages(check_one(
        &project,
        "src/Order.java",
        "class Order { void place() {} }",
    ));
    assert!(fixed.is_empty(), "{fixed:?}");
}

#[test]
fn tells_files_it_does_not_check_from_files_without_violations() {
    let temp = TempProject::new();
    temp.write("terms.yaml", ORDER);
    let project = temp.open();
    let text = "class Order { void update() {} }";

    assert!(matches!(
        check_one(&project, "src/generated/Order.java", text),
        FileReport::Skipped(Skip::Excluded)
    ));
    assert!(matches!(
        check_one(&project, "src/Order.kt", text),
        FileReport::Skipped(Skip::UnsupportedLanguage)
    ));
    assert!(matches!(
        check_one(&project, "test/Order.java", text),
        FileReport::NoRules
    ));
    assert!(matches!(
        check_one(&project, "src/Order.java", "class Order {"),
        FileReport::SyntaxErrors(_)
    ));
    assert_eq!(
        project.skip(Path::new("src/generated/Order.java")),
        Some(Skip::Excluded)
    );
    assert_eq!(project.skip(Path::new("src/Order.java")), None);
}

#[test]
fn reports_missing_and_invalid_dictionaries_only_for_files_that_parse() {
    let temp = TempProject::new();
    temp.write("src/broken/terms.yaml", "context: broken\nterms: wrong\n");
    let project = temp.open();

    assert!(matches!(
        check_one(&project, "src/Order.java", "class Order {}"),
        FileReport::MissingDictionary
    ));
    assert!(matches!(
        check_one(&project, "src/broken/Order.java", "class Order {}"),
        FileReport::InvalidDictionary(_)
    ));
    // Syntax errors come first, whatever the dictionary.
    assert!(matches!(
        check_one(&project, "src/broken/Order.java", "class Order {"),
        FileReport::SyntaxErrors(_)
    ));
}

#[test]
fn shares_a_run_between_sources_of_different_contexts() {
    let temp = TempProject::new();
    temp.write("terms.yaml", ORDER);
    temp.write(
        "src/billing/terms.yaml",
        &ORDER
            .replace("context: order", "context: billing")
            .replace("place", "issue")
            .replace("placing", "issuing")
            .replace("places", "issues")
            .replace("placed", "issued"),
    );
    let project = temp.open();
    let sources = [
        Path::new("src/Order.java"),
        Path::new("src/billing/Order.java"),
    ];
    let mut dictionaries = project.nearest_dictionaries();
    let mut run = CheckRun::new(&project, &mut dictionaries, sources);

    let text = "class Order { void place() {} void issue() {} }";
    let order = messages(run.check(sources[0], text));
    let billing = messages(run.check(sources[1], text));
    assert_eq!(order.len(), 1, "{order:?}");
    assert!(order[0].contains("`issue`"), "{order:?}");
    assert_eq!(billing.len(), 1, "{billing:?}");
    assert!(billing[0].contains("`place`"), "{billing:?}");
    assert!(
        billing[0].contains("(defined behaviors: `issue`)"),
        "{billing:?}"
    );
}

#[test]
fn a_fixed_dictionary_applies_to_every_source() {
    let temp = TempProject::new();
    temp.write("src/terms.yaml", "invalid: dictionary\n");
    let project = temp.open();
    let mut dictionaries = Dictionaries::Fixed {
        path: PathBuf::from("shared/terms.yaml"),
        dictionary: Dictionary::from_yaml_str(ORDER).unwrap(),
    };
    let relative = Path::new("src/Order.java");
    let mut run = CheckRun::new(&project, &mut dictionaries, [relative]);

    // The dictionary is identified by the path it was given with.
    let found = messages(run.check(relative, "class Helper { void place() {} }"));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0]
            .ends_with("the enclosing type `Helper` is not in the dictionary `shared/terms.yaml`"),
        "{found:?}"
    );
}

#[test]
fn walks_scopes_with_the_ignore_files_above_them() {
    let temp = TempProject::new();
    temp.write(".gitignore", "ignored/\n");
    temp.write("src/a/One.java", "class One {}");
    temp.write("src/a/ignored/Two.java", "class Two {}");
    temp.write("src/b/Three.java", "class Three {}");
    temp.write("src/generated/Four.java", "class Four {}");
    temp.write("src/ab/Five.java", "class Five {}");
    let project = temp.open();

    let whole = project.walk(&[]);
    assert!(whole.errors.is_empty(), "{:?}", whole.errors);
    assert_eq!(
        whole.files,
        // Hidden files such as `.gitignore` are never collected.
        [
            "src/a/One.java",
            "src/ab/Five.java",
            "src/b/Three.java",
            "ubiqlint.yaml",
        ]
        .map(PathBuf::from)
    );

    let scoped = project.walk(&[PathBuf::from("src/a")]);
    assert_eq!(scoped.files, [PathBuf::from("src/a/One.java")]);

    let excluded = project.walk(&[PathBuf::from("src/generated")]);
    assert!(excluded.files.is_empty(), "{:?}", excluded.files);
}

#[test]
fn turns_given_paths_into_paths_under_the_root() {
    let temp = TempProject::new();
    let project = temp.open();

    assert_eq!(
        project.relative(&temp.0.join("src/Order.java")),
        Some(PathBuf::from("src/Order.java"))
    );
    assert_eq!(project.relative(&temp.0), Some(PathBuf::new()));
    assert_eq!(project.relative(&temp.0.join("../Order.java")), None);
}

#[test]
fn opening_reports_unreadable_and_invalid_configurations() {
    let temp = TempProject::new();
    temp.write("invalid/ubiqlint.yaml", "rules: wrong\n");

    assert!(matches!(
        Project::open(&temp.0.join("missing/ubiqlint.yaml")),
        Err(LoadError::Read(_))
    ));
    assert!(matches!(
        Project::open(&temp.0.join("invalid/ubiqlint.yaml")),
        Err(LoadError::Invalid(_))
    ));
}

#[test]
fn edits_a_dictionary_on_disk_only_when_the_edit_succeeds() {
    let temp = TempProject::new();
    temp.write("terms.yaml", ORDER);
    let path = temp.0.join("terms.yaml");
    let remove = Edit::RemoveBehavior {
        term: "Order".to_owned(),
        verb: Words::parse("place").unwrap(),
    };

    assert_eq!(
        ubiqlint_project::edit_dictionary(&path, &remove).unwrap(),
        EditOutcome::Changed
    );
    assert!(!fs::read_to_string(&path).unwrap().contains("place"));

    let edited = fs::read_to_string(&path).unwrap();
    assert!(matches!(
        ubiqlint_project::edit_dictionary(&path, &remove),
        Err(EditFileError::Edit(EditError::Rejected(_)))
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), edited);

    assert!(matches!(
        ubiqlint_project::edit_dictionary(&temp.0.join("missing.yaml"), &remove),
        Err(EditFileError::Read(_))
    ));
}
