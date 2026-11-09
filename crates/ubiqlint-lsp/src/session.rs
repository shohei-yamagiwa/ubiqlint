//! Open buffers and fresh checks against the definitions on disk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lsp_types::{
    Diagnostic, DiagnosticSeverity, NumberOrString, PositionEncodingKind, PublishDiagnosticsParams,
    Uri,
};
use ubiqlint_core::DICTIONARY_FILE_NAME;
use ubiqlint_project::{CheckRun, FileReport, Project, report};

use crate::positions::Positions;
use crate::uri::file_path;

pub(crate) struct Document {
    pub(crate) uri: Uri,
    pub(crate) text: String,
    pub(crate) version: i32,
}

impl Document {
    fn report(&self, diagnostics: Vec<Diagnostic>) -> PublishDiagnosticsParams {
        PublishDiagnosticsParams::new(self.uri.clone(), diagnostics, Some(self.version))
    }
}

pub(crate) struct Session {
    pub(crate) config: PathBuf,
    root: PathBuf,
    pub(crate) encoding: PositionEncodingKind,
    // lsp_types::Uri contains an interior cache; use its spelling as the key.
    pub(crate) documents: BTreeMap<String, Document>,
}

impl Session {
    pub(crate) fn new(config: PathBuf, encoding: PositionEncodingKind) -> Self {
        Self {
            root: ubiqlint_project::config_root(&config).to_owned(),
            config,
            encoding,
            documents: BTreeMap::new(),
        }
    }

    pub(crate) fn is_definition(&self, uri: &Uri) -> bool {
        file_path(uri).is_some_and(|path| {
            let Some(relative) = ubiqlint_project::relative_to(&self.root, &path) else {
                return false;
            };
            // The selected config may have a custom name, such as lint.yaml;
            // only that file at the root is a config dependency of this server.
            self.config
                .file_name()
                .is_some_and(|name| relative == Path::new(name))
                || relative
                    .file_name()
                    .is_some_and(|name| name == DICTIONARY_FILE_NAME)
        })
    }

    /// Each pass reloads both configuration and dictionaries, so an edit
    /// made by MCP or another process is visible without restarting LSP.
    pub(crate) fn check(&self, only: Option<&Uri>) -> Vec<PublishDiagnosticsParams> {
        let documents: Vec<_> = self
            .documents
            .values()
            .filter(|document| only.is_none_or(|only| only == &document.uri))
            .map(|document| {
                let relative = file_path(&document.uri)
                    .and_then(|path| ubiqlint_project::relative_to(&self.root, &path))
                    // Keep unsupported documents quiet even if the config is
                    // broken. This guard also applies before Project::open.
                    .filter(|path| ubiqlint_project::adapter_for(path).is_some());
                (document, relative)
            })
            .collect();
        let project = match Project::open(&self.config) {
            Ok(project) => project,
            Err(error) => {
                let message = format!(
                    "Cannot check this file: {}",
                    report::load_error(&self.config, &error).join("\n")
                );
                return documents
                    .into_iter()
                    .map(|(document, relative)| {
                        document.report(if relative.is_some() {
                            vec![problem(message.clone())]
                        } else {
                            Vec::new()
                        })
                    })
                    .collect();
            }
        };
        let mut dictionaries = project.nearest_dictionaries();
        let mut run = CheckRun::new(
            &project,
            &mut dictionaries,
            documents
                .iter()
                .filter_map(|(_, relative)| relative.as_deref()),
        );
        documents
            .into_iter()
            .map(|(document, relative)| {
                let diagnostics = match relative {
                    None => Vec::new(),
                    Some(relative) => self.file_diagnostics(
                        run.check(&relative, &document.text),
                        &document.text,
                        &relative,
                    ),
                };
                document.report(diagnostics)
            })
            .collect()
    }

    fn file_diagnostics(&self, report: FileReport, text: &str, relative: &Path) -> Vec<Diagnostic> {
        match report {
            FileReport::Skipped(_) | FileReport::NoRules => Vec::new(),
            FileReport::SyntaxErrors(found) | FileReport::Checked(found) => {
                if found.is_empty() {
                    return Vec::new();
                }
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
                self.root.display(),
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
