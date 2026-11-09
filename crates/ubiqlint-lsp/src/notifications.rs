//! Document synchronization and the checks each notification requests.

use lsp_server::{Connection, Notification};
use lsp_types::{
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, PublishDiagnosticsParams, Uri,
};

use crate::session::{Document, Session};
use crate::{Result, publish};

enum Check {
    None,
    Document(Uri),
    All,
}

pub(crate) fn handle(
    connection: &Connection,
    session: &mut Session,
    notification: Notification,
) -> Result<()> {
    let check = match notification.method.as_str() {
        "textDocument/didOpen" => did_open(session, params(notification)?),
        "textDocument/didChange" => did_change(session, params(notification)?)?,
        "textDocument/didClose" => did_close(connection, session, params(notification)?)?,
        "textDocument/didSave" => did_save(session, params(notification)?),
        "workspace/didChangeWatchedFiles" => did_watch(session, params(notification)?),
        _ => Check::None,
    };
    let reports = match check {
        Check::None => return Ok(()),
        Check::Document(uri) => session.check(Some(&uri)),
        Check::All => session.check(None),
    };
    for report in reports {
        publish(connection, report)?;
    }
    Ok(())
}

fn did_open(session: &mut Session, params: DidOpenTextDocumentParams) -> Check {
    let document = params.text_document;
    let uri = document.uri;
    session.documents.insert(
        uri.as_str().to_owned(),
        Document {
            uri: uri.clone(),
            text: document.text,
            version: document.version,
        },
    );
    Check::Document(uri)
}

fn did_change(session: &mut Session, params: DidChangeTextDocumentParams) -> Result<Check> {
    let Some(document) = session.documents.get_mut(params.text_document.uri.as_str()) else {
        return Ok(Check::None);
    };
    if params.text_document.version <= document.version {
        return Ok(Check::None);
    }
    // Full replacements can be applied in order by taking the last one.
    // A ranged change is only a fragment: never treat it as the whole file,
    // and reject the entire notification before changing text or version.
    if params
        .content_changes
        .iter()
        .any(|change| change.range.is_some())
    {
        return Err("expected full-document content changes".into());
    }
    if let Some(last) = params.content_changes.into_iter().next_back() {
        document.text = last.text;
    }
    // An empty list is a no-op, but its version still describes this buffer.
    document.version = params.text_document.version;
    Ok(Check::Document(params.text_document.uri))
}

fn did_close(
    connection: &Connection,
    session: &mut Session,
    params: DidCloseTextDocumentParams,
) -> Result<Check> {
    session.documents.remove(params.text_document.uri.as_str());
    publish(
        connection,
        PublishDiagnosticsParams::new(params.text_document.uri, Vec::new(), None),
    )?;
    Ok(Check::None)
}

fn did_save(session: &Session, params: DidSaveTextDocumentParams) -> Check {
    if session.is_definition(&params.text_document.uri) {
        Check::All
    } else if session
        .documents
        .contains_key(params.text_document.uri.as_str())
    {
        // A source save also observes definitions changed outside the client,
        // including clients that do not provide file-watching notifications.
        Check::Document(params.text_document.uri)
    } else {
        Check::None
    }
}

fn did_watch(session: &Session, params: DidChangeWatchedFilesParams) -> Check {
    if params
        .changes
        .iter()
        .any(|change| session.is_definition(&change.uri))
    {
        Check::All
    } else {
        Check::None
    }
}

fn params<T: serde::de::DeserializeOwned>(notification: Notification) -> Result<T> {
    Ok(serde_json::from_value(notification.params)?)
}
