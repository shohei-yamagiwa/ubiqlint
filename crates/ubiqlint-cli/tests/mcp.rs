//! `ubiqlint mcp`, driven over standard input and output as an agent's
//! client would.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

/// A running `ubiqlint mcp`, stopped when dropped.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    /// Starts the server for `config`, a path relative to this crate.
    fn start(config: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
            .args(["mcp", "--config", config])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to run ubiqlint mcp");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    /// Sends a request and returns the response to it, skipping any
    /// notifications.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert_ne!(
                self.stdout.read_line(&mut line).unwrap(),
                0,
                "the server closed its output"
            );
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    /// Sends a request of protocol version 2026-07-28, which carries its
    /// version in `_meta` instead of an `initialize` handshake.
    fn modern(&mut self, method: &str, mut params: Value) -> Value {
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "0"},
            "io.modelcontextprotocol/clientCapabilities": {},
        });
        self.request(method, params)
    }

    /// Calls a tool and returns whether it reported an error, and its text.
    fn call(&mut self, tool: &str, arguments: Value) -> (bool, String) {
        let response = self.modern("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &response["result"];
        assert!(result.is_object(), "unexpected response: {response}");
        let text: Vec<_> = result["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|content| content["text"].as_str().unwrap())
            .collect();
        (result["isError"] == json!(true), text.join("\n"))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Closing the input asks the server to stop.
        drop(self.stdin.take());
        if self.child.wait().is_err() {
            let _ = self.child.kill();
        }
    }
}

static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);

/// A copy of a fixture in a temporary directory, removed when dropped.
struct TempProject(PathBuf);

impl TempProject {
    fn copy(fixture: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "ubiqlint-mcp-{}-{}",
            std::process::id(),
            NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture), &path);
        Self(path)
    }

    fn config(&self) -> String {
        self.0.join("ubiqlint.yaml").to_str().unwrap().to_owned()
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.0.join(path)).unwrap()
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

const SAMPLE: &str = "../../examples/order-service/ubiqlint.yaml";
const BAD: &str = "tests/fixtures/check/bad/ubiqlint.yaml";
const ORDER_TERMS: &str = "src/main/java/com/example/order/terms.yaml";

#[test]
fn serves_clients_with_an_initialize_handshake() {
    let mut server = Server::start(SAMPLE);
    let response = server.request(
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"},
        }),
    );
    let result = &response["result"];
    assert_eq!(result["protocolVersion"], "2025-11-25");
    assert_eq!(result["serverInfo"]["name"], "ubiqlint");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        result["instructions"]
            .as_str()
            .unwrap()
            .contains("Never edit `terms.yaml` directly"),
        "{result}"
    );
    server.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    let response = server.request("tools/list", json!({}));
    assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 10);
}

#[test]
fn serves_clients_that_discover_it_without_a_handshake() {
    let mut server = Server::start(SAMPLE);
    let response = server.modern("server/discover", json!({}));
    let result = &response["result"];
    let versions = result["supportedVersions"].as_array().unwrap();
    assert!(versions.contains(&json!("2026-07-28")), "{result}");
    assert!(versions.contains(&json!("2025-11-25")), "{result}");
    assert!(
        result["instructions"]
            .as_str()
            .unwrap()
            .contains("call `get_context`"),
        "{result}"
    );
}

/// The tools' names, descriptions, schemas and annotations are what agents
/// read, so any change to them is deliberate: run the tests with
/// `UBIQLINT_UPDATE_SNAPSHOTS=1` to accept one.
#[test]
fn lists_the_tools_as_recorded() {
    let mut server = Server::start(SAMPLE);
    let response = server.modern("tools/list", json!({}));
    let tools = serde_json::to_string_pretty(&response["result"]["tools"]).unwrap() + "\n";
    let snapshot = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp/tools.json");
    if std::env::var_os("UBIQLINT_UPDATE_SNAPSHOTS").is_some() {
        fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        fs::write(&snapshot, &tools).unwrap();
    }
    assert_eq!(tools, fs::read_to_string(&snapshot).unwrap());

    let names: Vec<_> = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "add_behavior",
            "add_term",
            "check_code",
            "check_files",
            "create_dictionary",
            "edit_behavior",
            "edit_term",
            "get_context",
            "remove_behavior",
            "remove_term",
        ]
    );
    for tool in response["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["annotations"]["openWorldHint"], false, "{tool}");
    }
}

