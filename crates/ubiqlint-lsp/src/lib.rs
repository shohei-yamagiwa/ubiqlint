//! A synchronous, diagnostics-only Language Server Protocol server.
//!
//! Source text comes from open buffers. Configuration and dictionaries
//! come from disk and are reloaded for each check, sharing the same
//! project layer as the command line and MCP server.

mod notifications;
mod positions;
mod session;
mod uri;
mod watchers;

use std::error::Error;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use lsp_types::notification::{Notification as _, PublishDiagnostics};
use lsp_types::*;
use ubiqlint_core::CONFIG_FILE_NAME;

use session::Session;
use uri::file_path;

// The only server-to-client request is this one-time registration. A stable
// string id keeps it distinct from numeric ids if other requests are added.
const DEFINITION_WATCH_REQUEST: &str = "ubiqlint-watch-definitions";

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// Serves LSP on standard input and output. Only protocol messages are
/// written to stdout. A relative `config` is relative to the startup cwd.
pub fn serve_stdio(config: Option<&Path>) -> Result<()> {
    let (connection, threads) = Connection::stdio();
    let result = serve(&connection, config);
    // Drop the sender before joining the transport's writer thread.
    drop(connection);
    threads.join()?;
    result
}

fn serve(connection: &Connection, config: Option<&Path>) -> Result<()> {
    let (id, value) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(value)?;
    let encoding = if params
        .capabilities
        .general
        .as_ref()
        .and_then(|general| general.position_encodings.as_ref())
        .is_some_and(|encodings| encodings.contains(&PositionEncodingKind::UTF32))
    {
        PositionEncodingKind::UTF32
    } else {
        PositionEncodingKind::UTF16
    };
    let config = config_path(config, &params)?;
    let capabilities = ServerCapabilities {
        position_encoding: Some(encoding.clone()),
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                    include_text: Some(false),
                })),
                ..TextDocumentSyncOptions::default()
            },
        )),
        ..ServerCapabilities::default()
    };
    connection.initialize_finish(
        id,
        serde_json::to_value(InitializeResult {
            capabilities,
            server_info: Some(ServerInfo {
                name: "ubiqlint".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        })?,
    )?;
    if let Some(watched) = params
        .capabilities
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.did_change_watched_files.as_ref())
        .filter(|watched| watched.dynamic_registration == Some(true))
    {
        connection.sender.send(Message::Request(Request::new(
            DEFINITION_WATCH_REQUEST.to_owned().into(),
            "client/registerCapability".to_owned(),
            RegistrationParams {
                registrations: vec![Registration {
                    id: "ubiqlint-definitions".to_owned(),
                    method: "workspace/didChangeWatchedFiles".to_owned(),
                    register_options: Some(serde_json::to_value(watchers::options(
                        &config,
                        watched.relative_pattern_support == Some(true),
                    )?)?),
                }],
            },
        )))?;
    }
    let mut session = Session::new(config, encoding);
    let mut shutdown = false;
    for message in &connection.receiver {
        match message {
            Message::Request(request) if request.method == "shutdown" && !shutdown => {
                shutdown = true;
                connection
                    .sender
                    .send(Message::Response(Response::new_ok(request.id, ())))?;
            }
            Message::Request(request) => {
                let (code, message) = if shutdown {
                    (ErrorCode::InvalidRequest, "the server has shut down")
                } else {
                    (
                        ErrorCode::MethodNotFound,
                        "ubiqlint only provides diagnostics",
                    )
                };
                connection.sender.send(Message::Response(Response::new_err(
                    request.id,
                    code as i32,
                    message.to_owned(),
                )))?;
            }
            Message::Notification(notification) if notification.method == "exit" => {
                return if shutdown {
                    Ok(())
                } else {
                    Err("exit received before shutdown".into())
                };
            }
            Message::Notification(notification) if !shutdown => {
                if let Err(error) = notifications::handle(connection, &mut session, notification) {
                    // A malformed notification cannot be answered. Keep serving
                    // the other buffers instead of terminating the connection.
                    eprintln!("ubiqlint lsp: {error}");
                }
            }
            Message::Response(response) => {
                if let Some(error) = response.error {
                    eprintln!("ubiqlint lsp: client request failed: {}", error.message);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn publish(connection: &Connection, params: PublishDiagnosticsParams) -> Result<()> {
    connection
        .sender
        .send(Message::Notification(Notification::new(
            PublishDiagnostics::METHOD.to_owned(),
            params,
        )))?;
    Ok(())
}

#[allow(deprecated)] // root_uri/root_path remain in clients without workspaceFolders.
fn config_path(explicit: Option<&Path>, params: &InitializeParams) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(std::path::absolute(path)?);
    }
    let root = params
        .workspace_folders
        .as_ref()
        .and_then(|folders| folders.first())
        .map(|folder| &folder.uri)
        .or(params.root_uri.as_ref());
    let root = if let Some(uri) = root {
        file_path(uri).ok_or("the workspace root must be a local file URI")?
    } else if let Some(path) = &params.root_path {
        std::path::absolute(path)?
    } else {
        std::env::current_dir()?
    };
    Ok(root.join(CONFIG_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selects_the_config_in_explicit_workspace_or_legacy_root_order() {
        let cwd = std::env::current_dir().unwrap();
        let root = std::env::temp_dir().join("ubiqlint-root");
        let uri = url::Url::from_directory_path(&root).unwrap();
        let mut params: InitializeParams = serde_json::from_value(json!({
            "capabilities":{}, "rootUri":uri.as_str(),
            "workspaceFolders":[{"uri":uri.join("first/").unwrap().as_str(), "name":"first"}]
        }))
        .unwrap();
        assert_eq!(
            config_path(None, &params).unwrap(),
            root.join("first/ubiqlint.yaml")
        );
        assert_eq!(
            config_path(Some(Path::new("lint.yaml")), &params).unwrap(),
            cwd.join("lint.yaml")
        );
        params.workspace_folders = Some(Vec::new());
        assert_eq!(
            config_path(None, &params).unwrap(),
            root.join(CONFIG_FILE_NAME)
        );
        #[allow(deprecated)]
        {
            params.root_uri = None;
            params.root_path = Some(root.to_str().unwrap().to_owned());
            assert_eq!(
                config_path(None, &params).unwrap(),
                root.join(CONFIG_FILE_NAME)
            );
            params.root_path = None;
        }
        assert_eq!(
            config_path(None, &params).unwrap(),
            cwd.join(CONFIG_FILE_NAME)
        );
    }
}
