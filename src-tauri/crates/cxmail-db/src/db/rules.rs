use crate::error::AppError;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailRule {
    pub id: Option<i64>,
    pub account_id: Option<String>,
    pub name: String,
    pub is_active: bool,
    pub priority: i32,
    pub conditions: Vec<RuleCondition>,
    pub actions: Vec<RuleAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleCondition {
    pub field: String,     // "from", "to", "subject", "body"
    pub operator: String,  // "contains", "equals", "starts_with", "ends_with"
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleAction {
    /// "move" | "mark_read" | "mark_flagged" | "delete" | "set_category"
    pub action_type: String,
    /// Depends on `action_type`:
    /// - `move` → destination folder name
    /// - `set_category` → one of: "primary", "updates", "social", "promotions", "junk"
    /// - `mark_read` / `mark_flagged` / `delete` → ignored
    pub value: Option<String>,
}

pub fn insert(conn: &Connection, rule: &MailRule) -> Result<i64, AppError> {
    let conditions_json = serde_json::to_string(&rule.conditions)
        .map_err(|e| AppError::General(format!("Failed to serialize conditions: {}", e)))?;
    let actions_json = serde_json::to_string(&rule.actions)
        .map_err(|e| AppError::General(format!("Failed to serialize actions: {}", e)))?;

    conn.execute(
        "INSERT INTO mail_rules (account_id, name, is_active, priority, conditions, actions) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![rule.account_id, rule.name, rule.is_active as i32, rule.priority, conditions_json, actions_json],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list(conn: &Connection) -> Result<Vec<MailRule>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, name, is_active, priority, conditions, actions FROM mail_rules ORDER BY priority DESC",
    )?;
    let rules = stmt
        .query_map([], |row| {
            let conditions_str: String = row.get(5)?;
            let actions_str: String = row.get(6)?;
            Ok(MailRule {
                id: row.get(0)?,
                account_id: row.get(1)?,
                name: row.get(2)?,
                is_active: row.get::<_, i32>(3)? != 0,
                priority: row.get(4)?,
                conditions: serde_json::from_str(&conditions_str).unwrap_or_default(),
                actions: serde_json::from_str(&actions_str).unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rules)
}

pub fn update(conn: &Connection, rule: &MailRule) -> Result<(), AppError> {
    let id = rule
        .id
        .ok_or_else(|| AppError::General("Rule id required for update".to_string()))?;
    let conditions_json = serde_json::to_string(&rule.conditions)
        .map_err(|e| AppError::General(format!("Failed to serialize conditions: {}", e)))?;
    let actions_json = serde_json::to_string(&rule.actions)
        .map_err(|e| AppError::General(format!("Failed to serialize actions: {}", e)))?;
    conn.execute(
        "UPDATE mail_rules
         SET account_id = ?1, name = ?2, is_active = ?3, priority = ?4, conditions = ?5, actions = ?6
         WHERE id = ?7",
        params![
            rule.account_id,
            rule.name,
            rule.is_active as i32,
            rule.priority,
            conditions_json,
            actions_json,
            id
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM mail_rules WHERE id = ?1", params![id])?;
    Ok(())
}

/// Evaluate rules against a message. Returns matching actions.
pub fn evaluate(
    rules: &[MailRule],
    from_email: &str,
    to_email: &str,
    subject: &str,
    body: &str,
    account_id: &str,
) -> Vec<RuleAction> {
    let mut actions = Vec::new();
    for rule in rules {
        if !rule.is_active {
            continue;
        }
        // Rule must match the account (or be global)
        if let Some(ref rule_account) = rule.account_id {
            if rule_account != account_id {
                continue;
            }
        }
        let all_match = rule.conditions.iter().all(|cond| {
            let field_value = match cond.field.as_str() {
                "from" => from_email,
                "to" => to_email,
                "subject" => subject,
                "body" => body,
                _ => "",
            };
            let fv = field_value.to_lowercase();
            let cv = cond.value.to_lowercase();
            match cond.operator.as_str() {
                "contains" => fv.contains(&cv),
                "equals" => fv == cv,
                "starts_with" => fv.starts_with(&cv),
                "ends_with" => fv.ends_with(&cv),
                _ => false,
            }
        });
        if all_match {
            actions.extend(rule.actions.clone());
        }
    }
    actions
}
