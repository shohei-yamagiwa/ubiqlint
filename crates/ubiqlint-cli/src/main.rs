use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{ArgGroup, Args, Parser, Subcommand};
use ubiqlint_core::dictionary::validate_term_name;
use ubiqlint_core::{
    BehaviorChanges, CONFIG_FILE_NAME, Config, DICTIONARY_FILE_NAME, Diagnostic, Dictionary,
    DictionaryResolver, Edit, EditError, FieldChange, NewTerm, ResolutionError, TermChanges,
    VerbForms, Words,
};
use ubiqlint_project::{
    CheckRun, Dictionaries, EditFileError, EditOutcome, FileReport, LoadError, Project, Skip,
};

/// A linter for a ubiquitous language.
#[derive(Parser)]
#[command(name = "ubiqlint", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check that dictionaries (`terms.yaml`) and configuration files
    /// (`ubiqlint.yaml`) are well-formed
    Validate {
        /// Files to check. Each file's kind is decided by its name.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Check the source files under a configuration file's directory
    /// against its rules
    Check {
        /// The configuration file to apply
        #[arg(long, default_value = CONFIG_FILE_NAME)]
        config: PathBuf,
        /// Use this dictionary for every source file instead of resolving
        /// the nearest `terms.yaml` up to the configuration directory
        #[arg(long)]
        dictionary: Option<PathBuf>,
        /// Files or directories to check, under the configuration file's
        /// directory. Defaults to everything under it
        #[arg(value_name = "PATHS")]
        paths: Vec<PathBuf>,
    },
    /// Add, edit or remove a term of a dictionary
    #[command(subcommand)]
    Term(TermCommand),
    /// Add, edit or remove a behavior of a term
    #[command(subcommand)]
    Behavior(BehaviorCommand),
}

/// The dictionary an editing command changes.
#[derive(Args)]
struct DictionaryFile {
    /// The dictionary to edit. It must exist and be valid; it is rewritten
    /// only if the edited dictionary is valid
    #[arg(long, value_name = "PATH", default_value = DICTIONARY_FILE_NAME)]
    dictionary: PathBuf,
}

#[derive(Subcommand)]
enum TermCommand {
    /// Add a term, without behaviors, after the existing terms
    Add {
        /// The canonical spelling, such as `LineItem`. The singular form
        /// is its words (`line item`)
        #[arg(value_parser = parse_term_name)]
        name: String,
        /// The plural form, in lowercase words (e.g. `line items`)
        #[arg(long, value_name = "WORDS", value_parser = parse_words)]
        plural: Words,
        /// The kind of term, such as `entity` or `value_object`
        #[arg(long, value_name = "TEXT", value_parser = parse_text)]
        kind: Option<String>,
        /// A Japanese label, such as `注文`
        #[arg(long, value_name = "TEXT", value_parser = parse_text)]
        ja: Option<String>,
        /// What the term means
        #[arg(long, value_name = "TEXT", value_parser = parse_text)]
        description: Option<String>,
        #[command(flatten)]
        file: DictionaryFile,
    },
    /// Change fields of a term. Give at least one change
    #[command(
        group(ArgGroup::new("changes").required(true).multiple(true)),
        override_usage = "ubiqlint term edit [OPTIONS] <NAME>"
    )]
    Edit {
        /// The name of the term to edit
        name: String,
        /// Rename the term. The singular form follows the new name; give
        /// `--plural` too if the singular form changes
        #[arg(
            long = "name",
            id = "new_name",
            value_name = "NAME",
            group = "changes",
            value_parser = parse_term_name
        )]
        new_name: Option<String>,
        /// The new plural form, in lowercase words
        #[arg(long, value_name = "WORDS", group = "changes", value_parser = parse_words)]
        plural: Option<Words>,
        /// Set the kind of term
        #[arg(long, value_name = "TEXT", group = "changes", value_parser = parse_text)]
        kind: Option<String>,
        /// Remove the kind of term
        #[arg(long, group = "changes", conflicts_with = "kind")]
        unset_kind: bool,
        /// Set the Japanese label
        #[arg(long, value_name = "TEXT", group = "changes", value_parser = parse_text)]
        ja: Option<String>,
        /// Remove the Japanese label
        #[arg(long, group = "changes", conflicts_with = "ja")]
        unset_ja: bool,
        /// Set the description
        #[arg(long, value_name = "TEXT", group = "changes", value_parser = parse_text)]
        description: Option<String>,
        /// Remove the description
        #[arg(long, group = "changes", conflicts_with = "description")]
        unset_description: bool,
        #[command(flatten)]
        file: DictionaryFile,
    },
    /// Remove a term, together with its behaviors
    Remove {
        /// The name of the term to remove
        name: String,
        #[command(flatten)]
        file: DictionaryFile,
    },
}

