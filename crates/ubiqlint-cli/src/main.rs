use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{ArgGroup, Args, Parser, Subcommand};
use ubiqlint_core::dictionary::validate_term_name;
use ubiqlint_core::{
    BehaviorChanges, CONFIG_FILE_NAME, Config, DICTIONARY_FILE_NAME, Diagnostic, Dictionary,
    DictionaryResolver, Edit, FieldChange, NewTerm, ResolutionError, TermChanges, VerbForms, Words,
};
use ubiqlint_project::{CheckRun, Dictionaries, FileReport, LoadError, Project, report};

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
    /// Serve the project of a configuration file to coding agents, as an
    /// MCP server over standard input and output
    Mcp {
        /// The configuration file of the project to serve. The server works
        /// in its directory, and paths are relative to it
        #[arg(long, default_value = CONFIG_FILE_NAME)]
        config: PathBuf,
    },
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
    let command = Cli::parse().command;
    if let Command::Mcp { config } = &command {
        // Served before the reporter exists: it locks standard output,
        // which carries the protocol. Problems go to stderr instead.
        return match ubiqlint_mcp::serve_stdio(config) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: the MCP server stopped: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let mut reporter = Reporter::new();
    match command {
        Command::Mcp { .. } => unreachable!("the MCP server was served above"),
        Command::Validate { files } => validate(&mut reporter, &files),
        Command::Check {
            config,
            dictionary,
            paths,
        } => check(&mut reporter, &config, dictionary.as_deref(), &paths),
        Command::Term(command) => {
            let (file, edit) = term_edit(command);
            edit_dictionary(&mut reporter, &file.dictionary, &edit);
        }
        Command::Behavior(command) => {
            let (file, edit) = behavior_edit(command);
            edit_dictionary(&mut reporter, &file.dictionary, &edit);
        }
    }
    reporter.finish()
}

fn term_edit(command: TermCommand) -> (DictionaryFile, Edit) {
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
            Edit::AddTerm(NewTerm {
                name,
                plural,
                kind,
                ja,
                description,
            }),
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
            (
                file,
                Edit::EditTerm {
                    name,
                    changes: TermChanges {
                        name: new_name,
                        plural,
                        kind: change(kind, unset_kind),
                        ja: change(ja, unset_ja),
                        description: change(description, unset_description),
                    },
                },
            )
        }
        TermCommand::Remove { name, file } => (file, Edit::RemoveTerm { name }),
    }
}

fn behavior_edit(command: BehaviorCommand) -> (DictionaryFile, Edit) {
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
            Edit::AddBehavior {
                term,
                forms: VerbForms {
                    base: verb,
                    s,
                    past,
                    pp,
                    ing,
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
        } => (
            file,
            Edit::EditBehavior {
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
        ),
        BehaviorCommand::Remove { term, verb, file } => (file, Edit::RemoveBehavior { term, verb }),
    }
}

/// Applies an edit to the dictionary at `path` and writes it back, or
/// reports why it was refused, leaving the file as it was.
fn edit_dictionary(reporter: &mut Reporter, path: &Path, edit: &Edit) {
    let result = ubiqlint_project::edit_dictionary(path, edit);
    match report::edit_result(path, edit, &result) {
        Ok(done) => {
            reporter.line(&done);
        }
        Err(problems) => reporter.problems(problems),
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

    /// Prints several problems and fails the run.
    fn problems(&mut self, lines: impl IntoIterator<Item = String>) {
        for line in lines {
            self.problem(&line);
        }
    }

    /// Formats all parser and checker diagnostics consistently.
    fn diagnostics(&mut self, path: &Path, diagnostics: &[Diagnostic]) {
        self.problems(report::diagnostics(path, diagnostics));
    }

    /// Reports a file that could not be read, in the same format for
    /// definitions and source code.
    fn read_error(&mut self, path: &Path, error: &io::Error) {
        self.problem(&report::read_error(path, error));
    }

    /// Reports why a definition file could not be loaded.
    fn load_error(&mut self, path: &Path, error: &LoadError) {
        self.problems(report::load_error(path, error));
    }

    /// Reports a dictionary failure once, even if many files use it.
    fn resolution_error(&mut self, error: &ResolutionError) {
        if self.reported.insert(error.path().to_owned()) {
            self.problems(report::resolution_error(error));
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
        let mut dictionaries = Dictionaries::Nearest(DictionaryResolver::new(root));
        if let Err(error) = dictionaries.check_root() {
            reporter.resolution_error(&error);
        }
        dictionaries
    };
    let Some(project) = project else {
        return;
    };

    let selection = project.select(paths);
    for path in &selection.outside {
        reporter.problem(&report::outside_root(&project, path));
    }
    for (relative, skip) in &selection.skipped {
        reporter.warn(&report::not_checked(&project.root().join(relative), *skip));
    }
    for error in &selection.walk_errors {
        reporter.problem(&report::walk_error(&project, error));
    }
    let files = selection.files;
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