#[test]
fn creates_a_dictionary_then_uses_it_instead_of_the_parent() {
    let project = TempProject::copy("../../examples/order-service");
    let mut server = Server::start(&project.config());
    let dictionary = "src/main/java/com/example/order/domain/terms.yaml";
    let source = "src/main/java/com/example/order/domain/Shipment.java";
    let (error, text) = server.call(
        "create_dictionary",
        json!({"dictionary": dictionary, "context": "shipping"}),
    );
    assert!(!error, "{text}");
    assert_eq!(text, format!("{dictionary}: created dictionary"));
    assert_eq!(project.read(dictionary), "context: shipping\nterms: []\n");

    let (error, text) = server.call("get_context", json!({"path": source}));
    assert!(!error, "{text}");
    assert!(
        text.contains(&format!("Its dictionary is `{dictionary}`:")),
        "{text}"
    );
    assert!(text.contains("terms: []"), "{text}");
    assert!(!text.contains("name: Order"), "{text}");

    let (error, text) = server.call(
        "add_term",
        json!({"dictionary": dictionary, "name": "Shipment", "plural": "shipments"}),
    );
    assert!(!error, "{text}");
    let (error, text) = server.call(
        "check_code",
        json!({"path": source, "content": "class Shipment {}"}),
    );
    assert!(!error, "{text}");
    assert!(project.read(dictionary).contains("name: Shipment"));
}

#[test]
fn create_dictionary_rejects_invalid_arguments_paths_and_existing_files() {
    let project = TempProject::copy("../../examples/order-service");
    let before = project.read(ORDER_TERMS);
    let mut server = Server::start(&project.config());
    for (dictionary, context, expected) in [
        ("terms.yaml", " \n", "`context` must not be empty"),
        (
            "../terms.yaml",
            "outside",
            "not under the configuration directory",
        ),
        (
            "/etc/terms.yaml",
            "outside",
            "not under the configuration directory",
        ),
        ("ubiqlint.yaml", "wrong", "not a dictionary"),
        (ORDER_TERMS, "other", "cannot create the dictionary"),
        (
            "missing/terms.yaml",
            "missing",
            "cannot create the dictionary",
        ),
    ] {
        let (error, text) = server.call(
            "create_dictionary",
            json!({"dictionary": dictionary, "context": context}),
        );
        assert!(error, "{text}");
        assert!(text.contains(expected), "{text}");
    }
    for arguments in [
        json!({"dictionary": "terms.yaml"}),
        json!({"context": "order"}),
        json!({"dictionary": "terms.yaml", "context": "order", "force": true}),
    ] {
        let response = server.modern(
            "tools/call",
            json!({"name": "create_dictionary", "arguments": arguments}),
        );
        assert!(
            response.get("error").is_some() || response["result"]["isError"] == json!(true),
            "{response}"
        );
    }
    assert_eq!(project.read(ORDER_TERMS), before);
    assert!(!project.0.join("terms.yaml").exists());
    assert!(!project.0.join("missing").exists());
}

#[test]
fn create_dictionary_works_without_a_configuration_and_preserves_an_existing_dictionary() {
    let project = TempProject::copy("../../examples/order-service");
    fs::remove_file(project.0.join("ubiqlint.yaml")).unwrap();
    let mut server = Server::start(&project.config());
    let context = "注文: \"受付\"\nterms: []";
    let arguments = json!({"dictionary": "terms.yaml", "context": context});
    let (error, text) = server.call("create_dictionary", arguments.clone());
    assert!(!error, "{text}");
    let before = project.read("terms.yaml");
    let dictionary = ubiqlint_core::Dictionary::from_yaml_str(&before).unwrap();
    assert_eq!(dictionary.context, context);
    assert!(dictionary.terms.is_empty());
    let (error, text) = server.call("create_dictionary", arguments);
    assert!(error, "{text}");
    assert_eq!(project.read("terms.yaml"), before);
}

