//! Problems and results as lines of text, the same for every interface.
//!
//! A problem is one line, `path:line:column: error: message`, or
//! `path: error: message` when it has no position. A file that is not
//! checked is `path: warning: not checked: reason`. Control characters in
//! messages are escaped, so that a quoted value containing a line break
//! cannot split a problem over two lines.

use std::io;
use std::path::Path;

use ubiqlint_core::edit::{Edit, EditError};
use ubiqlint_core::{Diagnostic, ResolutionError};

use crate::{EditFileError, EditOutcome, LoadError, Project, Skip, WalkError};

/// Escapes control characters other than tabs, so that each problem stays
/// on one line even when it quotes a value containing a line break.
pub fn single_line(text: &str) -> String {
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

/// A problem with the file at `path` as a whole.
pub fn problem(path: &Path, message: &str) -> String {
    format!("{}: error: {}", path.display(), single_line(message))
}

/// A problem at a position in the file at `path`.
pub fn diagnostic(path: &Path, diagnostic: &Diagnostic) -> String {
    format!(
        "{}:{}: error: {}",
        path.display(),
        diagnostic.location,
        single_line(&diagnostic.message)
    )
}

/// The problems at positions in the file at `path`.
pub fn diagnostics(path: &Path, diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .map(|found| diagnostic(path, found))
        .collect()
}

/// A file that could not be read.
pub fn read_error(path: &Path, error: &io::Error) -> String {
    problem(path, &format!("cannot read the file: {error}"))
}

/// A definition file that could not be loaded.
pub fn load_error(path: &Path, error: &LoadError) -> Vec<String> {
    match error {
        LoadError::Read(error) => vec![read_error(path, error)],
        LoadError::Invalid(found) => diagnostics(path, found),
    }
}

/// A dictionary that could not be resolved, at its own path.
pub fn resolution_error(error: &ResolutionError) -> Vec<String> {
    match error {
        ResolutionError::InvalidDictionary {
            path,
            diagnostics: found,
        } => diagnostics(path, found),
        _ => vec![problem(error.path(), &error.to_string())],
    }
}

/// A path that could not be walked, as seen from the current directory.
pub fn walk_error(project: &Project, error: &WalkError) -> String {
    let path = match &error.path {
        Some(relative) => project.root().join(relative),
        None => project.read_root().to_path_buf(),
    };
    problem(&path, &format!("cannot walk: {}", error.message))
}

/// A given path that is not under the project root.
pub fn outside_root(project: &Project, path: &Path) -> String {
    problem(
        path,
        &format!(
            "not under the configuration directory `{}`",
            project.read_root().display()
        ),
    )
}

/// A given file that is not checked.
pub fn not_checked(path: &Path, skip: Skip) -> String {
    format!(
        "{}: warning: not checked: {}",
        path.display(),
        skip.reason()
    )
}

impl Skip {
    /// Why a file is not checked, as the end of a sentence.
    pub fn reason(self) -> &'static str {
        match self {
            Skip::Excluded => "it matches `exclude` in the configuration",
            Skip::UnsupportedLanguage => "ubiqlint cannot check this kind of file",
        }
    }
}

/// The result of [`crate::edit_dictionary`] for the dictionary at `path`:
/// a line saying what was done, or the problems that refused the edit.
pub fn edit_result(
    path: &Path,
    edit: &Edit,
    result: &Result<EditOutcome, EditFileError>,
) -> Result<String, Vec<String>> {
    let (done, subject) = describe(edit);
    match result {
        Ok(EditOutcome::Changed) => Ok(format!("{}: {done}", path.display())),
        Ok(EditOutcome::Unchanged) => Ok(format!(
            "{}: no changes: {subject} already has these values",
            path.display()
        )),
        Err(EditFileError::Read(error)) => Err(vec![read_error(path, error)]),
        Err(EditFileError::Write(error)) => Err(vec![problem(
            path,
            &format!("cannot write the file: {error}"),
        )]),
        Err(EditFileError::Edit(EditError::InvalidDictionary(found))) => {
            let mut lines = diagnostics(path, found);
            lines.push(problem(
                path,
                "cannot edit an invalid dictionary; fix the problems above first",
            ));
            Err(lines)
        }
        Err(EditFileError::Edit(EditError::Rejected(problems))) => Err(problems
            .iter()
            .map(|message| problem(path, message))
            .collect()),
        Err(EditFileError::Edit(EditError::Layout(message))) => Err(vec![problem(path, message)]),
    }
}

/// What `edit` does once done, such as "added term `Shipment`", and what
/// it is about, for when there was nothing to change.
fn describe(edit: &Edit) -> (String, String) {
    match edit {
        Edit::AddTerm(term) => (
            format!("added term `{}`", term.name),
            format!("term `{}`", term.name),
        ),
        Edit::EditTerm { name, changes } => {
            let done = match &changes.name {
                Some(new_name) if new_name != name => {
                    format!("updated term `{name}` (renamed to `{new_name}`)")
                }
                _ => format!("updated term `{name}`"),
            };
            (done, format!("term `{name}`"))
        }
        Edit::RemoveTerm { name } => (format!("removed term `{name}`"), format!("term `{name}`")),
        Edit::AddBehavior { term, forms } => (
            format!("added behavior `{}` to term `{term}`", forms.base),
            format!("behavior `{}` of term `{term}`", forms.base),
        ),
        Edit::EditBehavior {
            term,
            verb,
            changes,
        } => {
            let done = match &changes.verb {
                Some(new_verb) if new_verb != verb => {
                    format!("updated behavior `{verb}` of term `{term}` (renamed to `{new_verb}`)")
                }
                _ => format!("updated behavior `{verb}` of term `{term}`"),
            };
            (done, format!("behavior `{verb}` of term `{term}`"))
        }
        Edit::RemoveBehavior { term, verb } => (
            format!("removed behavior `{verb}` from term `{term}`"),
            format!("behavior `{verb}` of term `{term}`"),
        ),
    }
}
