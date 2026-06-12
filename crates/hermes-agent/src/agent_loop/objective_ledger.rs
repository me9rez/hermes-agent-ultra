use super::*;

impl AgentLoop {
    fn objective_runtime_ledger_path(&self) -> PathBuf {
        let hermes_home = self
            .config()
            .hermes_home
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| std::env::var("HERMES_HOME").ok().map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|h| h.join(".hermes")))
            .unwrap_or_else(|| PathBuf::from(".hermes"));
        hermes_home
            .join("alpha")
            .join("objective_runtime_ledger.jsonl")
    }

    fn objective_eval_trend_path(&self) -> PathBuf {
        let hermes_home = self
            .config()
            .hermes_home
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| std::env::var("HERMES_HOME").ok().map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|h| h.join(".hermes")))
            .unwrap_or_else(|| PathBuf::from(".hermes"));
        hermes_home.join("alpha").join("objective_eval_trend.json")
    }

    fn append_objective_eval_sample(
        &self,
        objective_id: &str,
        objective_state: &str,
        note: &str,
    ) -> Result<(), AgentError> {
        let path = self.objective_eval_trend_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AgentError::Io(format!("create {} failed: {}", parent.display(), e))
            })?;
        }
        let mut root: serde_json::Value = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .unwrap_or_else(
                    || serde_json::json!({"updated_at": Utc::now().to_rfc3339(), "samples": []}),
                )
        } else {
            serde_json::json!({"updated_at": Utc::now().to_rfc3339(), "samples": []})
        };
        let Some(samples) = root.get_mut("samples").and_then(|v| v.as_array_mut()) else {
            root = serde_json::json!({"updated_at": Utc::now().to_rfc3339(), "samples": []});
            let samples = root
                .get_mut("samples")
                .and_then(|v| v.as_array_mut())
                .ok_or_else(|| {
                    AgentError::Config("objective_eval_trend samples field missing".to_string())
                })?;
            samples.push(serde_json::json!({
                "recorded_at": Utc::now().to_rfc3339(),
                "objective_id": objective_id,
                "objective_state": objective_state,
                "score": objective_eval_score(objective_state),
                "note": note,
            }));
            root["updated_at"] = serde_json::json!(Utc::now().to_rfc3339());
            let payload = serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".to_string());
            std::fs::write(&path, payload)
                .map_err(|e| AgentError::Io(format!("write {} failed: {}", path.display(), e)))?;
            return Ok(());
        };
        samples.push(serde_json::json!({
            "recorded_at": Utc::now().to_rfc3339(),
            "objective_id": objective_id,
            "objective_state": objective_state,
            "score": objective_eval_score(objective_state),
            "note": note,
        }));
        if samples.len() > 512 {
            let drain = samples.len().saturating_sub(512);
            samples.drain(0..drain);
        }
        root["updated_at"] = serde_json::json!(Utc::now().to_rfc3339());
        let payload = serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".to_string());
        std::fs::write(&path, payload)
            .map_err(|e| AgentError::Io(format!("write {} failed: {}", path.display(), e)))?;
        Ok(())
    }

    pub(crate) fn append_objective_runtime_ledger(
        &self,
        messages: &[Message],
        assistant_text: &str,
        total_turns: u32,
    ) -> Result<(), AgentError> {
        let Some(objective) = extract_session_objective(messages) else {
            return Ok(());
        };
        if objective.trim().is_empty() {
            return Ok(());
        }
        let objective_id = short_sha256_hex(&format!("objective:{}", objective))
            .chars()
            .take(12)
            .collect::<String>();
        let objective_state = extract_objective_state_marker(assistant_text);
        let evidence_files = extract_marker_values(assistant_text, "path=", 12);
        let evidence_commands = extract_marker_values(assistant_text, "cmd=", 12);
        let decision = if objective_state == "advancing" {
            "promote"
        } else if objective_state == "regressing" {
            "investigate"
        } else if objective_state == "unproven" {
            "collect-more-evidence"
        } else {
            "monitor"
        };
        let entry = serde_json::json!({
            "recorded_at": Utc::now().to_rfc3339(),
            "objective_id": format!("obj-{}", objective_id),
            "objective_state": objective_state,
            "decision": decision,
            "turns": total_turns,
            "evidence_files": evidence_files,
            "evidence_commands": evidence_commands,
        });
        let path = self.objective_runtime_ledger_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AgentError::Io(format!("create {} failed: {}", parent.display(), e))
            })?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| AgentError::Io(format!("open {} failed: {}", path.display(), e)))?;
        writeln!(file, "{}", entry)
            .map_err(|e| AgentError::Io(format!("append {} failed: {}", path.display(), e)))?;
        self.append_objective_eval_sample(
            &format!("obj-{}", objective_id),
            &objective_state,
            &format!("decision={decision} turns={total_turns}"),
        )?;
        Ok(())
    }
}
