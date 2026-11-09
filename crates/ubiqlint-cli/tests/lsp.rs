//! Drive the executable with LSP's Content-Length framing, as an editor does.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{Value, json};

const CONFIG: &str = "\
exclude: [generated/**]
rules:
  - id: types
    include: [src/**]
    target: type
    patterns: ['{noun}']
  - id: methods
    include: [src/**]
    target: method
    patterns: ['{verb}', '{verb}{noun}']
import_rules:
  - id: imports
    include: [src/**]
    internal_namespaces: [com.example]
";
const TERMS: &str = include_str!("fixtures/check/ok/terms.yaml");

struct Server {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Value>,
    id: i32,
}

impl Server {
    fn start(config: Option<&Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ubiqlint"));
        command.arg("lsp");
        if let Some(config) = config {
            command.arg("--config").arg(config);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            loop {
                let mut length = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Content-Length:") {
                        length = Some(value.trim().parse::<usize>().unwrap());
                    }
                }
                let mut bytes = vec![0; length.expect("only LSP frames may be written to stdout")];
                if reader.read_exact(&mut bytes).is_err() {
                    return;
                }
                if sender
                    .send(serde_json::from_slice(&bytes).unwrap())
                    .is_err()
                {
                    return;
                }
            }
        });
        Self {
            child,
            input,
            output,
            id: 0,
        }
    }

    fn send(&mut self, message: Value) {
        let bytes = serde_json::to_vec(&message).unwrap();
        let input = self.input.as_mut().unwrap();
        write!(input, "Content-Length: {}\r\n\r\n", bytes.len()).unwrap();
        input.write_all(&bytes).unwrap();
        input.flush().unwrap();
    }

    fn receive(&self) -> Value {
        self.output
            .recv_timeout(Duration::from_secs(10))
            .expect("expected an LSP message")
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        self.send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}));
        let response = self.receive();
        assert_eq!(response["id"], id, "unexpected message: {response}");
        response
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc":"2.0", "method":method, "params":params}));
    }

    fn initialize(&mut self, project: &TempProject, encodings: Value) -> Value {
        let response = self.request(
            "initialize",
            json!({
                "processId":null,
                "workspaceFolders":[{"uri":project.uri(""), "name":"test"}],
                "capabilities":{"general":{"positionEncodings":encodings}}
            }),
        );
        self.notify("initialized", json!({}));
        response
    }

    fn open(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument":{
                "uri":uri, "languageId":"java", "version":1, "text":text
            }}),
        );
    }

    fn change(&mut self, uri: &str, version: i32, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument":{"uri":uri,"version":version},
                "contentChanges":[{"text":text}]
            }),
        );
    }

    fn diagnostics(&self, uri: &str) -> Value {
        let message = self.receive();
        assert_eq!(message["method"], "textDocument/publishDiagnostics");
        assert_eq!(message["params"]["uri"], uri);
        message["params"].clone()
    }

    fn shutdown(&mut self) {
        assert_eq!(self.request("shutdown", Value::Null)["result"], Value::Null);
        self.notify("exit", Value::Null);
        drop(self.input.take());
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        let project = Self(std::env::temp_dir().join(format!(
            "ubiqlint lsp 注文-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        project.write("ubiqlint.yaml", CONFIG);
        project.write("terms.yaml", TERMS);
        project
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn uri(&self, path: &str) -> String {
        url::Url::from_file_path(self.0.join(path)).unwrap().into()
    }

    fn server(&self) -> Server {
        let mut server = Server::start(None);
        server.initialize(self, json!(["utf-16"]));
        server
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn checks_unsaved_buffers_clears_fixed_and_closed_files_and_shuts_down() {
    let project = TempProject::new();
    project.write("src/Order.java", "class Order {}");
    let mut server = project.server();
    let uri = project.uri("src/Order.java");
    server.open(&uri, "class Order { void updateOrder() {} }");
    let found = server.diagnostics(&uri);
    assert_eq!(found["version"], 1);
    assert_eq!(found["diagnostics"].as_array().unwrap().len(), 1);
    let diagnostic = &found["diagnostics"][0];
    assert_eq!(diagnostic["code"], "methods");
    assert_eq!(diagnostic["source"], "ubiqlint");
    assert_eq!(diagnostic["severity"], 1);
    assert_eq!(
        diagnostic["range"],
        json!({"start":{"line":0,"character":19},"end":{"line":0,"character":30}})
    );
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains("defined behaviors")
    );
    server.change(&uri, 2, "class Order { void placeOrder() {} }");
    let fixed = server.diagnostics(&uri);
    assert_eq!(fixed["version"], 2);
    assert_eq!(fixed["diagnostics"], json!([]));
    server.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}));
    assert_eq!(server.diagnostics(&uri)["diagnostics"], json!([]));
    server.shutdown();
}

#[test]
fn negotiates_utf32_or_falls_back_to_utf16_and_preserves_escape_ranges() {
    let project = TempProject::new();
    let uri = project.uri("src/Order.java");
    for (encodings, encoding, start) in [
        (json!(["utf-8", "utf-32", "utf-16"]), "utf-32", 13),
        (json!(["utf-8"]), "utf-16", 14),
    ] {
        let mut server = Server::start(None);
        let initialized = server.initialize(&project, encodings);
        assert_eq!(
            initialized["result"]["capabilities"]["positionEncoding"],
            encoding
        );
        assert_eq!(
            initialized["result"]["capabilities"]["textDocumentSync"]["change"],
            1
        );
        // Java's Unicode escapes are checked after translation, but the
        // range must cover the spelling in the original buffer.
        server.open(&uri, "// header\r\n/*😀注文*/class \\u0050urchase {}\r");
        let found = server.diagnostics(&uri);
        assert_eq!(found["diagnostics"][0]["code"], "types");
        assert_eq!(
            found["diagnostics"][0]["range"],
            json!({
                "start":{"line":1,"character":start}, "end":{"line":1,"character":start+13}
            })
        );
        server.shutdown();
    }
}

