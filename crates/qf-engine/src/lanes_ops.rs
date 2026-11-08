//! Lanes (FR-10) and Open/Replace with full history (FR-17, FR-18).

use crate::accounts::ApiErr;
use crate::core::{lane_meta, Engine, EnvState, Inner};
use qf_adapter::Env;
use qf_model::api::{HandoffResponse, LaneView};
use qf_model::time::now_ms;
use qf_model::*;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

impl Engine {
    pub fn lane_views(&self) -> Vec<LaneView> {
        let inner = self.lock();
        inner
            .envs
            .iter()
            .filter(|(id, _)| !id.is_default())
            .map(|(id, st)| {
                let links = qf_adapter::lanes::check(&self.default_env, &st.env, false);
                LaneView {
                    name: id.0.clone(),
                    config_dir: st.env.config_dir.display().to_string(),
                    live: st.live.clone(),
                    live_label: st.live.as_ref().map(|a| inner.config.label(a)),
                    links_ok: links.iter().all(|l| l.ok),
                    link_detail: links.iter().map(|l| format!("{}: {}", l.item, l.detail)).collect(),
                    canary_passed: st.lane_canary,
                    sessions: st.sessions.len(),
                }
            })
            .collect()
    }

    pub fn lane_create(&self, name: &str) -> Result<LaneView, ApiErr> {
        let env = qf_adapter::lanes::create(&self.default_env, &self.dd.lanes(), name).map_err(ApiErr::invalid)?;
        {
            let mut inner = self.lock();
            inner.envs.insert(EnvId(name.into()), EnvState::new(env));
            self.record(&inner, "lane.created", json!({"lane": name}), true);
        }
        self.lane_check(name)?;
        self.lane_views()
            .into_iter()
            .find(|l| l.name == name)
            .ok_or_else(|| ApiErr::not_found("lane"))
    }

    /// Repair links and run the lane check: links intact and the governing Claude version
    /// passes the static check. In-place switching in the lane is enabled only after this.
    pub fn lane_check(&self, name: &str) -> Result<LaneView, ApiErr> {
        let id = EnvId(name.into());
        let env = self
            .lock()
            .envs
            .get(&id)
            .map(|s| s.env.clone())
            .ok_or_else(|| ApiErr::not_found("no such lane"))?;
        let links = qf_adapter::lanes::check(&self.default_env, &env, true);
        let links_ok = links.iter().all(|l| l.ok);
        let mut inner = self.lock();
        self.refresh_env(&mut inner, &id);
        let version_ok = inner
            .envs
            .get(&id)
            .and_then(|s| s.in_place.governing_version.clone())
            .or_else(crate::core::installed_claude_version)
            .and_then(|v| inner.compat.get(&v).map(|r| r.in_place_allowed()))
            .unwrap_or(false);
        let passed = links_ok && version_ok;
        let meta = if passed {
            json!({"canaryPassedAt": now_ms()})
        } else {
            json!({})
        };
        let _ = qf_vault::atomic_write(&env.config_dir.join(".quayflow-lane.json"), meta.to_string().as_bytes());
        if let Some(st) = inner.envs.get_mut(&id) {
            st.lane_canary = passed;
        }
        self.refresh_env(&mut inner, &id);
        self.record(
            &inner,
            "lane.check",
            json!({"lane": name, "linksOk": links_ok, "versionOk": version_ok}),
            false,
        );
        drop(inner);
        let _ = lane_meta(&self.dd, name);
        self.lane_views()
            .into_iter()
            .find(|l| l.name == name)
            .ok_or_else(|| ApiErr::not_found("lane"))
    }

