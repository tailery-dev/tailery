use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::config::{get_config_dir, get_data_dir};

#[derive(Parser, Debug)]
#[command(author, version = version(), about)]
pub struct Cli {
    /// Tick rate, i.e. number of ticks per second
    #[arg(short, long, value_name = "FLOAT", default_value_t = 4.0)]
    pub tick_rate: f64,

    /// Frame rate, i.e. number of frames per second
    #[arg(short, long, value_name = "FLOAT", default_value_t = 60.0)]
    pub frame_rate: f64,

    /// Path to tailery configuration file
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Launch the interactive Ratatui terminal dashboard (default)
    Tui,

    /// Run headless stdio proxy or HTTP reverse proxy shim with security filtering and telemetry collection
    Shim {
        /// Unique MCP server name identifier
        #[arg(long)]
        server: String,

        /// Remote endpoint URL for HTTP reverse proxy shim
        #[arg(long)]
        remote_url: Option<String>,

        /// Port to bind HTTP reverse proxy listener (defaults to 8080)
        #[arg(long, default_value = "8080")]
        port: u16,

        /// Path to the Unix socket for live telemetry streaming
        #[arg(long)]
        socket: Option<String>,

        /// Comma-separated list of allowed tool names (* for all)
        #[arg(long)]
        allow: Option<String>,

        /// Comma-separated list of denied tool names
        #[arg(long)]
        deny: Option<String>,

        /// MCP target process command and arguments to execute
        #[arg(last = true)]
        cmd: Vec<String>,
    },

    /// Synchronize MCP configurations directly to installed AI coding clients
    Sync {
        /// Target specific profile (defaults to active profile)
        #[arg(short, long)]
        profile: Option<String>,

        /// Target specific client adapter (e.g. cursor, claude_code, zed, antigravity)
        #[arg(short, long)]
        client: Option<String>,
    },

    /// Manage rolling backups (10 max per profile-client pair, stored in XDG)
    Backup {
        #[command(subcommand)]
        action: BackupCommands,
    },

    /// Manage profiles, enabled clients, and switching
    Profile {
        #[command(subcommand)]
        action: ProfileSubcommands,
    },

    /// Quickly restore a client configuration from the latest backup
    Restore {
        /// Profile context (defaults to active profile)
        #[arg(short, long)]
        profile: Option<String>,

        /// Target client adapter (e.g. cursor, claude_code, zed, antigravity)
        #[arg(short, long)]
        client: Option<String>,

        /// Specific backup ID, filename, or client name to restore from
        #[arg(short, long)]
        id: Option<String>,
    },

    /// List configured MCP servers, active profiles, and containers
    List,

    /// Generate shell auto-completions for bash, zsh, fish, or powershell
    Completions {
        /// Target shell (bash, zsh, fish, powershell, elvish)
        shell: clap_complete::Shell,

        /// Output directory to write completion files to (defaults to stdout if omitted)
        #[arg(short, long)]
        out_dir: Option<PathBuf>,
    },

    /// Run comprehensive diagnostics and health checks on system, Docker, adapters, and configs
    Doctor,
}

#[derive(Subcommand, Debug, Clone)]
pub enum BackupCommands {
    /// List all available client backups (retaining up to 10 per profile-client pair)
    List {
        /// Filter by specific profile
        #[arg(short, long)]
        profile: Option<String>,

        /// Filter by specific client adapter (e.g. cursor, claude_code, zed, antigravity)
        #[arg(short, long)]
        client: Option<String>,
    },
    /// Create an immediate backup snapshot of client configuration files
    Create {
        /// Target profile (defaults to active profile)
        #[arg(short, long)]
        profile: Option<String>,

        /// Target specific client adapter (e.g. cursor, claude_code, zed, antigravity)
        #[arg(short, long)]
        client: Option<String>,
    },
    /// Restore a client configuration from a backup
    Restore {
        /// Target profile (defaults to active profile)
        #[arg(short, long)]
        profile: Option<String>,

        /// Target client adapter (e.g. cursor, claude_code, zed, antigravity)
        #[arg(short, long)]
        client: Option<String>,

        /// Specific backup ID or filename to restore
        #[arg(short, long)]
        id: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProfileSubcommands {
    /// List all profiles with their enabled servers and clients
    List,

    /// Create a new profile with custom client selection
    Create {
        /// Profile name
        name: String,

        /// Comma-separated list of enabled clients (e.g. "cursor,zed" or "all")
        #[arg(short, long, default_value = "all")]
        clients: String,

        /// Copy enabled servers from active profile
        #[arg(long)]
        copy_servers: bool,

        /// Switch to this profile immediately after creation
        #[arg(long)]
        switch: bool,
    },

    /// Switch active profile
    Switch {
        /// Name of profile to activate
        name: String,
    },

    /// Enable a client for a profile
    EnableClient {
        /// Client adapter name (e.g. cursor, claude_code, zed, antigravity)
        client: String,

        /// Target profile (defaults to active profile)
        #[arg(short, long)]
        profile: Option<String>,
    },

    /// Disable a client for a profile
    DisableClient {
        /// Client adapter name (e.g. cursor, claude_code, zed, antigravity)
        client: String,

        /// Target profile (defaults to active profile)
        #[arg(short, long)]
        profile: Option<String>,
    },

    /// Enable an MCP server for a profile
    EnableServer {
        /// MCP server identifier name
        server: String,

        /// Target profile (defaults to active profile)
        #[arg(short, long)]
        profile: Option<String>,
    },

    /// Disable an MCP server for a profile
    DisableServer {
        /// MCP server identifier name
        server: String,

        /// Target profile (defaults to active profile)
        #[arg(short, long)]
        profile: Option<String>,
    },

    /// Delete a profile
    Delete {
        /// Name of profile to delete
        name: String,
    },
}

const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "-",
    env!("VERGEN_GIT_DESCRIBE"),
    " (",
    env!("VERGEN_BUILD_DATE"),
    ")"
);

pub fn version() -> String {
    let author = clap::crate_authors!();

    let config_dir_path = get_config_dir().display().to_string();
    let data_dir_path = get_data_dir().display().to_string();

    format!(
        "\
{VERSION_MESSAGE}

Authors: {author}

Config directory: {config_dir_path}
Data directory: {data_dir_path}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_debug_and_version() {
        let v = version();
        assert!(!v.is_empty());
    }

    #[test]
    fn test_shell_completions_generation() {
        let shells = [
            clap_complete::Shell::Bash,
            clap_complete::Shell::Elvish,
            clap_complete::Shell::Fish,
            clap_complete::Shell::PowerShell,
            clap_complete::Shell::Zsh,
        ];

        for shell in shells {
            let mut cmd = Cli::command();
            let mut buffer = Vec::new();
            clap_complete::generate(shell, &mut cmd, "tailery", &mut buffer);
            assert!(!buffer.is_empty());
            let out_str = String::from_utf8(buffer).expect("Valid UTF-8 completions");
            assert!(out_str.contains("tailery"));
        }
    }
}
