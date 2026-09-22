use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub permissions: Vec<String>,
    pub triggers: Vec<String>,
    pub main: String,
    #[serde(skip)]
    pub path: PathBuf,
}

/// Known permission names that plugins can request.
pub const VALID_PERMISSIONS: &[&str] = &[
    "read_emails",
    "search_emails",
    "flag_email",
    "move_email",
    "compose_draft",
    "send_email",
    "delete_email",
    "notify",
];

/// Known trigger names.
pub const VALID_TRIGGERS: &[&str] = &[
    "on_new_email",
    "on_send",
    "on_folder_change",
    "on_startup",
    "on_schedule",
];

impl PluginManifest {
    /// Load a manifest from a plugin directory.
    pub fn load(plugin_dir: &Path) -> Result<Self, String> {
        let manifest_path = plugin_dir.join("plugin.json");
        if !manifest_path.exists() {
            return Err(format!("No plugin.json found in {:?}", plugin_dir));
        }

        let content = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("Failed to read plugin.json: {}", e))?;

        let mut manifest: PluginManifest = serde_json::from_str(&content)
            .map_err(|e| format!("Invalid plugin.json: {}", e))?;

        validate_known_values("permission", &manifest.permissions, VALID_PERMISSIONS)?;
        validate_known_values("trigger", &manifest.triggers, VALID_TRIGGERS)?;
        manifest.path = plugin_dir.to_path_buf();
        Ok(manifest)
    }

    /// Path to the main script file.
    pub fn main_script(&self) -> PathBuf {
        self.path.join(&self.main)
    }

    /// Check if this plugin has a specific permission.
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }

    /// Check if this plugin listens for a specific trigger.
    pub fn has_trigger(&self, trigger: &str) -> bool {
        self.triggers.iter().any(|t| t == trigger)
    }
}

/// Discover all plugins in the plugins directory.
pub fn discover_plugins(plugins_dir: &Path) -> Vec<PluginManifest> {
    let mut plugins = Vec::new();

    if !plugins_dir.exists() {
        return plugins;
    }

    if let Ok(entries) = std::fs::read_dir(plugins_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                match PluginManifest::load(&path) {
                    Ok(manifest) => {
                        log::info!("Discovered plugin: {} v{}", manifest.name, manifest.version);
                        plugins.push(manifest);
                    }
                    Err(e) => {
                        log::warn!("Skipping {:?}: {}", path, e);
                    }
                }
            }
        }
    }

    plugins
}

fn validate_known_values(kind: &str, values: &[String], valid: &[&str]) -> Result<(), String> {
    if let Some(invalid) = values.iter().find(|value| !valid.contains(&value.as_str())) {
        return Err(format!("Unknown {kind} '{invalid}'"));
    }
    Ok(())
}
