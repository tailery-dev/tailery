use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::config::get_data_dir;

pub const MAX_BACKUPS_PER_PAIR: usize = 10;
pub const MAX_BACKUPS_PER_CLIENT: usize = MAX_BACKUPS_PER_PAIR;

#[derive(Error, Debug)]
pub enum BackupError {
    #[error("I/O error during backup operation: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error during backup operation: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("No backups found for profile '{0}', client '{1}'")]
    NoBackupsFound(String, String),
    #[error("Backup '{0}' was not found")]
    NotFound(String),
    #[error("Target path resolution error: {0}")]
    TargetResolution(String),
}

fn default_backup_profile() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEntry {
    pub id: String,
    #[serde(default = "default_backup_profile")]
    pub profile: String,
    pub client: String,
    pub filename: String,
    pub original_path: PathBuf,
    pub backup_path: PathBuf,
    pub timestamp: String,
    pub size_bytes: u64,
}

/// Returns the root directory where Tailery stores all client backups: `<data_dir>/backups`.
pub fn backup_dir() -> PathBuf {
    get_data_dir().join("backups")
}

/// Returns the specific backup directory for a given profile-client pair: `$XDG_DATA_HOME/tailery/backups/<profile>/<client>`.
pub fn profile_client_backup_dir(profile: &str, client: &str) -> PathBuf {
    let p = profile.to_lowercase().trim().to_string();
    let c = client.to_lowercase().trim().to_string();
    backup_dir()
        .join(if p.is_empty() { "default" } else { &p })
        .join(if c.is_empty() { "unknown" } else { &c })
}

/// Legacy client directory (for fallback resolution).
pub fn client_backup_dir(client: &str) -> PathBuf {
    backup_dir().join(client.to_lowercase())
}

/// Creates a timestamped backup of the specified client configuration file, scoped per profile-client pair, enforcing the 10-backup limit.
pub fn create_backup(
    profile: &str,
    client: &str,
    file_path: &Path,
) -> Result<Option<BackupEntry>, BackupError> {
    if !file_path.exists() || !file_path.is_file() {
        return Ok(None);
    }

    let target_dir = profile_client_backup_dir(profile, client);
    fs::create_dir_all(&target_dir)?;

    let now = Utc::now();
    let id = format!("{}_{}", now.format("%Y%m%d_%H%M%S_%6f"), std::process::id());
    let orig_filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.json");

    let backup_filename = format!("{}_{}", id, orig_filename);
    let backup_path = target_dir.join(&backup_filename);
    let meta_path = target_dir.join(format!("{}.meta.json", backup_filename));

    // Copy original file content into backup storage
    let content = fs::read(file_path)?;
    fs::write(&backup_path, &content)?;

    let size_bytes = fs::metadata(&backup_path)?.len();
    let entry = BackupEntry {
        id,
        profile: if profile.trim().is_empty() {
            "default".to_string()
        } else {
            profile.to_lowercase().trim().to_string()
        },
        client: client.to_lowercase().trim().to_string(),
        filename: backup_filename,
        original_path: file_path.to_path_buf(),
        backup_path,
        timestamp: now.to_rfc3339(),
        size_bytes,
    };

    // Save metadata sidecar
    let meta_json = serde_json::to_string_pretty(&entry)?;
    fs::write(&meta_path, meta_json)?;

    // Enforce 10-backup FIFO retention per profile-client pair
    let _ = rotate_backups(profile, client, MAX_BACKUPS_PER_PAIR)?;

    Ok(Some(entry))
}

/// Enforces the maximum backup limit for a profile-client pair, removing oldest backups when exceeded.
pub fn rotate_backups(
    profile: &str,
    client: &str,
    max_backups: usize,
) -> Result<usize, BackupError> {
    let backups = list_backups(Some(profile), Some(client))?;
    if backups.len() <= max_backups {
        return Ok(0);
    }

    let mut deleted = 0;
    for old_entry in backups.iter().skip(max_backups) {
        if old_entry.backup_path.exists() {
            let _ = fs::remove_file(&old_entry.backup_path);
        }
        let meta_path = old_entry
            .backup_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{}.meta.json", old_entry.filename));
        if meta_path.exists() {
            let _ = fs::remove_file(&meta_path);
        }
        deleted += 1;
    }

    Ok(deleted)
}