#[test]
fn get_context_gives_the_rules_and_the_dictionary_of_a_new_file() {
    let mut server = Server::start(SAMPLE);
    let (error, text) = server.call(
        "get_context",
        json!({"path": "src/main/java/com/example/order/domain/Shipment.java"}),
    );

    assert!(!error, "{text}");
    for expected in [
        "naming rule `domain-type`, for type names: `{noun}`, `{noun}{role}`, `{noun}{verb:pp}`",
        "naming rule `domain-method`, for method names: `{verb}`, `{verb}{noun}`",
        "import rule `order-imports`: an import from outside `com.example.order`",
        "`{role}` is one of: `repository`, `factory`, `policy`.",
        "Its dictionary is `src/main/java/com/example/order/terms.yaml`:",
        "    ja: 注文\n",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
    // Rules for other paths are left out.
    assert!(!text.contains("dto-type"), "{text}");
    assert!(!text.contains("billing-imports"), "{text}");
}

#[test]
fn get_context_explains_files_it_does_not_check() {
    let mut server = Server::start(SAMPLE);

    let (error, text) = server.call("get_context", json!({"path": "README.md"}));
    assert!(!error, "{text}");
    assert_eq!(
        text,
        "`README.md` is not checked: ubiqlint cannot check this kind of file."
    );

    let (error, text) = server.call(
        "get_context",
        json!({"path": "src/test/java/OrderTest.java"}),
    );
    assert!(!error, "{text}");
    assert_eq!(
        text,
        "No rule in `ubiqlint.yaml` applies to `src/test/java/OrderTest.java`: its names and \
         imports are not checked."
    );
}

#[test]
fn rejects_paths_that_do_not_name_a_source_file_in_the_project() {
    let mut server = Server::start(SAMPLE);

    let (error, text) = server.call("get_context", json!({"path": "/etc/hosts"}));
    assert!(error);
    assert!(
        text.starts_with("/etc/hosts: error: not under the configuration directory `/"),
        "{text}"
    );

    let (error, text) = server.call("get_context", json!({"path": "src"}));
    assert!(error);
    assert!(text.starts_with("src: error: a directory"), "{text}");

    let (error, text) = server.call(
        "check_code",
        json!({"path": "../Order.java", "content": "class Order {}"}),
    );
    assert!(error, "{text}");
}

#[test]
fn check_code_checks_a_draft_without_writing_it() {
    let mut server = Server::start(SAMPLE);
    let path = "src/main/java/com/example/order/domain/Order.java";
    let before = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/order-service")
            .join(path),
    )
    .unwrap();

    let (error, text) = server.call(
        "check_code",
        json!({"path": path, "content": "class Order { void updateOrder() {} }"}),
    );
    assert!(!error, "{text}");
    assert_eq!(
        text,
        format!(
            "{path}:1:20: error: `updateOrder` does not match pattern `{{verb}}{{noun}}` of rule \
             `domain-method`: `update` is not a defined behavior of `Order` (defined behaviors: \
             `place`, `cancel`)\n1 problem in `{path}`."
        )
    );

    let (error, text) = server.call(
        "check_code",
        json!({"path": path, "content": "class Order { void place() {} }"}),
    );
    assert!(!error, "{text}");
    assert_eq!(text, format!("No problems in `{path}`."));

    let (error, text) = server.call(
        "check_code",
        json!({"path": path, "content": "class Order {"}),
    );
    assert!(!error, "{text}");
    assert!(
        text.ends_with(&format!(
            "1 syntax error in `{path}`, so its names and imports were not checked."
        )),
        "{text}"
    );

    // The file on disk is untouched.
    let after = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/order-service")
            .join(path),
    )
    .unwrap();
    assert_eq!(before, after);
}

#[test]
fn check_files_reports_what_the_command_line_reports() {
    let mut server = Server::start(BAD);
    let (error, text) = server.call("check_files", json!({}));

    let cli = Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
        .args(["check", "--config", BAD])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    let expected = String::from_utf8(cli.stdout)
        .unwrap()
        .replace("tests/fixtures/check/bad/", "");

    assert!(!error, "{text}");
    assert_eq!(text, format!("{expected}Checked 3 files: 3 problems."));
}

