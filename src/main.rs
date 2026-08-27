use clap::Parser;
use cli::{BackupCommands, Cli, Commands, ProfileSubcommands};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::adapters::all_adapters;
use crate::app::App;
use crate::shim::{HttpShimInterceptor, ShimInterceptor};
use crate::state::{
    AppState, ContainerConfig, GlobalSettings, MountConfig, ProfileConfig, ResourceLimits,
    ServerConfig, ToolFilter, resolve_config_path, save_config_to_path,
};

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod action;
mod adapters;
mod app;
mod backup;
mod cli;
mod components;
mod config;
mod docker;
mod errors;
mod logging;
mod scanner;
mod shim;
mod state;
mod tui;

fn parse_client_selection(s: &str) -> Vec<String> {
    if s.to_lowercase() == "all" {
        all_adapters()
            .into_iter()
            .map(|a| a.name().to_string())
            .collect()
    } else {
        s.split(',').map(|s| s.trim().to_string()).collect()
    }
}

fn load_or_create_config(path_override: Option<&Path>) -> (AppState, PathBuf) {
    let config_path = resolve_config_path(path_override);

    if config_path.exists()
        && let Ok(content) = std::fs::read_to_string(&config_path)
        && let Ok(cfg) = serde_json::from_str::<AppState>(&content)
    {
        return (cfg, config_path);
    }

    // Default sample configuration showcasing stdio (containerized with shim), streamable-http, and managed containers
    let mut servers = HashMap::new();

    servers.insert(
        "filesystem-sandbox".to_string(),
        ServerConfig::Local {
            command: Some("mcp/filesystem:latest".to_string()),
            args: vec![],
            container: ContainerConfig {
                auto_start: false,
                image: "mcp/filesystem:latest".to_string(),
                read_only_rootfs: true,
                mounts: vec![MountConfig {
                    host: "/Users/vlad.fratila/code".to_string(),
                    guest: "/workspace".to_string(),
                    read_only: true,
                }],
                ports: Vec::new(),
                network: "none".to_string(),
                resources: Some(ResourceLimits {
                    memory_mb: Some(512),
                    cpus: Some(1.0),
                }),
            },
            env: HashMap::new(),
            tool_filter: ToolFilter {
                allow: vec!["read_file".to_string(), "list_directory".to_string()],
                deny: vec!["write_file".to_string(), "delete_file".to_string()],
                auto_approve: vec![],
            },
            transport: crate::state::LocalTransport::Stdio,
        },
    );

    servers.insert(
        "github-context".to_string(),
        ServerConfig::Local {
            command: Some("npx".to_string()),
            args: vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-github".to_string(),
            ],
            env: HashMap::from([(
                "GITHUB_PERSONAL_ACCESS_TOKEN".to_string(),
                "${keychain:github-pat}".to_string(),
            )]),
            tool_filter: ToolFilter::default(),
            container: ContainerConfig::default(),
            transport: crate::state::LocalTransport::Stdio,
        },
    );

    servers.insert(
        "memory-service".to_string(),
        ServerConfig::Remote {
            url: "http://localhost:8080/mcp".to_string(),
            transport: crate::state::RemoteTransport::StreamableHttp,
            headers: HashMap::from([(
                "Authorization".to_string(),
                "Bearer ${keychain:memory-auth-token}".to_string(),
            )]),
            env: HashMap::new(),
            tool_filter: crate::state::ToolFilter::default(),
            shim_port: None,
        },
    );

    let mut profiles = HashMap::new();
    profiles.insert(
        "default".to_string(),
        ProfileConfig::with_servers(vec![
            "filesystem-sandbox".to_string(),
            "github-context".to_string(),
            "memory-service".to_string(),
        ]),
    );

    let app_cfg = AppState {
        version: "1.0.0".to_string(),
        settings: GlobalSettings {
            active_profile: "default".to_string(),
            docker_socket: None,
            sync_clients: vec![
                "cursor".to_string(),
                "claude_code".to_string(),
                "zed".to_string(),
                "antigravity".to_string(),
            ],
            filter_managed_containers_only: Some(true),
        },
        servers,
        configured_containers: HashMap::new(),
        profiles,
        workspaces: HashMap::new(),
        docker_status: String::new(),
        containers: Vec::new(),
        container_logs: Vec::new(),
        inspector_events: Vec::new(),
        managed_servers: std::collections::HashMap::new(),
    };

    let _ = save_config_to_path(&app_cfg, &config_path);
    (app_cfg, config_path)
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    crate::errors::init()?;
    crate::logging::init()?;

    let cli = Cli::parse();
    let (mut config, config_path) = load_or_create_config(cli.config.as_deref());

    match cli.command {
        None | Some(Commands::Tui) => {
            let mut app = App::new(cli.tick_rate, cli.frame_rate)?;
            app.run().await?;
        }
        Some(Commands::Shim {
            server,
            remote_url,
            port,
            socket,
            allow,
            deny,
            cmd,
        }) => {
            let allow_vec = allow
                .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
                .unwrap_or_default();
            let deny_vec = deny
                .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
                .unwrap_or_default();

            let filter = ToolFilter {
                allow: allow_vec,
                deny: deny_vec,
                auto_approve: vec![],
            };

            if let Some(url) = remote_url {
                let http_interceptor =
                    HttpShimInterceptor::new(server, url, filter, socket.as_deref());
                http_interceptor.run(port).await?;
            } else {
                let (command, args) = cmd
                    .split_first()
                    .ok_or_else(|| color_eyre::eyre::eyre!("No command specified for shim"))?;
                let interceptor = ShimInterceptor::new(
                    server,
                    command.clone(),
                    args.to_vec(),
                    HashMap::new(),
                    filter,
                    socket.as_deref(),
                );

                let exit_code = interceptor.run().await?;
                std::process::exit(exit_code);
            }
        }
        Some(Commands::Sync { profile, client }) => {
            let target_profile = profile.unwrap_or_else(|| config.settings.active_profile.clone());
            let adapters = all_adapters();
            println!(
                "🔄 Synchronizing MCP configurations for profile '{}'...",
                target_profile
            );

            let profile_cfg = config.profiles.get(&target_profile);
            let servers_to_sync = if let Some(p) = profile_cfg {
                if !p.enabled_servers.is_empty() {
                    config
                        .servers
                        .iter()
                        .filter(|(name, _)| p.is_server_enabled(name))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                } else if config.profiles.len() == 1 {
                    config.servers.clone()
                } else {
                    HashMap::new()
                }
            } else {
                config.servers.clone()
            };

            for adapter in adapters {
                if let Some(ref target) = client {
                    if adapter.name() != target.to_lowercase() {
                        continue;
                    }
                } else if let Some(p) = profile_cfg
                    && !p.is_client_enabled(adapter.name())
                {
                    println!(
                        "  ○ {:<14} (Disabled in profile '{}', skipping)",
                        adapter.display_name(),
                        target_profile
                    );
                    continue;
                }

                match adapter.config_path(None) {
                    Ok(path) => {
                        match adapter.write_servers(&target_profile, &path, &servers_to_sync) {
                            Ok(_) => {
                                println!("  ✔ {:<14} -> {}", adapter.display_name(), path.display())
                            }
                            Err(e) => eprintln!("  ✖ {:<14} failed: {}", adapter.display_name(), e),
                        }
                    }
                    Err(e) => eprintln!("  ✖ {:<14} path error: {}", adapter.display_name(), e),
                }
            }
            println!("Done.");
        }
        Some(Commands::Backup { action }) => match action {
            BackupCommands::List { profile, client } => {
                let backups = crate::backup::list_backups(profile.as_deref(), client.as_deref())?;
                if backups.is_empty() {
                    println!("No client backups found in storage.");
                    println!(
                        "Tip: Backups are automatically created before syncing, or manually via 'tailery backup create'."
                    );
                } else {
                    println!("Stored Client Backups (up to 10 retained per profile-client pair):");
                    println!(
                        "{:<12} {:<14} {:<24} {:<26} {:<10} TARGET FILE",
                        "PROFILE", "CLIENT", "BACKUP ID", "TIMESTAMP", "SIZE"
                    );
                    println!("{}", "-".repeat(110));
                    for b in backups {
                        let size_str = format!("{} B", b.size_bytes);
                        println!(
                            "{:<12} {:<14} {:<24} {:<26} {:<10} {}",
                            b.profile,
                            b.client,
                            b.id,
                            b.timestamp,
                            size_str,
                            b.original_path.display()
                        );
                    }
                }
            }
            BackupCommands::Create { profile, client } => {
                let target_profile =
                    profile.unwrap_or_else(|| config.settings.active_profile.clone());
                let adapters = all_adapters();
                let mut created_count = 0;
                println!(
                    "📦 Creating client configuration backups for profile '{}'...",
                    target_profile
                );

                for adapter in adapters {
                    if let Some(ref target) = client
                        && adapter.name() != target.to_lowercase()
                    {
                        continue;
                    }
                    match adapter.config_path(None) {
                        Ok(path) => {
                            if path.exists() {
                                match crate::backup::create_backup(
                                    &target_profile,
                                    adapter.name(),
                                    &path,
                                ) {
                                    Ok(Some(entry)) => {
                                        created_count += 1;
                                        println!(
                                            "  ✔ {:<14} -> {} (ID: {})",
                                            adapter.display_name(),
                                            entry.backup_path.display(),
                                            entry.id
                                        );
                                    }
                                    Ok(None) => println!(
                                        "  ○ {:<14} file does not exist ({})",
                                        adapter.display_name(),
                                        path.display()
                                    ),
                                    Err(e) => eprintln!(
                                        "  ✖ {:<14} backup failed: {}",
                                        adapter.display_name(),
                                        e
                                    ),
                                }
                            } else {
                                println!(
                                    "  ○ {:<14} file does not exist ({})",
                                    adapter.display_name(),
                                    path.display()
                                );
                            }
                        }
                        Err(e) => eprintln!("  ✖ {:<14} path error: {}", adapter.display_name(), e),
                    }
                }
                println!(
                    "Created {} backup(s). Older backups pruned to 10 max per profile-client pair.",
                    created_count
                );
            }
            BackupCommands::Restore {
                profile,
                client,
                id,
            } => {
                let target_profile = profile
                    .as_deref()
                    .unwrap_or(&config.settings.active_profile);
                if let Some(ref backup_id) = id {
                    match crate::backup::restore_by_id_or_client(
                        backup_id,
                        Some(target_profile),
                        None,
                    ) {
                        Ok(entry) => {
                            println!(
                                "✔ Successfully restored backup '{}' for client '{}' in profile '{}' to {}",
                                entry.id,
                                entry.client,
                                entry.profile,
                                entry.original_path.display()
                            );
                        }
                        Err(e) => eprintln!("✖ Failed to restore backup ID '{}': {}", backup_id, e),
                    }
                } else if let Some(ref target_client) = client {
                    match crate::backup::restore_latest(target_profile, target_client, None) {
                        Ok(entry) => {
                            println!(
                                "✔ Successfully restored latest backup '{}' for client '{}' in profile '{}' to {}",
                                entry.id,
                                entry.client,
                                entry.profile,
                                entry.original_path.display()
                            );
                        }
                        Err(e) => eprintln!(
                            "✖ Failed to restore latest backup for '{}' in profile '{}': {}",
                            target_client, target_profile, e
                        ),
                    }
                } else {
                    eprintln!(
                        "✖ Error: Please specify either --client <name> to restore the latest backup, or --id <id> to restore a specific backup."
                    );
                }
            }
        },
        Some(Commands::Profile { action }) => match action {
            ProfileSubcommands::List => {
                println!("Configured Profiles:");
                println!(
                    "{:<2} {:<18} {:<16} {:<30}",
                    "", "PROFILE NAME", "SERVERS", "ENABLED CLIENTS"
                );
                println!("{}", "-".repeat(70));
                let mut names: Vec<String> = config.profiles.keys().cloned().collect();
                names.sort();
                for name in names {
                    let is_active = name == config.settings.active_profile;
                    let marker = if is_active { "●" } else { " " };
                    let profile = config.profiles.get(&name);
                    let servers_str = format!(
                        "{} servers",
                        profile.map(|p| p.enabled_servers.len()).unwrap_or(0)
                    );
                    let clients_str = profile
                        .map(|p| p.enabled_clients.join(", "))
                        .unwrap_or_else(|| "all".to_string());
                    println!(
                        "{:<2} {:<18} {:<16} {:<30}",
                        marker, name, servers_str, clients_str
                    );
                }
            }
            ProfileSubcommands::Create {
                name,
                clients,
                copy_servers,
                switch,
            } => {
                let enabled_clients = parse_client_selection(&clients);
                let enabled_servers = if copy_servers {
                    config
                        .profiles
                        .get(&config.settings.active_profile)
                        .map(|p| p.enabled_servers.clone())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };

                config.profiles.insert(
                    name.clone(),
                    ProfileConfig::new(enabled_servers, enabled_clients.clone()),
                );

                if switch {
                    config.settings.active_profile = name.clone();
                }

                let _ = save_config_to_path(&config, &config_path);
                println!(
                    "✔ Created profile '{}' with {} enabled client(s) [{}]",
                    name,
                    enabled_clients.len(),
                    enabled_clients.join(", ")
                );
                if switch {
                    println!("✔ Switched active profile to '{}'", name);
                }
            }
            ProfileSubcommands::Switch { name } => {
                if !config.profiles.contains_key(&name) {
                    eprintln!("✖ Profile '{}' does not exist.", name);
                } else {
                    config.settings.active_profile = name.clone();
                    let _ = save_config_to_path(&config, &config_path);
                    println!("✔ Switched active profile to '{}'", name);
                }
            }
            ProfileSubcommands::EnableClient { client, profile } => {
                let prof_name = profile.unwrap_or_else(|| config.settings.active_profile.clone());
                let prof =
                    config
                        .profiles
                        .entry(prof_name.clone())
                        .or_insert_with(|| ProfileConfig {
                            enabled_servers: Vec::new(),
                            enabled_clients: crate::state::default_enabled_clients(),
                            include_project_mcps: false,
                            project_search_paths: Vec::new(),
                        });
                prof.enable_client(&client);
                let _ = save_config_to_path(&config, &config_path);
                println!("✔ Enabled client '{}' in profile '{}'", client, prof_name);
            }
            ProfileSubcommands::DisableClient { client, profile } => {
                let prof_name = profile.unwrap_or_else(|| config.settings.active_profile.clone());
                let prof =
                    config
                        .profiles
                        .entry(prof_name.clone())
                        .or_insert_with(|| ProfileConfig {
                            enabled_servers: Vec::new(),
                            enabled_clients: crate::state::default_enabled_clients(),
                            include_project_mcps: false,
                            project_search_paths: Vec::new(),
                        });
                prof.disable_client(&client);
                let _ = save_config_to_path(&config, &config_path);
                println!("✔ Disabled client '{}' in profile '{}'", client, prof_name);
            }
            ProfileSubcommands::EnableServer { server, profile } => {
                let prof_name = profile.unwrap_or_else(|| config.settings.active_profile.clone());
                let prof =
                    config
                        .profiles
                        .entry(prof_name.clone())
                        .or_insert_with(|| ProfileConfig {
                            enabled_servers: Vec::new(),
                            enabled_clients: crate::state::default_enabled_clients(),
                            include_project_mcps: false,
                            project_search_paths: Vec::new(),
                        });
                prof.enable_server(&server);
                let _ = save_config_to_path(&config, &config_path);
                println!("✔ Enabled server '{}' in profile '{}'", server, prof_name);
            }
            ProfileSubcommands::DisableServer { server, profile } => {
                let prof_name = profile.unwrap_or_else(|| config.settings.active_profile.clone());
                let prof =
                    config
                        .profiles
                        .entry(prof_name.clone())
                        .or_insert_with(|| ProfileConfig {
                            enabled_servers: Vec::new(),
                            enabled_clients: crate::state::default_enabled_clients(),
                            include_project_mcps: false,
                            project_search_paths: Vec::new(),
                        });
                prof.disable_server(&server);
                let _ = save_config_to_path(&config, &config_path);
                println!("✔ Disabled server '{}' in profile '{}'", server, prof_name);
            }
            ProfileSubcommands::Delete { name } => {
                if config.profiles.len() <= 1 {
                    eprintln!("✖ Cannot delete the only profile.");
                } else if !config.profiles.contains_key(&name) {
                    eprintln!("✖ Profile '{}' not found.", name);
                } else {
                    config.profiles.remove(&name);
                    if config.settings.active_profile == name {
                        config.settings.active_profile = config
                            .profiles
                            .keys()
                            .next()
                            .cloned()
                            .unwrap_or_else(|| "default".to_string());
                    }
                    let _ = save_config_to_path(&config, &config_path);
                    println!("✔ Deleted profile '{}'", name);
                }
            }
        },
        Some(Commands::Restore {
            profile,
            client,
            id,
        }) => {
            let target_profile = profile
                .as_deref()
                .unwrap_or(&config.settings.active_profile);
            if let Some(ref backup_id) = id {
                match crate::backup::restore_by_id_or_client(backup_id, Some(target_profile), None)
                {
                    Ok(entry) => {
                        println!(
                            "✔ Successfully restored backup '{}' for client '{}' in profile '{}' to {}",
                            entry.id,
                            entry.client,
                            entry.profile,
                            entry.original_path.display()
                        );
                    }
                    Err(e) => eprintln!("✖ Failed to restore backup ID '{}': {}", backup_id, e),
                }
            } else if let Some(ref target_client) = client {
                match crate::backup::restore_latest(target_profile, target_client, None) {
                    Ok(entry) => {
                        println!(
                            "✔ Successfully restored latest backup '{}' for client '{}' in profile '{}' to {}",
                            entry.id,
                            entry.client,
                            entry.profile,
                            entry.original_path.display()
                        );
                    }
                    Err(e) => eprintln!(
                        "✖ Failed to restore latest backup for '{}' in profile '{}': {}",
                        target_client, target_profile, e
                    ),
                }
            } else {
                eprintln!(
                    "✖ Error: Please specify either --client <name> to restore the latest backup, or --id <id> to restore a specific backup."
                );
            }
        }
        Some(Commands::List) => {
            println!("Tailery Config: {}", config_path.display());
            println!(
                "Config Dir:     {}",
                crate::config::get_config_dir().display()
            );
            println!(
                "Data Dir:       {}",
                crate::config::get_data_dir().display()
            );
            println!("Active Profile: {}", config.settings.active_profile);

            let (_docker_mgr, daemon_info) =
                crate::docker::DockerClient::auto_detect(config.settings.docker_socket.as_deref())
                    .await;
            if let Some(ref info) = daemon_info {
                let status_str = if info.is_running {
                    "● Online"
                } else {
                    "⚠ Offline"
                };
                println!(
                    "Runtime Daemon: {} ({}) [{}]",
                    info.name, info.socket_path, status_str
                );
            } else {
                println!("Runtime Daemon: None detected");
            }

            println!("\nProfiles:");
            println!(
                "{:<2} {:<18} {:<16} {:<30}",
                "", "PROFILE NAME", "SERVERS", "ENABLED CLIENTS"
            );
            println!("{}", "-".repeat(70));
            let mut pnames: Vec<String> = config.profiles.keys().cloned().collect();
            pnames.sort();
            for name in pnames {
                let marker = if name == config.settings.active_profile {
                    "●"
                } else {
                    " "
                };
                let p = config.profiles.get(&name);
                let sc = format!(
                    "{} servers",
                    p.map(|x| x.enabled_servers.len()).unwrap_or(0)
                );
                let cc = p
                    .map(|x| x.enabled_clients.join(", "))
                    .unwrap_or_else(|| "all".to_string());
                println!("{:<2} {:<18} {:<16} {:<30}", marker, name, sc, cc);
            }

            println!("\nMCP Servers:");
            println!(
                "{:<24} {:<10} {:<18} {:<12}",
                "SERVER NAME", "SCOPE", "TRANSPORT", "STATUS"
            );
            println!("{}", "-".repeat(68));
            for (name, s) in &config.servers {
                let scope = match s {
                    crate::state::ServerConfig::Local { .. } => "local",
                    crate::state::ServerConfig::Remote { url, .. } => {
                        if crate::components::mcps::is_remote_url(url) {
                            "remote"
                        } else {
                            "local"
                        }
                    }
                };
                let transport = match s {
                    crate::state::ServerConfig::Local { transport, .. } => match transport {
                        crate::state::LocalTransport::Stdio => "stdio",
                        crate::state::LocalTransport::StreamableHttp { .. } => "streamable-http",
                        crate::state::LocalTransport::Http { .. } => "http",
                        crate::state::LocalTransport::Sse { .. } => "sse",
                    },
                    crate::state::ServerConfig::Remote { transport, .. } => match transport {
                        crate::state::RemoteTransport::StreamableHttp => "streamable-http",
                        crate::state::RemoteTransport::Http => "http",
                        crate::state::RemoteTransport::Sse => "sse",
                    },
                };
                println!(
                    "{:<24} {:<10} {:<18} {:<12}",
                    name, scope, transport, "configured"
                );
            }
        }
    }

    Ok(())
}