    pub fn lane_delete(&self, name: &str) -> Result<String, ApiErr> {
        let id = EnvId(name.into());
        if id.is_default() {
            return Err(ApiErr::refused("the default environment is not a lane"));
        }
        // Lock order everywhere: cred_lock, then inner (switch apply, watcher). The engine
        // lock is not held across the bounded wait in `retire`.
        let _cred = self.cred_lock.lock().unwrap_or_else(|e| e.into_inner());
        let (env, login) = {
            let mut inner = self.lock();
            let st = inner.envs.get(&id).ok_or_else(|| ApiErr::not_found("no such lane"))?;
            if st.blocking_transition() {
                return Err(ApiErr::refused(
                    "a switch is in progress in this lane; try again when it ends",
                ));
            }
            let env = st.env.clone();
            // Keep the newest token of the lane's account before its copy goes away; if that
            // cannot be done, the lane's login is not removed.
            // A login that cannot be read (a Keychain that does not answer) is not "signed out".
            let login = qf_adapter::read_login(&env)
                .map_err(|e| ApiErr::refused(format!("lane not deleted: its login cannot be read ({e})")))?;
            if let Some(l) = &login {
                self.preserve_live(&mut inner, &id, l)
                    .map_err(|e| ApiErr::refused(format!("lane not deleted: {e}")))?;
            }
            // Sessions are checked last, right before the login is removed.
            self.refresh_env(&mut inner, &id);
            if inner.envs.get(&id).is_some_and(|st| !st.sessions.is_empty()) {
                return Err(ApiErr::refused("sessions are running in this lane; end them first"));
            }
            (env, login)
        };
        qf_adapter::login::retire(&env, Duration::from_secs(9)).map_err(|e| ApiErr::refused(e.to_string()))?;
        let mut inner = self.lock();
        // A session may have started in the lane meanwhile: give it its login back.
        self.refresh_env(&mut inner, &id);
        if inner.envs.get(&id).is_some_and(|st| !st.sessions.is_empty()) {
            let restored = match &login {
                Some(l) => self.restore_lane_login(&mut inner, &env, l),
                None => true,
            };
            return Err(ApiErr::refused(if restored {
                "a session started in this lane; its login was put back. End the session first"
            } else {
                "a session started in this lane and its login could not be put back; switch an account into the lane (quayflow use <account> --env <lane>)"
            }));
        }
        qf_adapter::lanes::delete(&env).map_err(|e| ApiErr::invalid(e.to_string()))?;
        inner.envs.remove(&id);
        self.record(&inner, "lane.deleted", json!({"lane": name}), true);
        Ok(format!("lane \"{name}\" deleted; the shared setup is untouched"))
    }

    /// Write back a lane's login removed by an interrupted delete. Returns false on failure.
    fn restore_lane_login(&self, inner: &mut Inner, env: &Env, login: &qf_vault::Login) -> bool {
        match qf_adapter::swap(env, login, Duration::from_secs(3), |_| Ok(())) {
            Ok(out) => {
                if let Some(st) = inner.envs.get_mut(&env.id) {
                    st.own_stamp = out.stamp;
                    st.last_stamp = out.stamp;
                }
                self.record(inner, "lane.login_restored", json!({"lane": env.id}), true);
                true
            }
            Err(e) => {
                self.log.log(
                    "error",
                    "lane.restore_failed",
                    json!({"lane": env.id, "error": e.to_string()}),
                    &[],
                );
                false
            }
        }
    }

    fn handoff_envs(&self, session_id: &str, to_env: &str) -> Result<(Env, Env, Option<SessionInfo>), ApiErr> {
        if !qf_adapter::history::valid_session_id(session_id) {
            return Err(ApiErr::invalid("a session id is required"));
        }
        let mut inner = self.lock();
        let ids: Vec<EnvId> = inner.envs.keys().cloned().collect();
        for id in &ids {
            self.refresh_env(&mut inner, id);
        }
        let to_id = if to_env.is_empty() {
            self.default_env.id.clone()
        } else {
            EnvId(to_env.into())
        };
        let to = inner
            .envs
            .get(&to_id)
            .map(|s| s.env.clone())
            .ok_or_else(|| ApiErr::not_found(format!("no environment \"{to_id}\"")))?;
        if inner
            .envs
            .get(&to_id)
            .is_some_and(|s| s.live.is_none() || s.unknown.is_some())
        {
            return Err(ApiErr::refused(format!(
                "environment \"{to_id}\" has no registered login; switch an account into it first"
            )));
        }
        let source = inner.envs.values().find_map(|st| {
            st.sessions
                .iter()
                .find(|s| s.session_id == session_id)
                .map(|s| (st.env.clone(), s.clone()))
        });
        let (from, session) = match source {
            Some((e, s)) => (e, Some(s)),
            None => {
                let found = inner
                    .envs
                    .values()
                    .find(|st| qf_adapter::history::find_project(&st.env, session_id).is_some())
                    .map(|st| st.env.clone())
                    .ok_or_else(|| ApiErr::not_found("session history not found"))?;
                (found, None)
            }
        };
        if from.id == to.id {
            return Err(ApiErr::invalid("the session already runs in that environment"));
        }
        Ok((from, to, session))
    }

    fn prepare_history(
        &self,
        from: &Env,
        to: &Env,
        session_id: &str,
    ) -> Result<qf_adapter::history::CopyResult, ApiErr> {
        let inner = self.lock();
        self.record(
            &inner,
            "handoff.preparing",
            json!({"session": session_id, "from": from.id, "to": to.id}),
            true,
        );
        drop(inner);
        let r = qf_adapter::history::copy_session(from, to, session_id, &self.dd.handoff_backups());
        let inner = self.lock();
        match r {
            Ok(r) => {
                if !to.id.is_default() {
                    let _ = qf_adapter::lanes::link_memory(&self.default_env, to, &r.project);
                }
                self.record(
                    &inner,
                    "handoff.copied",
                    json!({"session": session_id, "files": r.file_count, "rows": r.transcript_rows, "backups": r.backups.len()}),
                    true,
                );
                Ok(r)
            }
            Err(e) => {
                self.record(
                    &inner,
                    "handoff.failed",
                    json!({"session": session_id, "error": e.to_string()}),
                    true,
                );
                Err(ApiErr::refused(format!(
                    "{e}. The source session and its history are unchanged."
                )))
            }
        }
    }