#[test]
fn check_files_checks_only_the_given_paths_and_limits_the_problems_listed() {
    let mut server = Server::start(BAD);
    let domain = "src/main/java/com/example/order/domain";

    let (error, text) = server.call(
        "check_files",
        json!({"paths": [format!("{domain}/Order.java"), "terms.yaml"]}),
    );
    assert!(!error, "{text}");
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(lines[0].starts_with(&format!("{domain}/Order.java:7:17: error: ")));
    assert_eq!(
        lines[1],
        "terms.yaml: warning: not checked: ubiqlint cannot check this kind of file"
    );
    assert_eq!(lines[2], "Checked 1 file: 1 problem.");

    let (error, text) = server.call("check_files", json!({"limit": 1}));
    assert!(!error, "{text}");
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert_eq!(
        lines[1],
        "... and 2 more problems. Check fewer paths, or raise `limit`, to see them."
    );
    assert_eq!(lines[2], "Checked 3 files: 3 problems.");

    let (error, text) = server.call("check_files", json!({"paths": ["/etc"]}));
    assert!(error, "{text}");
    assert!(text.ends_with("No source files were checked."), "{text}");
}

#[test]
fn check_code_needs_a_dictionary_and_reports_a_broken_one() {
    let project = TempProject::copy("tests/fixtures/check/ok");
    let mut server = Server::start(&project.config());
    let draft = json!({
        "path": "src/main/java/com/example/order/domain/Order.java",
        "content": "class Order { void place() {} }",
    });

    let (error, text) = server.call("check_code", draft.clone());
    assert!(!error, "{text}");

    fs::write(
        project.0.join("terms.yaml"),
        "context: broken\nterms: wrong\n",
    )
    .unwrap();
    let (error, text) = server.call("check_code", draft.clone());
    assert!(error, "{text}");
    assert!(text.starts_with("terms.yaml:2:8: error: "), "{text}");

    fs::remove_file(project.0.join("terms.yaml")).unwrap();
    let (error, text) = server.call("check_code", draft);
    assert!(error, "{text}");
    assert!(
        text.starts_with(
            "src/main/java/com/example/order/domain: error: no `terms.yaml` found from this \
             directory up to the configuration directory"
        ),
        "{text}"
    );
}

#[test]
fn edits_dictionaries_and_checks_with_them_at_once() {
    let project = TempProject::copy("../../examples/order-service");
    let mut server = Server::start(&project.config());
    let draft = json!({
        "path": "src/main/java/com/example/order/domain/Order.java",
        "content": "class Order { void ship() {} }",
    });
    let ship = json!({
        "dictionary": ORDER_TERMS,
        "term": "Order",
        "verb": "ship",
        "s": "ships",
        "past": "shipped",
        "pp": "shipped",
        "ing": "shipping",
    });

    let (_, text) = server.call("check_code", draft.clone());
    assert!(
        text.contains("`ship` is not a defined behavior of `Order`"),
        "{text}"
    );

    let (error, text) = server.call("add_behavior", ship.clone());
    assert!(!error, "{text}");
    assert_eq!(
        text,
        format!("{ORDER_TERMS}: added behavior `ship` to term `Order`")
    );
    assert!(project.read(ORDER_TERMS).contains("- verb: ship"));

    // The server keeps no state, so the next check sees the new behavior.
    let (error, text) = server.call("check_code", draft);
    assert!(!error, "{text}");
    assert!(text.starts_with("No problems"), "{text}");

    let edited = project.read(ORDER_TERMS);
    let (error, text) = server.call("add_behavior", ship);
    assert!(error);
    assert_eq!(
        text,
        format!(
            "{ORDER_TERMS}: error: behavior `ship` is already defined on term `Order` (line 13)"
        )
    );
    assert_eq!(project.read(ORDER_TERMS), edited);

    let (error, text) = server.call(
        "edit_term",
        json!({"dictionary": ORDER_TERMS, "name": "Order", "ja": "注文"}),
    );
    assert!(!error, "{text}");
    assert_eq!(
        text,
        format!("{ORDER_TERMS}: no changes: term `Order` already has these values")
    );

    let (error, text) = server.call(
        "remove_behavior",
        json!({"dictionary": ORDER_TERMS, "term": "Order", "verb": "ship"}),
    );
    assert!(!error, "{text}");
    assert_eq!(
        text,
        format!("{ORDER_TERMS}: removed behavior `ship` from term `Order`")
    );
    assert_eq!(
        project.read(ORDER_TERMS),
        fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/order-service")
                .join(ORDER_TERMS)
        )
        .unwrap()
    );
}

