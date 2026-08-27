import re

with open("src/adapters/mod.rs", "r") as f:
    content = f.read()

# Add sync_servers to ClientAdapter trait
new_trait_method = """    /// Write active servers to the target config file, creating a safety backup of the full file for the (profile, client) pair and preserving client-specific metadata and unmanaged keys.
    fn write_servers(
        &self,
        profile: &str,
        path: &std::path::Path,
        servers: &std::collections::HashMap<String, crate::state::ServerConfig>,
    ) -> Result<(), AdapterError> {
        if path.exists() {
            let _ = crate::backup::create_backup(profile, self.name(), path);
        }

        let existing_json: Option<serde_json::Value> = if path.exists() {
            let content = std::fs::read_to_string(path).map_err(|source| AdapterError::Io {
                adapter: self.name(),
                source,
            })?;
            Some(parse_json_relaxed(&content))
        } else {
            None
        };

        let merged_val = self.merge_managed_config(path, existing_json.as_ref(), servers)?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| AdapterError::Io {
                adapter: self.name(),
                source,
            })?;
        }

        let formatted =
            serde_json::to_string_pretty(&merged_val).map_err(|source| AdapterError::Serialization {
                adapter: self.name(),
                source,
            })?;

        std::fs::write(path, formatted).map_err(|source| AdapterError::Io {
            adapter: self.name(),
            source,
        })?;

        Ok(())
    }

    /// Synchronizes all ManagedServers to the client's respective configuration tiers (Global, Global-Per-Project, In-Repo).
    fn sync_servers(
        &self,
        profile: &str,
        managed_servers: &std::collections::HashMap<String, crate::state::ManagedServer>,
    ) -> Result<usize, AdapterError> {
        // Default implementation falls back to the old global-only behavior.
        // Adapters should override this to handle all tiers.
        let mut global_servers = std::collections::HashMap::new();
        for (name, srv) in managed_servers {
            if srv.is_global {
                global_servers.insert(name.clone(), srv.config.clone());
            }
        }
        let global_path = self.config_path(None)?;
        self.write_servers(profile, &global_path, &global_servers)?;
        Ok(1)
    }"""

content = re.sub(r'    /// Write active servers.*?\n    \).*?Ok\(\(\)\)\n    }', new_trait_method, content, flags=re.DOTALL)

with open("src/adapters/mod.rs", "w") as f:
    f.write(content)