#[derive(Subcommand)]
enum BehaviorCommand {
    /// Add a behavior to a term, after its existing behaviors. Every form
    /// of the verb is required, since ubiqlint never guesses inflections
    Add {
        /// The name of the term, such as `Order`
        term: String,
        /// The verb in its base form, in lowercase words (e.g. `place`)
        #[arg(value_parser = parse_words)]
        verb: Words,
        /// The third-person singular present (e.g. `places`)
        #[arg(long = "s", value_name = "WORDS", value_parser = parse_words)]
        s: Words,
        /// The past tense (e.g. `placed`)
        #[arg(long, value_name = "WORDS", value_parser = parse_words)]
        past: Words,
        /// The past participle (e.g. `placed`)
        #[arg(long, value_name = "WORDS", value_parser = parse_words)]
        pp: Words,
        /// The present participle (e.g. `placing`)
        #[arg(long, value_name = "WORDS", value_parser = parse_words)]
        ing: Words,
        #[command(flatten)]
        file: DictionaryFile,
    },
    /// Change a behavior of a term. Give at least one change
    #[command(
        group(ArgGroup::new("changes").required(true).multiple(true)),
        override_usage = "ubiqlint behavior edit [OPTIONS] <TERM> <VERB>"
    )]
    Edit {
        /// The name of the term
        term: String,
        /// The verb of the behavior to edit
        #[arg(value_parser = parse_words)]
        verb: Words,
        /// Change the verb. The base form follows it; give `--s`, `--past`,
        /// `--pp` and `--ing` too
        #[arg(
            long = "verb",
            id = "new_verb",
            value_name = "WORDS",
            group = "changes",
            value_parser = parse_words
        )]
        new_verb: Option<Words>,
        /// The new third-person singular present
        #[arg(long = "s", value_name = "WORDS", group = "changes", value_parser = parse_words)]
        s: Option<Words>,
        /// The new past tense
        #[arg(long, value_name = "WORDS", group = "changes", value_parser = parse_words)]
        past: Option<Words>,
        /// The new past participle
        #[arg(long, value_name = "WORDS", group = "changes", value_parser = parse_words)]
        pp: Option<Words>,
        /// The new present participle
        #[arg(long, value_name = "WORDS", group = "changes", value_parser = parse_words)]
        ing: Option<Words>,
        #[command(flatten)]
        file: DictionaryFile,
    },
    /// Remove a behavior from a term
    Remove {
        /// The name of the term
        term: String,
        /// The verb of the behavior to remove
        #[arg(value_parser = parse_words)]
        verb: Words,
        #[command(flatten)]
        file: DictionaryFile,
    },
}

fn parse_words(text: &str) -> Result<Words, String> {
    Words::parse(text)
}

fn parse_term_name(name: &str) -> Result<String, String> {
    validate_term_name(name).map(|()| name.to_owned())
}

fn parse_text(text: &str) -> Result<String, String> {
    if text.trim().is_empty() {
        Err("must not be empty".to_owned())
    } else {
        Ok(text.to_owned())
    }
}