#[test]
fn rejects_invalid_edits_without_touching_the_dictionary() {
    let project = TempProject::copy("../../examples/order-service");
    let mut server = Server::start(&project.config());
    let before = project.read(ORDER_TERMS);

    let (error, text) = server.call(
        "edit_term",
        json!({"dictionary": ORDER_TERMS, "name": "Order", "plural": "Orders", "kind": " ", "ja": "x", "unset_ja": true}),
    );
    assert!(error);
    assert_eq!(
        text,
        "invalid arguments; the dictionary was not changed:\n\
         - `plural`: `Orders` is not a valid word sequence: use lowercase letters and digits, \
         with a single space between words (e.g. `line item`)\n\
         - `kind`: must not be empty\n\
         - give either `ja` or `unset_ja`, not both"
    );

    let (error, text) = server.call(
        "edit_term",
        json!({"dictionary": ORDER_TERMS, "name": "Order"}),
    );
    assert!(error);
    assert!(text.contains("give at least one change"), "{text}");

    let (error, text) = server.call(
        "remove_term",
        json!({"dictionary": "ubiqlint.yaml", "name": "Order"}),
    );
    assert!(error);
    assert_eq!(
        text,
        "ubiqlint.yaml: error: not a dictionary: dictionaries are files named `terms.yaml`"
    );

    let (error, text) = server.call(
        "remove_term",
        json!({"dictionary": "/etc/terms.yaml", "name": "Order"}),
    );
    assert!(error);
    assert!(
        text.starts_with("/etc/terms.yaml: error: not under the configuration directory"),
        "{text}"
    );

    let (error, text) = server.call(
        "add_term",
        json!({"dictionary": ORDER_TERMS, "name": "Order", "plural": "orders"}),
    );
    assert!(error);
    assert!(text.contains("`Order`"), "{text}");

    // Unknown arguments are refused, so that a typo is not silently ignored.
    let response = server.modern(
        "tools/call",
        json!({"name": "remove_term", "arguments": {"dictionary": ORDER_TERMS, "term": "Order"}}),
    );
    assert!(
        response.get("error").is_some() || response["result"]["isError"] == json!(true),
        "{response}"
    );

    assert_eq!(project.read(ORDER_TERMS), before);
}

#[test]
fn reports_an_invalid_configuration_from_every_tool_but_still_edits_dictionaries() {
    let project = TempProject::copy("../../examples/order-service");
    fs::write(project.0.join("ubiqlint.yaml"), "rules: wrong\n").unwrap();
    let mut server = Server::start(&project.config());

    for (tool, arguments) in [
        ("get_context", json!({"path": "src/Order.java"})),
        (
            "check_code",
            json!({"path": "src/Order.java", "content": "class Order {}"}),
        ),
        ("check_files", json!({})),
    ] {
        let (error, text) = server.call(tool, arguments);
        assert!(error, "{tool}: {text}");
        assert!(
            text.starts_with("ubiqlint.yaml:1:8: error: "),
            "{tool}: {text}"
        );
        assert!(
            text.ends_with(
                "The configuration file `ubiqlint.yaml` must be fixed before anything can be \
                 checked."
            ),
            "{tool}: {text}"
        );
    }

    let (error, text) = server.call(
        "add_term",
        json!({"dictionary": ORDER_TERMS, "name": "Shipment", "plural": "shipments"}),
    );
    assert!(!error, "{text}");
    assert!(project.read(ORDER_TERMS).contains("name: Shipment"));
}

