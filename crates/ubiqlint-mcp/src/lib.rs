//! The MCP server of ubiqlint, for coding agents.
//!
//! [`serve_stdio`] serves one project, the directory of a configuration
//! file, over standard input and output. Its tools give the dictionary and
//! the rules that apply to a file, check source code, and edit dictionaries
//! the way `ubiqlint term` and `ubiqlint behavior` do. See section 6.2 of
//! `docs/plan.md`.
//!
//! The server keeps no state between calls: each call reads the
//! configuration and the dictionaries again, so it always answers for the
//! files as they are on disk. Results are text in the same format as the
//! command line, so that agents see what CI would report.

mod check;
mod context;
mod edit;
mod params;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::ServerInitializeError;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use ubiqlint_project::{Project, report};

use crate::params::{
    AddBehaviorParams, AddTermParams, CheckCodeParams, CheckFilesParams, ContextParams,
    EditBehaviorParams, EditTermParams, RemoveBehaviorParams, RemoveTermParams,
};

/// Serves the project of the configuration file at `config_path` over
/// standard input and output, until the client closes the input.
///
/// The server first moves to the configuration file's directory, so that
/// every path it takes and gives, including the dictionary paths in
/// messages, is relative to that directory. It starts even if the
/// configuration file is missing or invalid, and reports that from each
/// tool instead: a server that fails to start only looks like a missing
/// one to an agent.
pub fn serve_stdio(config_path: &Path) -> io::Result<()> {
    let config = enter_config_directory(config_path);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let service = match Server::new(config).serve(rmcp::transport::stdio()).await {
            Ok(service) => service,
            // The client closed its end before any session began, perhaps
            // after only probing with `server/discover`: a normal stop.
            Err(ServerInitializeError::ConnectionClosed(_)) => return Ok(()),
            Err(error) => return Err(io::Error::other(error)),
        };
        service.waiting().await.map_err(io::Error::other)?;
        Ok(())
    })
}

/// Moves to the directory of the configuration file at `config_path` and
/// returns the file's path from there. If that fails, the current
/// directory and the path are kept, and the tools report the problem.
fn enter_config_directory(config_path: &Path) -> PathBuf {
    let directory = ubiqlint_project::config_root(config_path);
    match config_path.file_name() {
        Some(name) if !directory.as_os_str().is_empty() => {
            match std::env::set_current_dir(directory) {
                Ok(()) => PathBuf::from(name),
                Err(_) => config_path.to_owned(),
            }
        }
        _ => config_path.to_owned(),
    }
}

/// The tools of one project.
#[derive(Clone)]
pub struct Server {
    /// The configuration file, relative to the current directory.
    config: PathBuf,
    /// Held while a dictionary is edited, so that edits called in parallel
    /// do not overwrite each other.
    edits: Arc<Mutex<()>>,
    tool_router: ToolRouter<Self>,
}

/// What agents are told about using the server.
const INSTRUCTIONS: &str = "\
ubiqlint checks that the names in this project's source code follow its ubiquitous language: \
the terms and behaviors in its `terms.yaml` dictionaries, and the naming patterns in its \
`ubiqlint.yaml`.

- Before naming a new type, method, field or variable, call `get_context` with the file's path \
to see its dictionary and the naming rules that apply to it.
- After editing source files, call `check_files` with the files you changed. To check a draft \
before writing it, call `check_code`.
- Fix a violation by using the defined terms and behaviors. Never add a term or behavior only to \
make a violation go away: add one only when it belongs to the domain's ubiquitous language. \
Changes to dictionaries are reviewed.
- Never edit `terms.yaml` directly. Use `add_term`, `edit_term`, `remove_term`, `add_behavior`, \
`edit_behavior` and `remove_behavior`, which check the dictionary before writing it.

Paths are relative to the configuration directory, or absolute.";

#[tool_router]
impl Server {
    /// Creates the tools for the configuration file at `config`, relative
    /// to the current directory.
    pub fn new(config: PathBuf) -> Self {
        Self {
            config,
            edits: Arc::new(Mutex::new(())),
            tool_router: Self::tool_router(),
        }
    }