#[test]
fn checks_import_boundaries_and_reports_syntax_without_naming_guesses() {
    let project = TempProject::new();
    let mut server = project.server();
    let uri = project.uri("src/Order.java");
    server.open(&uri, "import com.vendor.Order;\nclass Order {}");
    let found = server.diagnostics(&uri);
    assert_eq!(found["diagnostics"][0]["code"], "imports");
    assert_eq!(
        found["diagnostics"][0]["range"],
        json!({
            "start":{"line":0,"character":0}, "end":{"line":0,"character":24}
        })
    );
    server.change(&uri, 2, "import com.vendor.Order; class Purchase {");
    let found = server.diagnostics(&uri);
    for diagnostic in found["diagnostics"].as_array().unwrap() {
        assert!(
            diagnostic["message"]
                .as_str()
                .unwrap()
                .contains("syntax error")
        );
        assert!(diagnostic.get("code").is_none());
    }
    server.shutdown();
}

#[test]
fn ignores_excluded_ruleless_unsupported_and_outside_files() {
    let project = TempProject::new();
    let mut server = project.server();
    for uri in [
        project.uri("generated/Order.java"),
        project.uri("other/Order.java"),
        project.uri("src/readme.txt"),
        project.uri("../Outside.java"),
    ] {
        server.open(&uri, "class Purchase {}");
        assert_eq!(server.diagnostics(&uri)["diagnostics"], json!([]));
    }
    server.shutdown();
}