/// A client may only probe the server with `server/discover`, or start it
/// and go away: closing the input then is a normal stop, not a failure.
#[test]
fn stops_without_an_error_when_the_client_leaves_before_a_session() {
    for input in [
        format!(
            "{}\n",
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "server/discover",
                "params": {"_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {},
                }},
            })
        ),
        String::new(),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
            .args(["mcp", "--config", SAMPLE])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();

        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
        let answers = String::from_utf8(output.stdout).unwrap();
        assert_eq!(answers.lines().count(), input.lines().count(), "{answers}");
    }
}

impl TempProject {
    /// A project whose rules only cover `src/**`, with a valid dictionary
    /// for `src`, a conforming `src/Order.java` and a rule-less
    /// `tests/Bad.java`.
    fn with_rules_for_src_only() -> Self {
        let project = Self::copy("tests/fixtures/check/nofiles");
        let write = |path: &str, contents: &str| {
            let path = project.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        };
        write(
            "ubiqlint.yaml",
            "rules:\n  - id: domain-method\n    include: [\"src/**\"]\n    target: method\n    patterns:\n      - \"{verb}\"\n",
        );
        let _ = fs::remove_file(project.0.join("terms.yaml"));
        write(
            "src/terms.yaml",
            "context: order\nterms:\n  - name: Order\n    forms: { singular: order, plural: orders }\n    behaviors:\n      - verb: place\n        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }\n",
        );
        write("src/Order.java", "class Order { void place() {} }\n");
        write("tests/Bad.java", "class Bad { void update() {} }\n");
        project
    }
}

/// A file that no rule applies to was only parsed: it must not read as one
/// whose names follow the rules.
#[test]
fn check_files_tells_files_that_no_rule_applies_to_from_files_without_problems() {
    let project = TempProject::with_rules_for_src_only();
    let mut server = Server::start(&project.config());
    let not_checked = "tests/Bad.java: warning: names and imports not checked: no rule in `ubiqlint.yaml` \
         applies to it";

    let (error, text) = server.call("check_files", json!({"paths": ["tests/Bad.java"]}));
    assert!(!error, "{text}");
    assert_eq!(
        text,
        format!("{not_checked}\nNo source files were checked against the rules.")
    );

    let (error, text) = server.call("check_files", json!({}));
    assert!(!error, "{text}");
    assert_eq!(text, format!("{not_checked}\nChecked 1 file: no problems."));

    let (error, text) = server.call(
        "check_files",
        json!({"limit": 1, "paths": ["tests", "src"]}),
    );
    assert!(!error, "{text}");
    assert_eq!(text, format!("{not_checked}\nChecked 1 file: no problems."));
}

/// CI reports a broken root dictionary even when no source file uses it,
/// so `check_files` must too, or a change it passes would fail CI.
#[test]
fn check_files_reports_a_broken_root_dictionary_as_the_command_line_does() {
    let project = TempProject::with_rules_for_src_only();
    fs::write(
        project.0.join("terms.yaml"),
        "context: broken\nterms: wrong\n",
    )
    .unwrap();
    let mut server = Server::start(&project.config());

    let cli = Command::new(env!("CARGO_BIN_EXE_ubiqlint"))
        .arg("check")
        .current_dir(&project.0)
        .output()
        .unwrap();
    let expected = String::from_utf8(cli.stdout).unwrap();
    assert_eq!(
        expected,
        "terms.yaml:2:8: error: expected `terms` to be a sequence, found a string\n"
    );

    for arguments in [json!({}), json!({"paths": ["src/Order.java"]})] {
        let (error, text) = server.call("check_files", arguments.clone());
        assert!(!error, "{arguments}: {text}");
        assert!(text.starts_with(&expected), "{arguments}: {text}");
        assert_eq!(
            text.matches("terms.yaml:2:8:").count(),
            1,
            "{arguments}: {text}"
        );
        assert!(
            text.ends_with("Checked 1 file: 1 problem."),
            "{arguments}: {text}"
        );
    }
}
