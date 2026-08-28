use std::fs::OpenOptions;
use std::path::Path;
use std::time::Instant;

use crate::adapters::all_adapters;
use crate::config::{get_config_dir, get_data_dir};
use crate::docker::DockerManager;
use crate::state::{AppState, resolve_config_path};

#[derive(Debug, Default)]
pub struct DiagnosticReport {
    pub passed: usize,
    pub warnings: usize,
    pub errors: usize,
}

impl DiagnosticReport {
    pub fn record_pass(&mut self) {
        self.passed += 1;
    }
    pub fn record_warn(&mut self) {
        self.warnings += 1;
    }
    pub fn record_error(&mut self) {
        self.errors += 1;
    }
}

pub async fn run_doctor(
    config_path_override: Option<&Path>,
) -> color_eyre::Result<DiagnosticReport> {
    let mut report = DiagnosticReport::default();

    println!("🦊 Tailery Doctor - System & Environment Diagnostics");
    println!("{}\n", "=".repeat(60));

    // 1. Host & Version Info
    println!("1. System Information");
    println!(
        "   OS:           {} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("   Version:      {}", env!("CARGO_PKG_VERSION"));
    let build_date = env!("VERGEN_BUILD_DATE");
    let git_sha = env!("VERGEN_GIT_SHA");
    println!("   Build:        {} (commit: {})", build_date, git_sha);
    report.record_pass();

    // 2. Directories & Permissions
    println!("\n2. Directories & Storage Access");
    let config_dir = get_config_dir();
    let data_dir = get_data_dir();

    check_directory_access("Config Directory", &config_dir, &mut report);
    check_directory_access("Data Directory", &data_dir, &mut report);

    let backup_dir = data_dir.join("backups");
    if backup_dir.exists() {
        match std::fs::read_dir(&backup_dir) {
            Ok(entries) => {
                let count = entries.filter_map(|e| e.ok()).count();
                println!(
                    "   ✔ {:<22} {} ({} snapshots found)",
                    "Backups Store:",
                    backup_dir.display(),
                    count
                );
                report.record_pass();
            }
            Err(e) => {
                println!(
                    "   ✖ {:<22} {} (Read error: {})",
                    "Backups Store:",
                    backup_dir.display(),
                    e
                );
                report.record_error();
            }
        }
    } else {
        println!(
            "   ● {:<22} {} (Will be created on first sync)",
            "Backups Store:",
            backup_dir.display()
        );
        report.record_pass();
    }

    // 3. Tailery Configuration & State
    println!("\n3. Tailery Configuration & Active Profile");
    let config_path = resolve_config_path(config_path_override);
    if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(content) => match serde_json::from_str::<AppState>(&content) {
                Ok(state) => {
                    println!("   ✔ {:<22} {}", "Config File:", config_path.display());
                    println!(
                        "   ✔ {:<22} '{}'",
                        "Active Profile:", state.settings.active_profile
                    );
                    println!(
                        "   ✔ {:<22} {} configured ({} total in state)",
                        "MCP Servers:",
                        state
                            .profiles
                            .get(&state.settings.active_profile)
                            .map(|p| p.enabled_servers.len())
                            .unwrap_or(0),
                        state.servers.len()
                    );
                    report.record_pass();
                }
                Err(e) => {
                    println!(
                        "   ✖ {:<22} {} (Parse error: {})",
                        "Config File:",
                        config_path.display(),
                        e
                    );
                    report.record_error();
                }
            },
            Err(e) => {
                println!(
                    "   ✖ {:<22} {} (Read error: {})",
                    "Config File:",
                    config_path.display(),
                    e
                );
                report.record_error();
            }
        }
    } else {
        println!(
            "   ● {:<22} {} (Not created yet - run 'tailery' or 'tailery sync' to initialize)",
            "Config File:",
            config_path.display()
        );
        report.record_pass();
    }

    // 4. Client Adapters Health Check
    println!("\n4. AI Coding Client Adapters");
    let adapters = all_adapters();
    for adapter in adapters {
        match adapter.config_path(None) {
            Ok(path) => {
                let name = adapter.display_name();
                if path.exists() {
                    let is_writable = check_file_writable(&path);
                    if is_writable {
                        println!("   ✔ {:<18} Found & Writable -> {}", name, path.display());
                        report.record_pass();
                    } else {
                        println!(
                            "   ⚠ {:<18} Found but NOT Writable (Permission Denied) -> {}",
                            name,
                            path.display()
                        );
                        report.record_warn();
                    }
                } else {
                    println!(
                        "   ● {:<18} Config not present (Client may not be installed) -> {}",
                        name,
                        path.display()
                    );
                    report.record_pass();
                }
            }
            Err(e) => {
                println!(
                    "   ⚠ {:<18} Could not resolve path: {}",
                    adapter.display_name(),
                    e
                );
                report.record_warn();
            }
        }
    }

    // 5. Docker & Container Runtime Health Check
    println!("\n5. Container Runtime & Docker Daemon");
    let start_detect = Instant::now();
    let (docker_mgr, daemon_info) = DockerManager::auto_detect(None).await;
    let detect_duration = start_detect.elapsed();

    if let Some(info) = daemon_info {
        if info.is_running {
            println!(
                "   ✔ {:<22} {} (Engine: {:?})",
                "Runtime Detected:", info.name, info.kind
            );
            println!("   ✔ {:<22} {}", "Socket Path:", info.socket_path);
            println!(
                "   ✔ {:<22} Connected in {:.1}ms",
                "Daemon Status:",
                detect_duration.as_secs_f64() * 1000.0
            );

            // Test ping through docker manager
            if let Some(ref mgr) = docker_mgr {
                if mgr.ping().await {
                    println!("   ✔ {:<22} Ping successful", "API Ping:");
                    report.record_pass();
                } else {
                    println!("   ⚠ {:<22} Ping failed (timeout or error)", "API Ping:");
                    report.record_warn();
                }
            } else {
                report.record_pass();
            }
        } else {
            println!(
                "   ⚠ {:<22} Socket exists at '{}' but daemon is not responding",
                "Daemon Offline:", info.socket_path
            );
            println!(
                "     Suggestion: Start your Docker provider (Docker Desktop, OrbStack, or Colima)."
            );
            report.record_warn();
        }
    } else {
        println!(
            "   ⚠ {:<22} No Docker daemon socket found in standard candidate paths.",
            "Docker Runtime:"
        );
        println!(
            "     Suggestion: If using containerized MCP servers, install Docker Desktop, OrbStack, or Colima."
        );
        report.record_warn();
    }

    // 6. Summary & Recommendations
    println!("\n{}\n", "=".repeat(60));
    println!(
        "Diagnostic Summary: {} passed, {} warning(s), {} error(s)",
        report.passed, report.warnings, report.errors
    );

    if report.errors > 0 {
        println!("⚠ Some critical errors were found. Please review the output above.");
    } else if report.warnings > 0 {
        println!("✔ Tailery is operational with minor warnings.");
    } else {
        println!("✨ Everything looks great! Tailery is fully healthy.");
    }

    Ok(report)
}

fn check_directory_access(label: &str, path: &Path, report: &mut DiagnosticReport) {
    if !path.exists()
        && let Err(e) = std::fs::create_dir_all(path)
    {
        println!(
            "   ✖ {:<22} {} (Cannot create directory: {})",
            format!("{}:", label),
            path.display(),
            e
        );
        report.record_error();
        return;
    }

    let test_file = path.join(".tailery_doctor_probe");
    match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&test_file)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_file);
            println!(
                "   ✔ {:<22} {} (Read/Write OK)",
                format!("{}:", label),
                path.display()
            );
            report.record_pass();
        }
        Err(e) => {
            println!(
                "   ✖ {:<22} {} (Write permission denied: {})",
                format!("{}:", label),
                path.display(),
                e
            );
            report.record_error();
        }
    }
}

fn check_file_writable(path: &Path) -> bool {
    OpenOptions::new().append(true).open(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_doctor_diagnostic_execution() {
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_cfg = temp_dir.path().join("tailery.json");

        let report = run_doctor(Some(&temp_cfg)).await;
        assert!(report.is_ok());
        let r = report.unwrap();
        assert!(r.passed > 0);
    }
}