/// Lists all available backups, optionally filtered by profile name and/or client name, sorted newest to oldest.
pub fn list_backups(
    profile_filter: Option<&str>,
    client_filter: Option<&str>,
) -> Result<Vec<BackupEntry>, BackupError> {
    let base_dir = backup_dir();
    if !base_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();

    fn scan_dir(dir: &Path, entries: &mut Vec<BackupEntry>) {
        if let Ok(read_dir) = fs::read_dir(dir) {
            for entry in read_dir.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    scan_dir(&path, entries);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && path.to_string_lossy().ends_with(".meta.json")
                {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(parsed) = serde_json::from_str::<BackupEntry>(&content) {
                            if parsed.backup_path.exists() {
                                entries.push(parsed);
                            }
                        }
                    }
                }
            }
        }
    }

    scan_dir(&base_dir, &mut entries);

    let prof_norm = profile_filter.map(|p| p.to_lowercase().trim().to_string());
    let cli_norm = client_filter.map(|c| c.to_lowercase().trim().to_string());

    entries.retain(|e| {
        if let Some(ref p) = prof_norm {
            if e.profile.to_lowercase() != *p {
                return false;
            }
        }
        if let Some(ref c) = cli_norm {
            let e_c = e.client.to_lowercase();
            if e_c != *c {
                let matches_alias = match c.as_str() {
                    "claude" | "claude_code" | "claude-code" => {
                        e_c == "claude" || e_c == "claude_code" || e_c == "claude-code"
                    }
                    "antigravity" | "agy" | "google_antigravity" | "google-antigravity" => {
                        e_c == "antigravity"
                            || e_c == "agy"
                            || e_c == "google_antigravity"
                            || e_c == "google-antigravity"
                    }
                    _ => false,
                };
                if !matches_alias {
                    return false;
                }
            }
        }
        true
    });

    // Sort by timestamp descending (newest first), tie-breaking by ID
    entries.sort_by(|a, b| (&b.timestamp, &b.id).cmp(&(&a.timestamp, &a.id)));
    Ok(entries)
}

/// Restores a backup entry to a target destination (or to its recorded original path).
pub fn restore_backup_entry(
    entry: &BackupEntry,
    target_path: Option<&Path>,
) -> Result<PathBuf, BackupError> {
    let dest = target_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| entry.original_path.clone());

    if !entry.backup_path.exists() {
        return Err(BackupError::NotFound(
            entry.backup_path.to_string_lossy().to_string(),
        ));
    }

    let content = fs::read(&entry.backup_path)?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&dest, content)?;
    Ok(dest)
}

/// Quickly restores the most recent backup for a given profile-client pair.
pub fn restore_latest(
    profile: &str,
    client: &str,
    target_path: Option<&Path>,
) -> Result<BackupEntry, BackupError> {
    let backups = list_backups(Some(profile), Some(client))?;
    let latest = backups
        .into_iter()
        .next()
        .ok_or_else(|| BackupError::NoBackupsFound(profile.to_string(), client.to_string()))?;

    restore_backup_entry(&latest, target_path)?;
    Ok(latest)
}

