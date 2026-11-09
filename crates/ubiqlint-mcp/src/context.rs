//! `get_context`: what a source file must follow.

use std::fmt::Write;
use std::fs;
use std::path::Path;

use rmcp::model::CallToolResult;
use ubiqlint_core::{DictionaryResolver, Element};
use ubiqlint_project::report;

use crate::params::ContextParams;
use crate::{failure, open, source_file, success};

pub(crate) fn get_context(config: &Path, params: &ContextParams) -> CallToolResult {
    let project = match open(config) {
        Ok(project) => project,
        Err(result) => return result,
    };
    let relative = match source_file(&project, &params.path) {
        Ok(relative) => relative,
        Err(result) => return result,
    };
    let shown = project.root().join(&relative);
    if let Some(skip) = project.skip(&relative) {
        return success(format!(
            "`{}` is not checked: {}.",
            shown.display(),
            skip.reason()
        ));
    }
    let settings = project.config();
    let rules: Vec<_> = settings
        .rules
        .iter()
        .filter(|rule| rule.includes(&relative))
        .collect();
    let import_rules: Vec<_> = settings
        .import_rules
        .iter()
        .filter(|rule| rule.includes(&relative))
        .collect();
    if rules.is_empty() && import_rules.is_empty() {
        return success(format!(
            "No rule in `{}` applies to `{}`: its names and imports are not checked.",
            config.display(),
            shown.display()
        ));
    }

    let mut text = String::new();
    let _ = writeln!(
        text,
        "`{}` must follow these rules of `{}`:",
        shown.display(),
        config.display()
    );
    for rule in &rules {
        let patterns: Vec<_> = rule
            .patterns
            .iter()
            .map(|pattern| format!("`{}`", pattern.source()))
            .collect();
        let _ = writeln!(
            text,
            "- naming rule `{}`, for {} names: {}",
            rule.id,
            rule.target.name(),
            patterns.join(", ")
        );
    }
    for rule in &import_rules {
        let internal: Vec<_> = rule
            .internal_namespaces()
            .iter()
            .map(|namespace| format!("`{}`", namespace.join(".")))
            .collect();
        let from = if internal.is_empty() {
            "any namespace".to_owned()
        } else {
            format!("outside {}", internal.join(", "))
        };
        let _ = writeln!(
            text,
            "- import rule `{}`: an import from {from} must not bring in the name of a term, \
             nor use `*`; use a fully qualified name instead",
            rule.id
        );
    }
    // The vocabularies the patterns refer to, in the order they are defined.
    let used: Vec<_> = settings
        .vocabularies
        .iter()
        .filter(|vocabulary| {
            rules.iter().any(|rule| {
                rule.patterns.iter().any(|pattern| {
                    pattern
                        .elements()
                        .iter()
                        .any(|element| matches!(element, Element::Vocabulary(name) if *name == vocabulary.name))
                })
            })
        })
        .collect();
    if !used.is_empty() {
        text.push('\n');
        for vocabulary in used {
            let words: Vec<_> = vocabulary
                .words
                .iter()
                .map(|words| format!("`{words}`"))
                .collect();
            let _ = writeln!(
                text,
                "`{{{}}}` is one of: {}.",
                vocabulary.name,
                words.join(", ")
            );
        }
    }

    let mut resolver = DictionaryResolver::new(project.root());
    match resolver.resolve(&relative) {
        Ok(Some(resolved)) => {
            let path = resolved.path.to_owned();
            match fs::read_to_string(&path) {
                Ok(source) => {
                    let _ = write!(
                        text,
                        "\nIts dictionary is `{}`:\n\n```yaml\n{}",
                        path.display(),
                        source
                    );
                    if !source.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str("```\n");
                }
                Err(error) => return failure(report::read_error(&path, &error)),
            }
        }
        Ok(None) => {
            let _ = write!(
                text,
                "\nNo dictionary applies to it: no `terms.yaml` was found from its directory up to \
                 the configuration directory, so it cannot be checked until one is added.\n"
            );
        }
        Err(error) => {
            let mut lines = report::resolution_error(&error);
            lines.push(format!(
                "The dictionary `{}` of `{}` must be fixed before it can be checked.",
                error.path().display(),
                shown.display()
            ));
            return failure(lines.join("\n"));
        }
    }
    success(text)
}
