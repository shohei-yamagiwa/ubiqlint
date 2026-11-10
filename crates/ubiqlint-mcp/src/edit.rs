//! The tools that edit dictionaries, as `ubiqlint term` and
//! `ubiqlint behavior` do.

use std::path::{Path, PathBuf};
use std::sync::PoisonError;

use rmcp::model::CallToolResult;
use ubiqlint_core::dictionary::validate_term_name;
use ubiqlint_core::{
    BehaviorChanges, DICTIONARY_FILE_NAME, Edit, FieldChange, NewTerm, TermChanges, VerbForms,
    Words,
};
use ubiqlint_project::report;

use crate::params::{
    AddBehaviorParams, AddTermParams, CreateDictionaryParams, EditBehaviorParams, EditTermParams,
    RemoveBehaviorParams, RemoveTermParams,
};
use crate::{Server, failure, outside, success};

pub(crate) fn create_dictionary(server: &Server, params: CreateDictionaryParams) -> CallToolResult {
    let path = match dictionary_path(server, &params.dictionary) {
        Ok(path) => path,
        Err(result) => return result,
    };
    let _edit = server.edits.lock().unwrap_or_else(PoisonError::into_inner);
    let result = ubiqlint_project::create_dictionary(&path, &params.context);
    match report::create_result(&path, &result) {
        Ok(done) => success(done),
        Err(problems) => failure(problems.join("\n")),
    }
}

pub(crate) fn add_term(server: &Server, params: AddTermParams) -> CallToolResult {
    let mut arguments = Arguments::default();
    let name = arguments.term_name("name", params.name);
    let plural = arguments.words("plural", &params.plural);
    let kind = arguments.optional_text("kind", params.kind);
    let ja = arguments.optional_text("ja", params.ja);
    let description = arguments.optional_text("description", params.description);
    if let Some(result) = arguments.problems() {
        return result;
    }
    let edit = Edit::AddTerm(NewTerm {
        name: name.unwrap(),
        plural: plural.unwrap(),
        kind,
        ja,
        description,
    });
    apply(server, &params.dictionary, &edit)
}

pub(crate) fn edit_term(server: &Server, params: EditTermParams) -> CallToolResult {
    let mut arguments = Arguments::default();
    let new_name = params
        .new_name
        .and_then(|name| arguments.term_name("new_name", name));
    let plural = params
        .plural
        .as_deref()
        .and_then(|plural| arguments.words("plural", plural));
    let kind = arguments.field("kind", params.kind, params.unset_kind);
    let ja = arguments.field("ja", params.ja, params.unset_ja);
    let description = arguments.field("description", params.description, params.unset_description);
    let changes = TermChanges {
        name: new_name,
        plural,
        kind,
        ja,
        description,
    };
    if arguments.is_empty() && changes == TermChanges::default() {
        arguments.problem(
            "give at least one change: `new_name`, `plural`, `kind`, `unset_kind`, `ja`, \
             `unset_ja`, `description` or `unset_description`",
        );
    }
    if let Some(result) = arguments.problems() {
        return result;
    }
    let edit = Edit::EditTerm {
        name: params.name,
        changes,
    };
    apply(server, &params.dictionary, &edit)
}

pub(crate) fn remove_term(server: &Server, params: RemoveTermParams) -> CallToolResult {
    let edit = Edit::RemoveTerm { name: params.name };
    apply(server, &params.dictionary, &edit)
}

pub(crate) fn add_behavior(server: &Server, params: AddBehaviorParams) -> CallToolResult {
    let mut arguments = Arguments::default();
    let base = arguments.words("verb", &params.verb);
    let s = arguments.words("s", &params.s);
    let past = arguments.words("past", &params.past);
    let pp = arguments.words("pp", &params.pp);
    let ing = arguments.words("ing", &params.ing);
    if let Some(result) = arguments.problems() {
        return result;
    }
    let edit = Edit::AddBehavior {
        term: params.term,
        forms: VerbForms {
            base: base.unwrap(),
            s: s.unwrap(),
            past: past.unwrap(),
            pp: pp.unwrap(),
            ing: ing.unwrap(),
        },
    };
    apply(server, &params.dictionary, &edit)
}