/// Finds and restores a backup matching a specific ID, filename, or client name within a profile context.
pub fn restore_by_id_or_client(
    query: &str,
    profile_hint: Option<&str>,
    target_path: Option<&Path>,
) -> Result<BackupEntry, BackupError> {
    let all = list_backups(profile_hint, None)?;

    // 1. Try exact ID match
    if let Some(entry) = all.iter().find(|e| e.id == query) {
        restore_backup_entry(entry, target_path)?;
        return Ok(entry.clone());
    }

    // 2. Try exact filename match
    if let Some(entry) = all.iter().find(|e| e.filename == query) {
        restore_backup_entry(entry, target_path)?;
        return Ok(entry.clone());
    }

    // 3. Try client name match (restore latest for that client)
    let client_query = query.to_lowercase();
    if let Some(entry) = all.iter().find(|e| {
        let ec = e.client.to_lowercase();
        ec == client_query
            || match client_query.as_str() {
                "claude" | "claude_code" | "claude-code" => {
                    ec == "claude" || ec == "claude_code" || ec == "claude-code"
                }
                "antigravity" | "agy" | "google_antigravity" | "google-antigravity" => {
                    ec == "antigravity"
                        || ec == "agy"
                        || ec == "google_antigravity"
                        || ec == "google-antigravity"
                }
                _ => false,
            }
    }) {
        restore_backup_entry(entry, target_path)?;
        return Ok(entry.clone());
    }

    // If profile hint was provided and no match found, fallback to search across all profiles
    if profile_hint.is_some() {
        return restore_by_id_or_client(query, None, target_path);
    }

    Err(BackupError::NotFound(query.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_list_backups_per_profile_client() {
        let profile = "work_create_test";
        let client = "test_antigravity_unique_create";
        let dir = profile_client_backup_dir(profile, client);
        let _ = fs::remove_dir_all(&dir);

        let temp_dir = std::env::temp_dir().join("tailery_test_backup_create_pair");
        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::create_dir_all(&temp_dir);
        let sample_file = temp_dir.join("mcp_config.json");
        fs::write(
            &sample_file,
            r#"{"mcpServers": {"test": {"command": "echo"}}}"#,
        )
        .unwrap();

        let entry = create_backup(profile, client, &sample_file).unwrap();
        assert!(entry.is_some());
        let entry = entry.unwrap();

        assert_eq!(entry.profile, profile);
        assert_eq!(entry.client, client);
        assert!(entry.backup_path.exists());

        let list = list_backups(Some(profile), Some(client)).unwrap();
        assert!(!list.is_empty());
        assert_eq!(list[0].id, entry.id);
        assert_eq!(list[0].profile, profile);

        // Verify other profile doesn't see it when filtered
        let other_list = list_backups(Some("personal_unique_check"), Some(client)).unwrap();
        assert!(other_list.is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_rolling_10_backups_retention_per_profile_client_pair() {
        let profile = "audit_rot_test";
        let client = "test_rotation_client_unique_rot";
        let dir = profile_client_backup_dir(profile, client);
        let _ = fs::remove_dir_all(&dir);

        let temp_dir = std::env::temp_dir().join("tailery_test_backup_rotation_pair");
        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::create_dir_all(&temp_dir);
        let sample_file = temp_dir.join("mcp.json");

        // Create 15 backups sequentially for (audit_rot_test, test_rotation_client_unique_rot)
        for i in 1..=15 {
            fs::write(
                &sample_file,
                format!(r#"{{"mcpServers": {{"test_{}": {{}}}}}}"#, i),
            )
            .unwrap();
            let _ = create_backup(profile, client, &sample_file).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let backups = list_backups(Some(profile), Some(client)).unwrap();
        assert_eq!(backups.len(), MAX_BACKUPS_PER_PAIR);

        // Verify that the newest backup has the latest content (test_15)
        let latest_content = fs::read_to_string(&backups[0].backup_path).unwrap();
        assert!(latest_content.contains("test_15"));

        // Verify that the oldest kept backup is test_6 (since 1..5 were pruned)
        let oldest_content = fs::read_to_string(&backups.last().unwrap().backup_path).unwrap();
        assert!(oldest_content.contains("test_6"));

        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_restore_backup_roundtrip_per_profile() {
        let profile = "dev_restore_test";
        let client = "test_restore_client_unique_res";
        let dir = profile_client_backup_dir(profile, client);
        let _ = fs::remove_dir_all(&dir);

        let temp_dir = std::env::temp_dir().join("tailery_test_backup_restore_pair");
        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::create_dir_all(&temp_dir);
        let sample_file = temp_dir.join("settings.json");
        let original_content =
            r#"{"context_servers": {"important_server": {"command": "python"}}}"#;
        fs::write(&sample_file, original_content).unwrap();

        let backup_entry = create_backup(profile, client, &sample_file)
            .unwrap()
            .unwrap();

        // Mutate original file
        fs::write(&sample_file, r#"{"context_servers": {}}"#).unwrap();
        assert_ne!(fs::read_to_string(&sample_file).unwrap(), original_content);

        // Restore latest backup
        let restored = restore_latest(profile, client, Some(&sample_file)).unwrap();
        assert_eq!(restored.id, backup_entry.id);

        let restored_content = fs::read_to_string(&sample_file).unwrap();
        assert_eq!(restored_content, original_content);

        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_restore_by_id_or_client_per_profile() {
        let profile = "staging_query_test";
        let client = "test_query_client_unique_q";
        let dir = profile_client_backup_dir(profile, client);
        let _ = fs::remove_dir_all(&dir);

        let temp_dir = std::env::temp_dir().join("tailery_test_backup_query_pair");
        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::create_dir_all(&temp_dir);
        let sample_file = temp_dir.join("mcp.json");
        fs::write(&sample_file, r#"{"test": "query"}"#).unwrap();

        let entry = create_backup(profile, client, &sample_file)
            .unwrap()
            .unwrap();

        // Test restore by ID
        let restored_by_id =
            restore_by_id_or_client(&entry.id, Some(profile), Some(&sample_file)).unwrap();
        assert_eq!(restored_by_id.id, entry.id);

        // Test restore by client
        let restored_by_client =
            restore_by_id_or_client(client, Some(profile), Some(&sample_file)).unwrap();
        assert_eq!(restored_by_client.id, entry.id);

        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::remove_dir_all(&dir);
    }
}