fn main() -> ExitCode {
    let mut reporter = Reporter::new();
    match Cli::parse().command {
        Command::Validate { files } => validate(&mut reporter, &files),
        Command::Check {
            config,
            dictionary,
            paths,
        } => check(&mut reporter, &config, dictionary.as_deref(), &paths),
        Command::Term(command) => {
            let (file, request) = term_request(command);
            edit_dictionary(&mut reporter, &file.dictionary, &request);
        }
        Command::Behavior(command) => {
            let (file, request) = behavior_request(command);
            edit_dictionary(&mut reporter, &file.dictionary, &request);
        }
    }
    reporter.finish()
}

/// An edit and how to describe it once done.
struct EditRequest {
    edit: Edit,
    /// What the edit did, such as "added term `Shipment`".
    done: String,
    /// What the edit was about, for when there was nothing to change.
    subject: String,
}

fn term_request(command: TermCommand) -> (DictionaryFile, EditRequest) {
    match command {
        TermCommand::Add {
            name,
            plural,
            kind,
            ja,
            description,
            file,
        } => (
            file,
            EditRequest {
                done: format!("added term `{name}`"),
                subject: format!("term `{name}`"),
                edit: Edit::AddTerm(NewTerm {
                    name,
                    plural,
                    kind,
                    ja,
                    description,
                }),
            },
        ),
        TermCommand::Edit {
            name,
            new_name,
            plural,
            kind,
            unset_kind,
            ja,
            unset_ja,
            description,
            unset_description,
            file,
        } => {
            let change = |value: Option<String>, unset: bool| match value {
                Some(value) => FieldChange::Set(value),
                None if unset => FieldChange::Unset,
                None => FieldChange::Keep,
            };
            let done = match &new_name {
                Some(new_name) if *new_name != name => {
                    format!("updated term `{name}` (renamed to `{new_name}`)")
                }
                _ => format!("updated term `{name}`"),
            };
            (
                file,
                EditRequest {
                    done,
                    subject: format!("term `{name}`"),
                    edit: Edit::EditTerm {
                        name,
                        changes: TermChanges {
                            name: new_name,
                            plural,
                            kind: change(kind, unset_kind),
                            ja: change(ja, unset_ja),
                            description: change(description, unset_description),
                        },
                    },
                },
            )
        }
        TermCommand::Remove { name, file } => (
            file,
            EditRequest {
                done: format!("removed term `{name}`"),
                subject: format!("term `{name}`"),
                edit: Edit::RemoveTerm { name },
            },
        ),
    }
}

fn behavior_request(command: BehaviorCommand) -> (DictionaryFile, EditRequest) {
    match command {
        BehaviorCommand::Add {
            term,
            verb,
            s,
            past,
            pp,
            ing,
            file,
        } => (
            file,
            EditRequest {
                done: format!("added behavior `{verb}` to term `{term}`"),
                subject: format!("behavior `{verb}` of term `{term}`"),
                edit: Edit::AddBehavior {
                    term,
                    forms: VerbForms {
                        base: verb,
                        s,
                        past,
                        pp,
                        ing,
                    },
                },
            },
        ),
        BehaviorCommand::Edit {
            term,
            verb,
            new_verb,
            s,
            past,
            pp,
            ing,
            file,
        } => {
            let done = match &new_verb {
                Some(new_verb) if *new_verb != verb => {
                    format!("updated behavior `{verb}` of term `{term}` (renamed to `{new_verb}`)")
                }
                _ => format!("updated behavior `{verb}` of term `{term}`"),
            };
            (
                file,
                EditRequest {
                    done,
                    subject: format!("behavior `{verb}` of term `{term}`"),
                    edit: Edit::EditBehavior {
                        term,
                        verb,
                        changes: BehaviorChanges {
                            verb: new_verb,
                            s,
                            past,
                            pp,
                            ing,
                        },
                    },
                },
            )
        }
        BehaviorCommand::Remove { term, verb, file } => (
            file,
            EditRequest {
                done: format!("removed behavior `{verb}` from term `{term}`"),
                subject: format!("behavior `{verb}` of term `{term}`"),
                edit: Edit::RemoveBehavior { term, verb },
            },
        ),
    }
}

