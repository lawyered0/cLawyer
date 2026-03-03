//! CLI command handling.
//!
//! Provides subcommands for:
//! - Running the agent (`run`)
//! - Interactive onboarding wizard (`onboard`)
//! - Managing configuration (`config list`, `config get`, `config set`)
//! - Backup and restore (`backup create`, `backup verify`, `backup restore`)
//! - Managing WASM tools (`tool install`, `tool list`, `tool remove`)
//! - Managing MCP servers (`mcp add`, `mcp auth`, `mcp list`, `mcp test`)
//! - Querying workspace memory (`memory search`, `memory read`, `memory write`)
//! - Managing OS service (`service install`, `service start`, `service stop`)
//! - Active health diagnostics (`doctor`)
//! - Checking system health (`status`)

mod backup;
mod completion;
mod config;
mod doctor;
mod mcp;
pub mod memory;
pub mod oauth_defaults;
mod pairing;
mod registry;
mod service;
pub mod status;
mod tool;

pub use backup::{BackupCommand, run_backup_command};
pub use completion::Completion;
pub use config::{ConfigCommand, run_config_command};
pub use doctor::run_doctor_command;
pub use mcp::{McpCommand, run_mcp_command};
pub use memory::MemoryCommand;
#[cfg(feature = "postgres")]
pub use memory::run_memory_command;
pub use memory::run_memory_command_with_db;
pub use pairing::{PairingCommand, run_pairing_command, run_pairing_command_with_store};
pub use registry::{RegistryCommand, run_registry_command};
pub use service::{ServiceCommand, run_service_command};
pub use status::run_status_command;
pub use tool::{ToolCommand, run_tool_command};

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum LegalProfileArg {
    Standard,
    MaxLockdown,
}

#[derive(Parser, Debug)]
#[command(name = "clawyer")]
#[command(about = "Secure legal AI assistant with matter-scoped workflows and hardening controls")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Run in interactive CLI mode only (disable other channels)
    #[arg(long, global = true)]
    pub cli_only: bool,

    /// Skip database connection (for testing)
    #[arg(long, global = true)]
    pub no_db: bool,

    /// Disable the interactive REPL channel.
    #[arg(long, global = true)]
    pub no_repl: bool,

    /// Run in daemon-friendly mode (disable REPL and onboarding prompts).
    #[arg(long, global = true)]
    pub headless: bool,

    /// Single message mode - send one message and exit
    #[arg(short, long, global = true)]
    pub message: Option<String>,

    /// Configuration file path (optional, uses env vars by default)
    #[arg(short, long, global = true)]
    pub config: Option<std::path::PathBuf>,

    /// Skip first-run onboarding check
    #[arg(long, global = true)]
    pub no_onboard: bool,

    /// Active legal matter ID for this session.
    #[arg(long, global = true)]
    pub matter: Option<String>,

    /// Legal jurisdiction profile (default: us-general).
    #[arg(long, global = true)]
    pub jurisdiction: Option<String>,

    /// Legal hardening profile (default: max-lockdown).
    #[arg(long, value_enum, global = true)]
    pub legal_profile: Option<LegalProfileArg>,

    /// Add an allowed outbound domain in legal deny-by-default mode.
    #[arg(long = "allow-domain", global = true)]
    pub allow_domain: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the agent (default if no subcommand given)
    Run,

    /// Interactive onboarding wizard
    Onboard {
        /// Skip authentication (use existing session)
        #[arg(long)]
        skip_auth: bool,

        /// Reconfigure channels only
        #[arg(long)]
        channels_only: bool,

        /// Use the lawyer-focused quickstart onboarding flow.
        #[arg(long, conflicts_with = "advanced")]
        quickstart: bool,

        /// Use the full advanced onboarding flow.
        #[arg(long, conflicts_with = "quickstart")]
        advanced: bool,
    },

    /// Manage configuration settings
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Backup, restore, and retrieval export operations
    #[command(subcommand)]
    Backup(BackupCommand),

    /// Manage WASM tools
    #[command(subcommand)]
    Tool(ToolCommand),

    /// Browse and install extensions from the registry
    #[command(subcommand)]
    Registry(RegistryCommand),

    /// Manage MCP servers (hosted tool providers)
    #[command(subcommand)]
    Mcp(McpCommand),

    /// Query and manage workspace memory
    #[command(subcommand)]
    Memory(MemoryCommand),

    /// DM pairing (approve inbound requests from unknown senders)
    #[command(subcommand)]
    Pairing(PairingCommand),

    /// Manage OS service (launchd / systemd)
    #[command(subcommand)]
    Service(ServiceCommand),

    /// Probe external dependencies and validate configuration
    Doctor,

    /// Show system health and diagnostics
    Status,

    /// Generate shell completion scripts
    Completion(Completion),

    /// Run as a sandboxed worker inside a Docker container (internal use).
    /// This is invoked automatically by the orchestrator, not by users directly.
    Worker {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,

        /// Maximum iterations before stopping.
        #[arg(long, default_value = "50")]
        max_iterations: u32,
    },

    /// Run as a Claude Code bridge inside a Docker container (internal use).
    /// Spawns the `claude` CLI and streams output back to the orchestrator.
    ClaudeBridge {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,

        /// Maximum agentic turns for Claude Code.
        #[arg(long, default_value = "50")]
        max_turns: u32,

        /// Claude model to use (e.g. "sonnet", "opus").
        #[arg(long, default_value = "sonnet")]
        model: String,
    },
}

impl Cli {
    /// Check if we should run the agent (default behavior or explicit `run` command).
    pub fn should_run_agent(&self) -> bool {
        matches!(self.command, None | Some(Command::Run))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_version() {
        let cmd = Cli::command();
        assert_eq!(
            cmd.get_version().unwrap_or("unknown"),
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn test_parse_no_repl_flag() {
        let cli = Cli::parse_from(["clawyer", "--no-repl", "run"]);
        assert!(cli.no_repl);
        assert!(!cli.headless);
    }

    #[test]
    fn test_parse_headless_flag() {
        let cli = Cli::parse_from(["clawyer", "--headless", "run"]);
        assert!(cli.headless);
        assert!(!cli.no_repl);
    }

    #[test]
    fn test_parse_headless_and_no_repl_flags() {
        let cli = Cli::parse_from(["clawyer", "--headless", "--no-repl", "run"]);
        assert!(cli.headless);
        assert!(cli.no_repl);
    }

    #[test]
    fn test_parse_onboard_quickstart_flag() {
        let cli = Cli::parse_from(["clawyer", "onboard", "--quickstart"]);
        match cli.command {
            Some(Command::Onboard { quickstart, .. }) => assert!(quickstart),
            _ => panic!("expected onboard command"),
        }
    }

    #[test]
    fn test_parse_onboard_advanced_flag() {
        let cli = Cli::parse_from(["clawyer", "onboard", "--advanced"]);
        match cli.command {
            Some(Command::Onboard { advanced, .. }) => assert!(advanced),
            _ => panic!("expected onboard command"),
        }
    }

    #[test]
    fn test_onboard_quickstart_and_advanced_are_mutually_exclusive() {
        let parsed = Cli::try_parse_from(["clawyer", "onboard", "--quickstart", "--advanced"]);
        assert!(parsed.is_err(), "expected clap arg conflict error");
    }
}