    pub fn resume_command(to: &Env, session_id: &str, cwd: Option<&str>) -> String {
        if !cfg!(windows) {
            // POSIX shells: one line for bash or zsh.
            let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
            let mut parts = vec![];
            if let Some(c) = cwd {
                parts.push(format!("cd {}", q(c)));
            }
            parts.push(match to.selector() {
                Some(dir) => format!(
                    "CLAUDE_CONFIG_DIR={} claude --resume {session_id}",
                    q(&dir.display().to_string())
                ),
                None => format!("env -u CLAUDE_CONFIG_DIR claude --resume {session_id}"),
            });
            return parts.join(" && ");
        }
        let mut parts = vec![];
        if let Some(c) = cwd {
            parts.push(format!("Set-Location -LiteralPath '{}'", c.replace('\'', "''")));
        }
        match to.selector() {
            Some(dir) => parts.push(format!(
                "$env:CLAUDE_CONFIG_DIR='{}'",
                dir.display().to_string().replace('\'', "''")
            )),
            None => parts.push("Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue".into()),
        }
        parts.push(format!("claude --resume {session_id}"));
        parts.join("; ")
    }

    pub fn handoff(&self, action: &str, session_id: &str, to_env: &str) -> Result<HandoffResponse, ApiErr> {
        let (from, to, session) = self.handoff_envs(session_id, to_env)?;
        let r = self.prepare_history(&from, &to, session_id)?;
        let cwd = session.as_ref().and_then(|s| s.cwd.clone());
        let command = Engine::resume_command(&to, session_id, cwd.as_deref());
        let mut resp = HandoffResponse {
            ok: true,
            file_count: r.file_count,
            command: command.clone(),
            launched: false,
            message: None,
        };
        match action {
            "prepare" => {
                resp.message = Some("History copied. Run the command after this session ends.".into());
            }
            "open" => {
                // One executor: the new window waits for the source process to end first.
                let exe = std::env::current_exe().map_err(|e| ApiErr::invalid(e.to_string()))?;
                let mut args = vec![
                    "handoff".to_string(),
                    "resume".into(),
                    "--session".into(),
                    session_id.into(),
                    "--to".into(),
                    to.id.0.clone(),
                ];
                if let Some(s) = &session {
                    args.push("--wait-pid".into());
                    args.push(s.pid.to_string());
                }
                if let Some(c) = &cwd {
                    args.push("--cwd".into());
                    args.push(c.clone());
                }
                match qf_adapter::process::spawn_new_console(&exe, &args, &[], cwd.as_deref().map(Path::new)) {
                    Ok(_) => {
                        resp.launched = true;
                        resp.message = Some(if session.is_some() {
                            "A new window opened. It continues this conversation as soon as this session ends (/exit)."
                                .into()
                        } else {
                            "A new window opened with the conversation.".into()
                        });
                    }
                    Err(e) => {
                        resp.message = Some(format!(
                            "No window could be opened ({e}); run the command after this session ends."
                        ))
                    }
                }
            }
            "replace" => {
                let s = session
                    .as_ref()
                    .ok_or_else(|| ApiErr::refused("the session is not running; use Open New Session"))?;
                let Some((shell_pid, shell)) = qf_adapter::process::powershell_ancestor(s.pid) else {
                    return Err(ApiErr::refused(
                        "same-terminal replacement needs PowerShell 7 or Windows PowerShell 5.1 with the Quayflow prompt hook; use Open New Session",
                    ));
                };
                let claim = json!({
                    "sessionId": session_id,
                    "configDir": to.selector().map(|p| p.display().to_string()),
                    "cwd": cwd,
                    "sourcePid": s.pid,
                    "createdAt": now_ms(),
                    "expiresAt": now_ms() + 15 * 60_000,
                });
                let path = self.dd.claims().join(format!("{shell_pid}.json"));
                qf_vault::atomic_write(&path, claim.to_string().as_bytes())
                    .map_err(|e| ApiErr::invalid(e.to_string()))?;
                let hook = self.lock().config.consents.prompt_hook.is_some();
                resp.message = Some(format!(
                    "Ready. End this session (/exit); this {shell} terminal then continues the conversation under \"{}\".{}",
                    to.id,
                    if hook { "" } else { " The prompt hook is not installed (quayflow shell install); without it, run the command shown." }
                ));
            }
            _ => return Err(ApiErr::not_found("unknown handoff action")),
        }
        let inner = self.lock();
        self.record(
            &inner,
            "handoff.ready",
            json!({"session": session_id, "action": action, "to": to.id}),
            true,
        );
        Ok(resp)
    }
}
