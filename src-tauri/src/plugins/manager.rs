use crate::error::AppError;
use crate::plugins::manifest::{self, PluginManifest};
use crate::plugins::runtime;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub permissions: Vec<String>,
    pub triggers: Vec<String>,
    pub enabled: bool,
}

/// Manages plugin lifecycle: discovery, loading, execution.
pub struct PluginManager {
    plugins_dir: PathBuf,
    plugins: Vec<PluginManifest>,
}

impl PluginManager {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self {
            plugins_dir,
            plugins: Vec::new(),
        }
    }

    /// Discover and load all plugins from the plugins directory.
    pub fn discover(&mut self) {
        self.plugins = manifest::discover_plugins(&self.plugins_dir);
        log::info!("Loaded {} plugins", self.plugins.len());
    }

    /// List all loaded plugins.
    pub fn list(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|m| PluginInfo {
                name: m.name.clone(),
                version: m.version.clone(),
                description: m.description.clone(),
                author: m.author.clone(),
                permissions: m.permissions.clone(),
                triggers: m.triggers.clone(),
                enabled: true,
            })
            .collect()
    }

    /// Fire a trigger, executing all plugins that listen for it.
    pub fn fire_trigger(&self, trigger: &str, data: &str) -> Vec<(String, Result<String, AppError>)> {
        let mut results = Vec::new();

        for plugin in &self.plugins {
            if plugin.has_trigger(trigger) {
                log::info!("Firing trigger '{}' for plugin '{}'", trigger, plugin.name);
                let result = runtime::execute_plugin(plugin, trigger, data);
                results.push((plugin.name.clone(), result));
            }
        }

        results
    }

    /// Execute a specific plugin by name with a trigger.
    pub fn execute(&self, name: &str, trigger: &str, data: &str) -> Result<String, AppError> {
        let plugin = self
            .plugins
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| AppError::NotFound(format!("Plugin '{}' not found", name)))?;

        runtime::execute_plugin(plugin, trigger, data)
    }
}