pub(crate) fn edit_behavior(server: &Server, params: EditBehaviorParams) -> CallToolResult {
    let mut arguments = Arguments::default();
    let verb = arguments.words("verb", &params.verb);
    let mut optional =
        |name: &str, value: Option<String>| value.and_then(|value| arguments.words(name, &value));
    let changes = BehaviorChanges {
        verb: optional("new_verb", params.new_verb),
        s: optional("s", params.s),
        past: optional("past", params.past),
        pp: optional("pp", params.pp),
        ing: optional("ing", params.ing),
    };
    if arguments.is_empty() && changes == BehaviorChanges::default() {
        arguments.problem("give at least one change: `new_verb`, `s`, `past`, `pp` or `ing`");
    }
    if let Some(result) = arguments.problems() {
        return result;
    }
    let edit = Edit::EditBehavior {
        term: params.term,
        verb: verb.unwrap(),
        changes,
    };
    apply(server, &params.dictionary, &edit)
}

pub(crate) fn remove_behavior(server: &Server, params: RemoveBehaviorParams) -> CallToolResult {
    let mut arguments = Arguments::default();
    let verb = arguments.words("verb", &params.verb);
    if let Some(result) = arguments.problems() {
        return result;
    }
    let edit = Edit::RemoveBehavior {
        term: params.term,
        verb: verb.unwrap(),
    };
    apply(server, &params.dictionary, &edit)
}

/// Applies `edit` to the dictionary named by `dictionary`, one edit at a
/// time, and tells what was done or why nothing was.
fn apply(server: &Server, dictionary: &str, edit: &Edit) -> CallToolResult {
    let path = match dictionary_path(server, dictionary) {
        Ok(path) => path,
        Err(result) => return result,
    };
    // The lock only guards the file, so a poisoned one is still usable.
    let _edit = server.edits.lock().unwrap_or_else(PoisonError::into_inner);
    let result = ubiqlint_project::edit_dictionary(&path, edit);
    match report::edit_result(&path, edit, &result) {
        Ok(done) => success(done),
        Err(problems) => failure(problems.join("\n")),
    }
}

/// The path of the dictionary named by `dictionary`, which must be a
/// `terms.yaml` under the configuration directory. Editing a dictionary
/// does not need a valid configuration file, only its directory.
fn dictionary_path(server: &Server, dictionary: &str) -> Result<PathBuf, CallToolResult> {
    let root = ubiqlint_project::config_root(&server.config);
    let Some(relative) = ubiqlint_project::relative_to(root, Path::new(dictionary)) else {
        return Err(outside(dictionary));
    };
    if relative.file_name().and_then(|name| name.to_str()) != Some(DICTIONARY_FILE_NAME) {
        return Err(failure(report::problem(
            Path::new(dictionary),
            &format!("not a dictionary: dictionaries are files named `{DICTIONARY_FILE_NAME}`"),
        )));
    }
    Ok(root.join(relative))
}

/// Checks the arguments of an edit, collecting every problem so that all
/// of them are reported at once.
#[derive(Default)]
struct Arguments {
    problems: Vec<String>,
}

impl Arguments {
    fn problem(&mut self, problem: &str) {
        self.problems.push(problem.to_owned());
    }

    fn is_empty(&self) -> bool {
        self.problems.is_empty()
    }

    fn invalid(&mut self, name: &str, problem: &str) {
        self.problems.push(format!("`{name}`: {problem}"));
    }

    fn term_name(&mut self, name: &str, value: String) -> Option<String> {
        match validate_term_name(&value) {
            Ok(()) => Some(value),
            Err(problem) => {
                self.invalid(name, &problem);
                None
            }
        }
    }

    fn words(&mut self, name: &str, value: &str) -> Option<Words> {
        match Words::parse(value) {
            Ok(words) => Some(words),
            Err(problem) => {
                self.invalid(name, &problem);
                None
            }
        }
    }

    fn optional_text(&mut self, name: &str, value: Option<String>) -> Option<String> {
        let value = value?;
        if value.trim().is_empty() {
            self.invalid(name, "must not be empty");
            None
        } else {
            Some(value)
        }
    }

    /// A change to an optional field: set to `value`, or removed if
    /// `unset`, but not both.
    fn field(&mut self, name: &str, value: Option<String>, unset: bool) -> FieldChange {
        if value.is_some() && unset {
            self.problem(&format!("give either `{name}` or `unset_{name}`, not both"));
            return FieldChange::Keep;
        }
        match self.optional_text(name, value) {
            Some(value) => FieldChange::Set(value),
            None if unset => FieldChange::Unset,
            None => FieldChange::Keep,
        }
    }

    /// The problems found, as a result to return, if any.
    fn problems(&mut self) -> Option<CallToolResult> {
        if self.problems.is_empty() {
            return None;
        }
        let lines: Vec<_> = self
            .problems
            .iter()
            .map(|problem| format!("- {}", report::single_line(problem)))
            .collect();
        Some(failure(format!(
            "invalid arguments; the dictionary was not changed:\n{}",
            lines.join("\n")
        )))
    }
}
