//! Open buffers and fresh checks against the definitions on disk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lsp_types::{
    Diagnostic, DiagnosticSeverity, NumberOrString, PositionEncodingKind, PublishDiagnosticsParams,
    Uri,
};
use ubiqlint_core::{CONFIG_FILE_NAME, DICTIONARY_FILE_NAME};
use ubiqlint_project::{CheckRun, FileReport, Project, report};

use crate::file_path;
use crate::positions::Positions;

pub(crate) struct Document {
    pub(crate) uri: Uri,
    pub(crate) text: String,
    pub(crate) version: i32,
}

pub(crate) struct Session {
    pub(crate) config: PathBuf,
    pub(crate) encoding: PositionEncodingKind,
    // lsp_types::Uri contains an interior cache; use its spelling as the key.
    pub(crate) documents: BTreeMap<String, Document>,
}

impl Session {
    pub(crate) fn new(config: PathBuf, encoding: PositionEncodingKind) -> Self {
        Self {
            config,
            encoding,
            documents: BTreeMap::new(),
        }
    }

    pub(crate) fn is_definition(&self, uri: &Uri) -> bool {
        file_path(uri).is_some_and(|path| {
            let Some(relative) =
                ubiqlint_project::relative_to(ubiqlint_project::config_root(&self.config), &path)
            else {
                return false;
            };
            self.config
                .file_name()
                .is_some_and(|name| relative == Path::new(name))
                || relative
                    .file_name()
                    .is_some_and(|name| name == DICTIONARY_FILE_NAME || name == CONFIG_FILE_NAME)
        })
    }

    /// Each pass reloads both configuration and dictionaries, so an edit
    /// made by MCP or another process is visible without restarting LSP.
    pub(crate) fn check(&self, only: Option<&Uri>) -> Vec<PublishDiagnosticsParams> {
        let documents: Vec<_> = self
            .documents
            .iter()
            .filter(|(uri, _)| only.is_none_or(|only| only.as_str() == uri.as_str()))
            .collect();
        let project = Project::open(&self.config);
        let paths: BTreeMap<_, _> = documents
            .iter()
            .filter_map(|(uri, document)| {
                let path = file_path(&document.uri)?;
                let relative = ubiqlint_project::relative_to(
                    ubiqlint_project::config_root(&self.config),
                    &path,
                )?;
                Some(((*uri).clone(), relative))
            })
            .collect();
        let mut dictionaries = project.as_ref().ok().map(Project::nearest_dictionaries);
        let mut run =
            project
                .as_ref()
                .ok()
                .zip(dictionaries.as_mut())
                .map(|(project, dictionaries)| {
                    CheckRun::new(project, dictionaries, paths.values().map(PathBuf::as_path))
                });
        documents
            .into_iter()
            .map(|(uri, document)| {
                let diagnostics = match paths.get(uri) {
                    None => Vec::new(),
                    Some(relative) if ubiqlint_project::adapter_for(relative).is_none() => {
                        Vec::new()
                    }
                    Some(relative) => match (&project, &mut run) {
                        (Err(error), _) => vec![problem(format!(
                            "Cannot check this file: {}",
                            report::load_error(&self.config, error).join("\n")
                        ))],
                        (Ok(_), Some(run)) => self.file_diagnostics(
                            run.check(relative, &document.text),
                            &document.text,
                            relative,
                        ),
                        _ => unreachable!("a valid project has a check run"),
                    },
                };
                PublishDiagnosticsParams::new(
                    document.uri.clone(),
                    diagnostics,
                    Some(document.version),
                )
            })
            .collect()
    }

    fn file_diagnostics(&self, report: FileReport, text: &str, relative: &Path) -> Vec<Diagnostic> {
        match report {
            FileReport::Skipped(_) | FileReport::NoRules => Vec::new(),
            FileReport::SyntaxErrors(found) | FileReport::Checked(found) => {
                let positions = Positions::new(text, &self.encoding);
                found
                    .into_iter()
                    .map(|found| Diagnostic {
                        range: positions.range(&found),
                        severity: Some(DiagnosticSeverity::ERROR),
                        code: found.rule_id.map(NumberOrString::String),
                        source: Some("ubiqlint".to_owned()),
                        message: found.message,
                        ..Diagnostic::default()
                    })
                    .collect()
            }
            FileReport::MissingDictionary => vec![problem(format!(
                "Cannot check this file: no `{DICTIONARY_FILE_NAME}` found from `{}` up to `{}`; add a dictionary",
                relative.parent().unwrap_or_else(|| Path::new("")).display(),
                ubiqlint_project::config_root(&self.config).display(),
            ))],
            FileReport::InvalidDictionary(error) => vec![problem(format!(
                "Cannot check this file: {}",
                report::resolution_error(&error).join("\n")
            ))],
        }
    }
}

fn problem(message: String) -> Diagnostic {
    Diagnostic {
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("ubiqlint".to_owned()),
        message,
        ..Diagnostic::default()
    }
}
