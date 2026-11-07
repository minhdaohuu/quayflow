//! Local support report (FR-23): sanitized, reviewed, saved locally, shared only by the user.

use crate::core::Engine;
use qf_model::api::ReportPreview;
use qf_model::time::{now_ms, to_rfc3339};
use serde_json::json;

impl Engine {
    /// The sanitized report text, built now and not saved.
    pub fn report_text(&self) -> Result<String, String> {
        let state = self.state_view(None, None);
        let inner = self.lock();
        let labels = Engine::labels(&inner);
        let cfg = &inner.config;
        let report = json!({
            "generatedAt": to_rfc3339(now_ms()),
            "engine": {"version": state.engine.version, "protocol": state.engine.protocol, "integrity": state.engine.integrity, "sha256": state.engine.exe_sha256},
            "accounts": cfg.accounts.iter().map(|a| json!({"id": a.id, "kind": a.kind, "subscription": a.subscription, "health": a.health, "unavailableModels": a.unavailable_models})).collect::<Vec<_>>(),
            "groups": cfg.groups.iter().map(|g| json!({"name": g.name, "members": g.accounts, "mode": g.mode, "threshold": g.threshold, "margin": g.margin, "preferred": g.preferred, "mixedApproved": g.mixed_approval.is_some()})).collect::<Vec<_>>(),
            "projectAssignments": cfg.projects.len(),
            "tuning": cfg.tuning,
            "consents": cfg.consents,
            "dOrg": cfg.d_org.enabled,
            "envs": state.envs.iter().map(|e| json!({
                "id": e.id, "live": e.live.account, "known": e.live.known, "mode": e.mode, "inPlace": e.in_place,
                "managed": e.managed, "sessions": e.sessions.len(), "decision": crate::journal::scrub(&e.decision, &labels),
                "paused": e.paused_reason.as_ref().map(|p| crate::journal::scrub(p, &labels)),
                "transition": e.transition.as_ref().map(|t| json!({"state": t.state, "how": t.how, "from": t.from, "to": t.to}))
            })).collect::<Vec<_>>(),
            "compat": state.compat,
            "observations": inner.observations.iter().map(|o| json!({"account": o.account, "ageS": (now_ms() - o.ts) / 1000, "source": o.source, "windows": o.windows})).collect::<Vec<_>>(),
            "journalTail": self.journal.tail(200),
            "logTail": self.log.tail(200),
        });
        // Field by field, so fixed-vocabulary fields ("kind": "personal") are never mistaken for
        // an account label of the same text.
        let mut report = report;
        crate::journal::scrub_value(&mut report, &labels);
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
    }

    fn save_report_text(&self, text: &str) -> Result<String, String> {
        let path = self.dd.reports().join(format!("quayflow-report-{}.json", now_ms()));
        qf_vault::atomic_write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
        Ok(path.display().to_string())
    }

    /// Build and save at once (`quayflow report`, `POST /v1/report`).
    pub fn build_report(&self) -> Result<(String, String), String> {
        let text = self.report_text()?;
        let path = self.save_report_text(&text)?;
        let preview: String = text.lines().take(60).collect::<Vec<_>>().join("\n");
        Ok((path, preview))
    }

    /// Build a report for review without saving it; a later [`Engine::save_reviewed_report`]
    /// with this id saves exactly this text.
    pub fn preview_report(&self) -> Result<ReportPreview, String> {
        let text = self.report_text()?;
        let id = crate::core::random_hex(8);
        self.lock().pending_report = Some((id.clone(), text.clone()));
        Ok(ReportPreview { id, text })
    }

    /// Save the reviewed report `id`. Refused when it is not the report last previewed.
    pub fn save_reviewed_report(&self, id: &str) -> Result<(String, String), String> {
        let text = {
            let inner = self.lock();
            match &inner.pending_report {
                Some((pid, text)) if pid == id && !id.is_empty() => text.clone(),
                _ => return Err("that report preview is no longer current; preview it again".into()),
            }
        };
        let path = self.save_report_text(&text)?;
        self.lock().pending_report = None;
        let preview: String = text.lines().take(60).collect::<Vec<_>>().join("\n");
        Ok((path, preview))
    }
}
