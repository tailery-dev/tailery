use std::path::{Path, PathBuf};

/// Recursively scans for directories containing a `.git` folder up to a maximum depth.
pub fn find_projects(base_paths: &[String], max_depth: usize) -> Vec<PathBuf> {
    let mut projects = Vec::new();

    for path_str in base_paths {
        let base_path = expand_tilde(path_str);
        if !base_path.exists() || !base_path.is_dir() {
            continue;
        }
        scan_dir(&base_path, 0, max_depth, &mut projects);
    }

    // Deduplicate
    projects.sort();
    projects.dedup();
    projects
}

fn scan_dir(dir: &Path, current_depth: usize, max_depth: usize, found: &mut Vec<PathBuf>) {
    if current_depth > max_depth {
        return;
    }

    // If it has a .git folder, it's a project! We don't need to go deeper inside the project.
    if dir.join(".git").is_dir() {
        found.push(dir.to_path_buf());
        return;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type()
                && file_type.is_dir()
            {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                // Skip common heavy directories to speed up search
                if name == "node_modules" || name == "target" || name == ".DS_Store" {
                    continue;
                }
                scan_dir(&path, current_depth + 1, max_depth, found);
            }
        }
    }
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if path.starts_with("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(&path[2..]);
    }
    PathBuf::from(path)
}
