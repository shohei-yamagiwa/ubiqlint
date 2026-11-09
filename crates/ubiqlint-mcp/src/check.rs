//! `check_code` and `check_files`: checking source code.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rmcp::model::CallToolResult;
use ubiqlint_core::DICTIONARY_FILE_NAME;
use ubiqlint_project::{CheckRun, FileReport, report};

use crate::params::{CheckCodeParams, CheckFilesParams};
use crate::{count, failure, open, outside_line, source_file, success};

/// The most problems `check_files` lists unless told otherwise, so that a
/// project full of violations does not fill an agent's context.
const DEFAULT_LIMIT: usize = 100;

pub(crate) fn check_code(config: &Path, params: &CheckCodeParams) -> CallToolResult {
    let project = match open(config) {
        Ok(project) => project,
        Err(result) => return result,
    };
    let relative = match source_file(&project, &params.path) {
        Ok(relative) => relative,
        Err(result) => return result,
    };
    let path = project.root().join(&relative);
    let mut dictionaries = project.nearest_dictionaries();
    let mut run = CheckRun::new(&project, &mut dictionaries, [relative.as_path()]);
    match run.check(&relative, &params.content) {
        FileReport::Skipped(skip) => success(format!(
            "`{}` was not checked: {}.",
            path.display(),
            skip.reason()
        )),
        FileReport::NoRules => success(format!(
            "`{}` has no syntax errors, but no rule applies to it, so its names and imports \
             were not checked.",
            path.display()
        )),
        FileReport::SyntaxErrors(diagnostics) => {
            let mut lines = report::diagnostics(&path, &diagnostics);
            lines.push(format!(
                "{} in `{}`, so its names and imports were not checked.",
                count(diagnostics.len(), "syntax error", "syntax errors"),
                path.display()
            ));
            success(lines.join("\n"))
        }
        FileReport::Checked(diagnostics) => {
            let mut lines = report::diagnostics(&path, &diagnostics);
            lines.push(if diagnostics.is_empty() {
                format!("No problems in `{}`.", path.display())
            } else {
                format!(
                    "{} in `{}`.",
                    count(diagnostics.len(), "problem", "problems"),
                    path.display()
                )
            });
            success(lines.join("\n"))
        }
        FileReport::MissingDictionary => failure(missing_dictionary(&directory_of(&path))),
        FileReport::InvalidDictionary(error) => {
            let mut lines = report::resolution_error(&error);
            lines.push(format!(
                "The dictionary `{}` must be fixed before `{}` can be checked.",
                error.path().display(),
                path.display()
            ));
            failure(lines.join("\n"))
        }
    }
}

pub(crate) fn check_files(config: &Path, params: &CheckFilesParams) -> CallToolResult {
    let project = match open(config) {
        Ok(project) => project,
        Err(result) => return result,
    };
    let paths: Vec<PathBuf> = params.paths.iter().map(PathBuf::from).collect();
    let selection = project.select(&paths);

    let mut dictionaries = project.nearest_dictionaries();
    // A missing or broken dictionary is reported once, however many files
    // use it.
    let mut reported = HashSet::new();
    let mut problems = Vec::new();
    // As in CI, a broken root dictionary is a problem even if no file
    // checked here uses it.
    if let Err(error) = dictionaries.check_root() {
        reported.insert(error.path().to_owned());
        problems.extend(report::resolution_error(&error));
    }
    // A given path outside the project is the caller's mistake: report it
    // with the rest, and mark the result as an error.
    problems.extend(
        selection
            .outside
            .iter()
            .map(|path| outside_line(&path.display().to_string())),
    );
    let notes: Vec<_> = selection
        .skipped
        .iter()
        .map(|(relative, skip)| report::not_checked(&project.root().join(relative), *skip))
        .collect();
    problems.extend(
        selection
            .walk_errors
            .iter()
            .map(|error| report::walk_error(&project, error)),
    );

    let mut run = CheckRun::new(
        &project,
        &mut dictionaries,
        selection.files.iter().map(PathBuf::as_path),
    );
    let mut attempted = 0;
    // Files that parse but that no rule applies to: unlike files without
    // problems, their names and imports were not checked at all.
    let mut no_rules = Vec::new();
    for relative in &selection.files {
        if project.skip(relative).is_some() {
            continue;
        }
        attempted += 1;
        let path = project.root().join(relative);
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                problems.push(report::read_error(&path, &error));
                continue;
            }
        };
        match run.check(relative, &source) {
            FileReport::Skipped(_) => {}
            FileReport::NoRules => no_rules.push(path),
            FileReport::SyntaxErrors(diagnostics) | FileReport::Checked(diagnostics) => {
                problems.extend(report::diagnostics(&path, &diagnostics));
            }
            FileReport::MissingDictionary => {
                let directory = directory_of(&path);
                if reported.insert(directory.clone()) {
                    problems.push(missing_dictionary(&directory));
                }
            }
            FileReport::InvalidDictionary(error) => {
                if reported.insert(error.path().to_owned()) {
                    problems.extend(report::resolution_error(&error));
                }
            }
        }
    }

    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).max(1);
    let total = problems.len();
    let mut lines: Vec<_> = problems.into_iter().take(limit).collect();
    if total > limit {
        let more = total - limit;
        lines.push(format!(
            "... and {more} more {}. Check fewer paths, or raise `limit`, to see them.",
            if more == 1 { "problem" } else { "problems" }
        ));
    }
    lines.extend(notes);
    let reason = format!(
        "names and imports not checked: no rule in `{}` applies to it",
        config.display()
    );
    lines.extend(
        no_rules
            .iter()
            .take(limit)
            .map(|path| format!("{}: warning: {reason}", path.display())),
    );
    if no_rules.len() > limit {
        let more = no_rules.len() - limit;
        lines.push(format!(
            "... and {} that no rule applies to.",
            count(more, "more file", "more files")
        ));
    }
    let checked = attempted - no_rules.len();
    lines.push(if attempted == 0 {
        "No source files were checked.".to_owned()
    } else if checked == 0 {
        "No source files were checked against the rules.".to_owned()
    } else if total == 0 {
        format!("Checked {}: no problems.", count(checked, "file", "files"))
    } else {
        format!(
            "Checked {}: {}.",
            count(checked, "file", "files"),
            count(total, "problem", "problems")
        )
    });
    let text = lines.join("\n");
    if selection.outside.is_empty() {
        success(text)
    } else {
        failure(text)
    }
}

/// The directory of a source file, as shown in problems: `.` rather than
/// an empty path.
fn directory_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_owned(),
        _ => PathBuf::from("."),
    }
}

/// No dictionary was found for the sources in `directory`.
fn missing_dictionary(directory: &Path) -> String {
    report::problem(
        directory,
        &format!(
            "no `{DICTIONARY_FILE_NAME}` found from this directory up to the configuration \
             directory; add a dictionary"
        ),
    )
}