#[test]
fn rechecks_all_open_buffers_on_dictionary_and_configuration_saves() {
    let project = TempProject::new();
    let mut server = project.server();
    let first = project.uri("src/A.java");
    let second = project.uri("src/B.java");
    for uri in [&first, &second] {
        server.open(uri, "class Purchase {}");
        assert_eq!(
            server.diagnostics(uri)["diagnostics"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    // The disk changes; the open source buffers have never been written.
    project.write("terms.yaml", "context: purchasing\nterms:\n  - name: Purchase\n    forms: { singular: purchase, plural: purchases }\n");
    server.notify(
        "textDocument/didSave",
        json!({"textDocument":{"uri":project.uri("terms.yaml")}}),
    );
    for uri in [&first, &second] {
        assert_eq!(server.diagnostics(uri)["diagnostics"], json!([]));
    }
    project.write("ubiqlint.yaml", &CONFIG.replace("{noun}", "Allowed"));
    server.notify(
        "textDocument/didSave",
        json!({"textDocument":{"uri":project.uri("ubiqlint.yaml")}}),
    );
    for uri in [&first, &second] {
        assert_eq!(server.diagnostics(uri)["diagnostics"][0]["code"], "types");
    }
    project.write("ubiqlint.yaml", "rules: []\n");
    server.notify(
        "textDocument/didSave",
        json!({"textDocument":{"uri":project.uri("ubiqlint.yaml")}}),
    );
    for uri in [&first, &second] {
        assert_eq!(server.diagnostics(uri)["diagnostics"], json!([]));
    }
    server.shutdown();
}

#[test]
fn recovers_from_missing_or_broken_definitions_without_restarting() {
    let project = TempProject::new();
    fs::remove_file(project.0.join("ubiqlint.yaml")).unwrap();
    let mut server = project.server();
    let uri = project.uri("src/Order.java");
    server.open(&uri, "class Order {}");
    assert!(
        server.diagnostics(&uri)["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("cannot read")
    );
    project.write("ubiqlint.yaml", CONFIG);
    fs::remove_file(project.0.join("terms.yaml")).unwrap();
    server.change(&uri, 2, "class Order {}");
    assert!(
        server.diagnostics(&uri)["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("no `terms.yaml`")
    );
    project.write("terms.yaml", "context: broken\nterms: nope\n");
    server.change(&uri, 3, "class Order {}");
    assert!(
        server.diagnostics(&uri)["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("terms.yaml:2:")
    );
    project.write("terms.yaml", TERMS);
    server.change(&uri, 4, "class Order {}");
    assert_eq!(server.diagnostics(&uri)["diagnostics"], json!([]));
    project.write("ubiqlint.yaml", "rules: wrong\n");
    server.change(&uri, 5, "class Order {}");
    assert!(
        server.diagnostics(&uri)["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("ubiqlint.yaml:1:")
    );
    server.shutdown();
}

#[test]
fn uses_nearest_dictionary_and_an_explicit_configuration_over_the_workspace() {
    let project = TempProject::new();
    project.write("src/billing/terms.yaml", "context: billing\nterms:\n  - name: Invoice\n    forms: { singular: invoice, plural: invoices }\n");
    let other = TempProject::new();
    let mut server = Server::start(Some(&project.0.join("ubiqlint.yaml")));
    server.initialize(&other, json!(["utf-16"]));
    let uri = project.uri("src/billing/Invoice.java");
    server.open(&uri, "class Invoice {}");
    assert_eq!(server.diagnostics(&uri)["diagnostics"], json!([]));
    server.change(&uri, 2, "class Order {}");
    assert_eq!(server.diagnostics(&uri)["diagnostics"][0]["code"], "types");
    server.shutdown();
}

#[test]
fn registers_definition_watchers_and_rechecks_external_edits_and_deletions() {
    let project = TempProject::new();
    let mut server = Server::start(None);
    server.request(
        "initialize",
        json!({
            "rootUri":project.uri(""), "capabilities":{
                "workspace":{"didChangeWatchedFiles":{"dynamicRegistration":true}}
            }
        }),
    );
    server.notify("initialized", json!({}));
    let registration = server.receive();
    assert_eq!(registration["method"], "client/registerCapability");
    assert_eq!(
        registration["params"]["registrations"][0]["method"],
        "workspace/didChangeWatchedFiles"
    );
    server.send(json!({"jsonrpc":"2.0", "id":registration["id"], "result":null}));
    let uri = project.uri("src/Order.java");
    server.open(&uri, "class Order {}");
    assert_eq!(server.diagnostics(&uri)["diagnostics"], json!([]));
    fs::remove_file(project.0.join("terms.yaml")).unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{
            "uri":project.uri("terms.yaml"), "type":3
        }]}),
    );
    assert!(
        server.diagnostics(&uri)["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("no `terms.yaml`")
    );
    server.shutdown();
}

#[test]
fn rechecks_custom_configuration_saves_and_does_not_reopen_closed_buffers() {
    let project = TempProject::new();
    project.write("lint.yaml", CONFIG);
    let mut server = Server::start(Some(&project.0.join("lint.yaml")));
    server.initialize(&project, json!(["utf-16"]));
    let uri = project.uri("src/Order.java");
    server.open(&uri, "class Purchase {}");
    assert_eq!(server.diagnostics(&uri)["diagnostics"][0]["code"], "types");
    project.write("lint.yaml", "exclude: [src/**]\nrules: []\n");
    server.notify(
        "textDocument/didSave",
        json!({"textDocument":{"uri":project.uri("lint.yaml")}}),
    );
    assert_eq!(server.diagnostics(&uri)["diagnostics"], json!([]));
    server.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}));
    assert_eq!(server.diagnostics(&uri)["diagnostics"], json!([]));
    server.notify(
        "textDocument/didSave",
        json!({"textDocument":{"uri":project.uri("lint.yaml")}}),
    );
    // The closed buffer must not receive a fresh diagnostic publication.
    assert_eq!(
        server.request("textDocument/hover", json!({}))["error"]["code"],
        -32601
    );
    server.shutdown();
}

#[test]
fn survives_bad_notifications_and_rejects_stale_changes_and_unknown_requests() {
    let project = TempProject::new();
    let mut server = project.server();
    let uri = project.uri("src/Order.java");
    server.notify("textDocument/didOpen", json!({"broken":true}));
    server.open(&uri, "class Order {}");
    assert_eq!(server.diagnostics(&uri)["diagnostics"], json!([]));
    server.change(&uri, 1, "class Purchase {}");
    server.notify("textDocument/didChange", json!({
        "textDocument":{"uri":uri,"version":2},
        "contentChanges":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"text":"x"}]
    }));
    // A request also serves as a barrier: no stale or malformed change
    // should have published diagnostics ahead of this response.
    assert_eq!(
        server.request("textDocument/hover", json!({}))["error"]["code"],
        -32601
    );
    server.change(&uri, 3, "class Order {}");
    assert_eq!(server.diagnostics(&uri)["diagnostics"], json!([]));
    server.shutdown();
}

#[test]
fn exits_with_failure_when_the_client_skips_shutdown() {
    let project = TempProject::new();
    let mut server = project.server();
    server.notify("exit", Value::Null);
    drop(server.input.take());
    assert!(!server.child.wait().unwrap().success());
}
