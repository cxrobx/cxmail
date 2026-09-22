use crate::error::AppError;
use crate::plugins::manifest::PluginManifest;
use rquickjs::{Context, Runtime};

/// Execute a plugin script in a sandboxed QuickJS runtime.
pub fn execute_plugin(
    manifest: &PluginManifest,
    trigger: &str,
    trigger_data: &str,
) -> Result<String, AppError> {
    let script_path = manifest.main_script();
    let script = std::fs::read_to_string(&script_path)
        .map_err(|e| AppError::General(format!("Failed to read plugin script {:?}: {}", script_path, e)))?;

    let rt = Runtime::new()
        .map_err(|e| AppError::General(format!("QuickJS runtime creation failed: {}", e)))?;

    // Memory limit: 16MB per plugin
    rt.set_memory_limit(16 * 1024 * 1024);
    // Max stack size: 1MB
    rt.set_max_stack_size(1024 * 1024);

    let ctx = Context::full(&rt)
        .map_err(|e| AppError::General(format!("QuickJS context creation failed: {}", e)))?;

    let result = ctx.with(|ctx| -> Result<String, AppError> {
        // Inject cxmail API stub (permissions checked at this level)
        let permissions: Vec<String> = manifest.permissions.clone();

        // Inject a minimal cxmail object with log and notify
        ctx.eval::<(), _>(r#"
            var cxmail = {
                _results: [],
                _permissions: [],
                log: function(msg) { cxmail._results.push("LOG:" + msg); },
                notify: function(title, body) {
                    if (cxmail._permissions.indexOf("notify") >= 0) {
                        cxmail._results.push("NOTIFY:" + title + "|" + body);
                    } else {
                        cxmail._results.push("LOG:Permission denied: notify");
                    }
                },
                flagEmail: function(uid, flag) {
                    if (cxmail._permissions.indexOf("flag_email") >= 0) {
                        cxmail._results.push("FLAG:" + uid + "|" + flag);
                    } else {
                        cxmail._results.push("LOG:Permission denied: flag_email");
                    }
                },
                moveEmail: function(uid, from, to) {
                    if (cxmail._permissions.indexOf("move_email") >= 0) {
                        cxmail._results.push("MOVE:" + uid + "|" + from + "|" + to);
                    } else {
                        cxmail._results.push("LOG:Permission denied: move_email");
                    }
                },
                categorize: function(uid, accountId, category) {
                    cxmail._results.push("CATEGORIZE:" + uid + "|" + accountId + "|" + category);
                },
            };
        "#).map_err(|e| AppError::General(format!("API injection failed: {}", e)))?;

        // Set permissions array
        let perms_json = serde_json::to_string(&permissions).unwrap_or("[]".to_string());
        ctx.eval::<(), _>(format!("cxmail._permissions = {};", perms_json))
            .map_err(|e| AppError::General(format!("Permission injection failed: {}", e)))?;

        // Set trigger data
        let trigger_json = serde_json::to_string(trigger)
            .map_err(|e| AppError::General(format!("Trigger serialization failed: {}", e)))?;
        let trigger_data_json = serde_json::to_string(trigger_data)
            .map_err(|e| AppError::General(format!("Trigger data serialization failed: {}", e)))?;
        ctx.eval::<(), _>(format!(
            "var __trigger = {trigger_json}; var __triggerDataRaw = {trigger_data_json}; var __triggerData = (() => {{ try {{ return JSON.parse(__triggerDataRaw); }} catch (_err) {{ return __triggerDataRaw; }} }})();",
        )).map_err(|e| AppError::General(format!("Trigger data injection failed: {}", e)))?;

        // Execute the plugin script
        ctx.eval::<(), _>(script.as_str())
            .map_err(|e| AppError::General(format!("Plugin execution failed: {}", e)))?;

        // Collect results
        let results: String = ctx.eval("JSON.stringify(cxmail._results)")
            .map_err(|e| AppError::General(format!("Result collection failed: {}", e)))?;

        Ok(results)
    })?;

    Ok(result)
}
