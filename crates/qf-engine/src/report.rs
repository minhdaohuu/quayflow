//! Local support report (FR-23): sanitized, saved locally, shared only by the user.

use crate::core::Engine;
use qf_model::time::{now_ms, to_rfc3339};
use serde_json::json;

impl Engine {
    pub fn build_report(&self) -> Result<(String, String), String> {
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
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        let text = crate::journal::scrub(&text, &labels);
        let path = self.dd.reports().join(format!("quayflow-report-{}.json", now_ms()));
        qf_vault::atomic_write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
        let preview: String = text.lines().take(60).collect::<Vec<_>>().join("\n");
        Ok((path.display().to_string(), preview))
    }
}