/// Applies an edit to the dictionary at `path` and writes it back, or
/// reports why it was refused, leaving the file as it was.
fn edit_dictionary(reporter: &mut Reporter, path: &Path, request: &EditRequest) {
    match ubiqlint_project::edit_dictionary(path, &request.edit) {
        Ok(EditOutcome::Changed) => {
            reporter.line(&format!("{}: {}", path.display(), request.done));
        }
        Ok(EditOutcome::Unchanged) => {
            reporter.line(&format!(
                "{}: no changes: {} already has these values",
                path.display(),
                request.subject
            ));
        }
        Err(EditFileError::Read(error)) => reporter.read_error(path, &error),
        Err(EditFileError::Write(error)) => reporter.problem(&format!(
            "{}: error: cannot write the file: {error}",
            path.display()
        )),
        Err(EditFileError::Edit(EditError::InvalidDictionary(diagnostics))) => {
            reporter.diagnostics(path, &diagnostics);
            reporter.problem(&format!(
                "{}: error: cannot edit an invalid dictionary; fix the problems above first",
                path.display()
            ));
        }
        Err(EditFileError::Edit(EditError::Rejected(problems))) => {
            for problem in problems {
                reporter.problem(&format!(
                    "{}: error: {}",
                    path.display(),
                    single_line(&problem)
                ));
            }
        }
        Err(EditFileError::Edit(EditError::Layout(message))) => reporter.problem(&format!(
            "{}: error: {}",
            path.display(),
            single_line(&message)
        )),
    }
}

/// Prints problems to stdout, one per line. Rust ignores SIGPIPE, so a
/// closed pipe (`ubiqlint check ... | head -1`) is a write error here, not
/// a panic: the run just fails with exit code 1.
struct Reporter {
    out: io::BufWriter<io::StdoutLock<'static>>,
    failed: bool,
    broken: bool,
    reported: HashSet<PathBuf>,
}

impl Reporter {
    fn new() -> Self {
        Self {
            out: io::BufWriter::new(io::stdout().lock()),
            failed: false,
            broken: false,
            reported: HashSet::new(),
        }
    }

    /// Prints one line, returning whether the pipe still accepts writes.
    fn line(&mut self, text: &str) -> bool {
        if self.broken {
            return false;
        }
        match writeln!(self.out, "{text}") {
            Ok(()) => true,
            Err(_) => {
                self.broken = true;
                self.failed = true;
                false
            }
        }
    }

    /// Prints one problem and fails the run.
    fn problem(&mut self, text: &str) {
        self.failed = true;
        self.line(text);
    }

    /// Formats all parser and checker diagnostics consistently.
    fn diagnostics(&mut self, path: &Path, diagnostics: &[Diagnostic]) {
        for diagnostic in diagnostics {
            self.problem(&format!(
                "{}:{}: error: {}",
                path.display(),
                diagnostic.location,
                single_line(&diagnostic.message)
            ));
        }
    }

    /// Reports a file that could not be read, in the same format for
    /// definitions and source code.
    fn read_error(&mut self, path: &Path, error: &io::Error) {
        self.problem(&format!(
            "{}: error: cannot read the file: {error}",
            path.display()
        ));
    }

    /// Reports why a definition file could not be loaded.
    fn load_error(&mut self, path: &Path, error: &LoadError) {
        match error {
            LoadError::Read(error) => self.read_error(path, error),
            LoadError::Invalid(diagnostics) => self.diagnostics(path, diagnostics),
        }
    }