    /// Shows what a source file must follow: the naming rules and import rules that apply to it,
    /// and the contents of its dictionary, the nearest `terms.yaml`. Call it before naming
    /// anything in the file. In patterns, `{noun}` is a term of the dictionary and
    /// `{noun:plural}` its plural; `{verb}` is a behavior of the term it is bound to (a `{noun}`
    /// in the same name, or else the enclosing type), and `{verb:s}`, `{verb:past}`,
    /// `{verb:pp}` and `{verb:ing}` are its forms; `{any}` is any words; other `{name}`s are
    /// vocabularies; anything else is literal.
    #[tool(
        name = "get_context",
        annotations(
            title = "Get the rules and dictionary of a file",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_context(&self, Parameters(params): Parameters<ContextParams>) -> CallToolResult {
        context::get_context(&self.config, &params)
    }

    /// Checks code before it is written: `content` is checked as the contents of the file at
    /// `path`, which need not exist. Reports each violation as
    /// `path:line:column: error: message`, with the defined terms or behaviors to choose from.
    #[tool(
        name = "check_code",
        annotations(
            title = "Check a draft of a file",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn check_code(&self, Parameters(params): Parameters<CheckCodeParams>) -> CallToolResult {
        check::check_code(&self.config, &params)
    }

    /// Checks source files on disk, as `ubiqlint check` does in CI: the given files, the files
    /// under the given directories, or the whole project. Reports each violation as
    /// `path:line:column: error: message`, with the defined terms or behaviors to choose from.
    #[tool(
        name = "check_files",
        annotations(
            title = "Check files on disk",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn check_files(
        &self,
        Parameters(params): Parameters<CheckFilesParams>,
    ) -> CallToolResult {
        check::check_files(&self.config, &params)
    }

    /// Adds a term, without behaviors, to a dictionary. Every form is given: ubiqlint never
    /// guesses plurals. Only add a term that belongs to the domain's ubiquitous language, never
    /// just to make a violation go away.
    #[tool(
        name = "add_term",
        annotations(
            title = "Add a term to a dictionary",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn add_term(&self, Parameters(params): Parameters<AddTermParams>) -> CallToolResult {
        edit::add_term(self, params)
    }

    /// Changes fields of a term in a dictionary. Give at least one change.
    #[tool(
        name = "edit_term",
        annotations(
            title = "Edit a term of a dictionary",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn edit_term(&self, Parameters(params): Parameters<EditTermParams>) -> CallToolResult {
        edit::edit_term(self, params)
    }

    /// Removes a term, together with its behaviors, from a dictionary.
    #[tool(
        name = "remove_term",
        annotations(
            title = "Remove a term from a dictionary",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn remove_term(
        &self,
        Parameters(params): Parameters<RemoveTermParams>,
    ) -> CallToolResult {
        edit::remove_term(self, params)
    }

    /// Adds a behavior (a verb) to a term of a dictionary. Every form of the verb is given:
    /// ubiqlint never guesses inflections. Only add a behavior that belongs to the domain's
    /// ubiquitous language, never just to make a violation go away.
    #[tool(
        name = "add_behavior",
        annotations(
            title = "Add a behavior to a term",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn add_behavior(
        &self,
        Parameters(params): Parameters<AddBehaviorParams>,
    ) -> CallToolResult {
        edit::add_behavior(self, params)
    }

    /// Changes a behavior of a term in a dictionary. Give at least one change; a new verb needs
    /// all of its other forms too.
    #[tool(
        name = "edit_behavior",
        annotations(
            title = "Edit a behavior of a term",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn edit_behavior(
        &self,
        Parameters(params): Parameters<EditBehaviorParams>,
    ) -> CallToolResult {
        edit::edit_behavior(self, params)
    }

    /// Removes a behavior from a term of a dictionary.
    #[tool(
        name = "remove_behavior",
        annotations(
            title = "Remove a behavior from a term",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn remove_behavior(
        &self,
        Parameters(params): Parameters<RemoveBehaviorParams>,
    ) -> CallToolResult {
        edit::remove_behavior(self, params)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("ubiqlint", env!("CARGO_PKG_VERSION")).with_title("ubiqlint"),
            )
            .with_instructions(INSTRUCTIONS)
    }
}

/// A successful result.
fn success(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text)])
}

/// A result telling the agent what to fix before trying again.
fn failure(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text)])
}

/// Loads the project, or explains why it cannot be used.
fn open(config: &Path) -> Result<Project, CallToolResult> {
    Project::open(config).map_err(|error| {
        let mut lines = report::load_error(config, &error);
        lines.push(format!(
            "The configuration file `{}` must be fixed before anything can be checked.",
            config.display()
        ));
        failure(lines.join("\n"))
    })
}

/// A given path that is not under the configuration directory.
fn outside(path: &str) -> CallToolResult {
    failure(outside_line(path))
}

/// The problem with a given path that is not under the configuration
/// directory, which is named in full since every other path is relative.
fn outside_line(path: &str) -> String {
    let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    report::problem(
        Path::new(path),
        &format!(
            "not under the configuration directory `{}`",
            directory.display()
        ),
    )
}

/// The path of a source file, relative to the project root, or why `path`
/// does not name one.
fn source_file(project: &Project, path: &str) -> Result<PathBuf, CallToolResult> {
    match project.relative(Path::new(path)) {
        None => Err(outside(path)),
        Some(relative)
            if relative.as_os_str().is_empty() || project.read_root().join(&relative).is_dir() =>
        {
            Err(failure(format!(
                "{path}: error: a directory: give the path of a source file, even one that \
                 does not exist yet"
            )))
        }
        Some(relative) => Ok(relative),
    }
}

/// `n` with the singular or plural of a noun.
fn count(n: usize, singular: &str, plural: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural })
}