    /// Reports a dictionary failure once, even if many files use it.
    fn resolution_error(&mut self, error: &ResolutionError) {
        if !self.reported.insert(error.path().to_owned()) {
            return;
        }
        match error {
            ResolutionError::InvalidDictionary { path, diagnostics } => {
                self.diagnostics(path, diagnostics);
            }
            _ => self.problem(&format!(
                "{}: error: {}",
                error.path().display(),
                single_line(&error.to_string())
            )),
        }
    }

    /// A single missing dictionary can affect many files in a directory.
    fn missing_dictionary(&mut self, directory: &Path) {
        let directory = if directory.as_os_str().is_empty() {
            Path::new(".")
        } else {
            directory
        };
        if self.reported.insert(directory.to_owned()) {
            self.problem(&format!(
                "{}: error: no `{DICTIONARY_FILE_NAME}` found from this directory \
                 up to the configuration directory; add a dictionary or use `--dictionary`",
                directory.display()
            ));
        }
    }

    /// Prints a warning to stderr. Warnings never fail the run and do not
    /// mix into the `path:line:column: error:` lines on stdout.
    fn warn(&mut self, text: &str) {
        let _ = writeln!(io::stderr(), "{text}");
    }

    /// Tells whether the pipe is broken, so callers can stop early instead
    /// of doing a whole run's work into a closed pipe.
    fn broken(&self) -> bool {
        self.broken
    }

    fn finish(mut self) -> ExitCode {
        // A large output can still fail here, when the buffer is flushed.
        if self.out.flush().is_err() {
            self.failed = true;
        }
        if self.failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }
}

/// Checks that each file is a well-formed dictionary or configuration file.
fn validate(reporter: &mut Reporter, files: &[PathBuf]) {
    for path in files {
        let parse: fn(&str) -> Result<(), Vec<Diagnostic>> =
            match path.file_name().and_then(|name| name.to_str()) {
                Some(DICTIONARY_FILE_NAME) => |source| Dictionary::from_yaml_str(source).map(drop),
                Some(CONFIG_FILE_NAME) => |source| Config::from_yaml_str(source).map(drop),
                _ => {
                    reporter.problem(&format!(
                        "{}: error: cannot tell what kind of file this is: name a dictionary \
                         `{DICTIONARY_FILE_NAME}` and a configuration file `{CONFIG_FILE_NAME}`",
                        path.display()
                    ));
                    continue;
                }
            };
        load(reporter, path, parse);
    }
}

/// Checks the source files under the configuration file's directory, or
/// only those under `paths`, printing every problem as
/// `path:line:column: error: message`.
fn check(
    reporter: &mut Reporter,
    config_path: &Path,
    dictionary: Option<&Path>,
    paths: &[PathBuf],
) {
    // Every checked path is relative to the configuration directory, which
    // is also the upper boundary of dictionary resolution.
    let root = ubiqlint_project::config_root(config_path);
    // Validate the root dictionary, if present, alongside the config so
    // both sets of definition errors are still reported in one run. A root
    // dictionary is optional: a project may only have nested contexts.
    let project = Project::open(config_path)
        .map_err(|error| reporter.load_error(config_path, &error))
        .ok();
    let mut dictionaries = if let Some(path) = dictionary {
        let Some(loaded) = load(reporter, path, Dictionary::from_yaml_str) else {
            return;
        };
        Dictionaries::Fixed {
            path: path.to_owned(),
            dictionary: loaded,
        }
    } else {
        let mut resolver = DictionaryResolver::new(root);
        if let Err(error) = resolver.root_dictionary() {
            reporter.resolution_error(&error);
        }
        Dictionaries::Nearest(resolver)
    };
    let Some(project) = project else {
        return;
    };

    let files = select_files(reporter, &project, paths);
    // Finish dictionary loading before checking any file. This lets every
    // source using a dictionary share its lookup indices. Errors are
    // reported later, after the corresponding source has passed its read
    // and syntax checks.
    let mut run = CheckRun::new(
        &project,
        &mut dictionaries,
        files.iter().map(PathBuf::as_path),
    );
    let mut checked = 0;
    for relative in &files {
        if reporter.broken() {
            break;
        }
        if project.skip(relative).is_some() {
            continue;
        }
        checked += 1;
        let path = root.join(relative);
        let Some(source) = read_file(reporter, &path) else {
            continue;
        };
        match run.check(relative, &source) {
            // Files outside every rule still get syntax checks, but do not
            // need a domain dictionary merely for being in the tree.
            FileReport::Skipped(_) | FileReport::NoRules => {}
            FileReport::SyntaxErrors(diagnostics) | FileReport::Checked(diagnostics) => {
                reporter.diagnostics(&path, &diagnostics);
            }
            FileReport::MissingDictionary => reporter
                .missing_dictionary(path.parent().expect("a source file has a parent directory")),
            FileReport::InvalidDictionary(error) => reporter.resolution_error(&error),
        }
    }
    if checked == 0 {
        // Distinguish "no violations" from "nothing was checked", which CI
        // and hooks would otherwise not see. Warnings go to stderr and never
        // fail the run; only problems do.
        reporter.warn("warning: no source files were checked");
    }
}

/// Returns the files to check, relative to the project root and sorted:
/// every file under the root if no `paths` are given, or else the files
/// under the given directories and the given files themselves.
///
/// Directories are walked as the whole tree would be. A file given
/// directly is checked even if `.gitignore` would hide it from a walk, but
/// never if it matches `exclude` or is in no language ubiqlint knows; it is
/// then skipped with a warning.
fn select_files(reporter: &mut Reporter, project: &Project, paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut scopes = Vec::new();
    let mut files = Vec::new();
    for path in paths {
        let Some(relative) = project.relative(path) else {
            reporter.problem(&format!(
                "{}: error: not under the configuration directory `{}`",
                path.display(),
                project.read_root().display()
            ));
            continue;
        };
        if project.read_root().join(&relative).is_dir() {
            scopes.push(relative);
            continue;
        }
        let reason = match project.skip(&relative) {
            None => {
                files.push(relative);
                continue;
            }
            Some(Skip::Excluded) => "it matches `exclude` in the configuration",
            Some(Skip::UnsupportedLanguage) => "ubiqlint cannot check this kind of file",
        };
        reporter.warn(&format!(
            "{}: warning: not checked: {reason}",
            project.root().join(&relative).display()
        ));
    }
    if paths.is_empty() || !scopes.is_empty() {
        let walk = project.walk(&scopes);
        for error in &walk.errors {
            let path = match &error.path {
                Some(relative) => project.root().join(relative),
                None => project.read_root().to_path_buf(),
            };
            reporter.problem(&format!(
                "{}: error: cannot walk: {}",
                path.display(),
                error.message
            ));
        }
        if paths.is_empty() {
            // The walk is already sorted and has no duplicates.
            return walk.files;
        }
        files.extend(walk.files);
    }
    files.sort();
    files.dedup();
    files
}

/// Reads `path` and parses it, reporting every problem through `reporter`.
fn load<T>(
    reporter: &mut Reporter,
    path: &Path,
    parse: fn(&str) -> Result<T, Vec<Diagnostic>>,
) -> Option<T> {
    ubiqlint_project::load(path, parse)
        .map_err(|error| reporter.load_error(path, &error))
        .ok()
}

/// Reads input files using the same error format for definitions and source code.
fn read_file(reporter: &mut Reporter, path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .map_err(|error| reporter.read_error(path, &error))
        .ok()
}

/// Escapes control characters other than tabs, so that each problem stays on
/// one line even when it quotes a value containing a line break.
fn single_line(text: &str) -> String {
    let mut line = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && c != '\t' {
            line.extend(c.escape_default());
        } else {
            line.push(c);
        }
    }
    line
}
