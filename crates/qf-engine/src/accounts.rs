//! Onboarding, account lifecycle (FR-01..FR-04), groups and projects (FR-05, FR-06),
//! managed-login settings (FR-06a).

use crate::core::{random_hex, Engine, Inner, Signin};
use qf_model::api::{GroupChangeResponse, SigninIdentity, SigninStatus};
use qf_model::time::now_ms;
use qf_model::*;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ApiErr {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl ApiErr {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        ApiErr {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn invalid(m: impl Into<String>) -> Self {
        ApiErr::new(400, "invalid", m)
    }
    pub fn refused(m: impl Into<String>) -> Self {
        ApiErr::new(409, "refused", m)
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        ApiErr::new(404, "not_found", m)
    }
}

pub fn valid_label(l: &str) -> Result<String, ApiErr> {
    let l = l.trim();
    if l.is_empty() || l.chars().count() > 40 || l.chars().any(|c| c.is_control()) {
        return Err(ApiErr::invalid("labels are 1-40 printable characters"));
    }
    Ok(l.to_string())
}

pub const DISCLOSURE_TAIL: &str = "Switching between accounts of this group needs no further confirmation. Organization rules on where its work may be processed remain your responsibility.";

/// One-time mixed-identity disclosure text for a membership set.
pub fn disclosure(cfg: &Config, group: &str, members: &[AccountId]) -> String {
    let list: Vec<String> = members
        .iter()
        .filter_map(|id| cfg.account(id))
        .map(|a| {
            format!(
                "{} ({})",
                a.label,
                if a.kind == AccountKind::Organization {
                    "organization"
                } else {
                    "personal"
                }
            )
        })
        .collect();
    format!(
        "Work in projects that use group \"{group}\" may be sent to any of these accounts: {}. {DISCLOSURE_TAIL}",
        list.join(", ")
    )
}

impl Engine {
    pub fn find_account(&self, inner: &Inner, key: &str) -> Result<AccountId, ApiErr> {
        inner
            .config
            .find_account(key)
            .map(|a| a.id.clone())
            .ok_or_else(|| ApiErr::not_found(format!("no registered account \"{key}\"")))
    }

    pub fn onboarding(&self, consents: &Value, complete: bool) -> Result<(), ApiErr> {
        let mut inner = self.lock();
        let now = now_ms();
        let set = |v: Option<&Value>, slot: &mut Option<i64>| match v.and_then(|x| x.as_bool()) {
            Some(true) => {
                if slot.is_none() {
                    *slot = Some(now);
                }
            }
            Some(false) => *slot = None,
            None => {}
        };
        let c = &mut inner.config.consents;
        set(consents.get("engineStart"), &mut c.engine_start);
        set(consents.get("signinStartup"), &mut c.signin_startup);
        set(consents.get("credentialStorage"), &mut c.credential_storage);
        set(consents.get("terms"), &mut c.terms);
        set(consents.get("promptHook"), &mut c.prompt_hook);
        set(consents.get("liveCanary"), &mut c.live_canary);
        if complete {
            if inner.config.consents.terms.is_none() {
                return Err(ApiErr::refused(
                    "the terms and organization-rules notice must be acknowledged first",
                ));
            }
            inner.config.onboarded_at.get_or_insert(now);
        }
        self.save_config(&inner).map_err(ApiErr::invalid)?;
        self.record(
            &inner,
            "consents",
            json!({"consents": inner.config.consents, "complete": complete}),
            false,
        );
        Ok(())
    }

    /// Apply a membership change; returns the disclosure when it is needed and not accepted.
    fn apply_members(
        &self,
        inner: &mut Inner,
        group: &str,
        add: &[AccountId],
        remove: &[AccountId],
        accept: bool,
    ) -> Result<GroupChangeResponse, ApiErr> {
        let cfg = &inner.config;
        let g = cfg
            .group(group)
            .ok_or_else(|| ApiErr::not_found(format!("no group \"{group}\"")))?;
        let mut members = g.accounts.clone();
        for a in add {
            if !members.contains(a) {
                members.push(a.clone());
            }
        }
        members.retain(|m| !remove.contains(m));
        let mixed = mixed_identities(members.iter().filter_map(|id| cfg.account(id)));
        let covered = g
            .mixed_approval
            .as_ref()
            .is_some_and(|a| members.iter().all(|m| a.accounts.contains(m)));
        if mixed && !covered && !accept {
            return Ok(GroupChangeResponse {
                ok: false,
                needs_disclosure: true,
                disclosure: Some(disclosure(cfg, &g.name, &members)),
                message: None,
            });
        }
        let name = g.name.clone();
        let g = inner.config.group_mut(&name).expect("group");
        g.accounts = members.clone();
        if g.preferred.as_ref().is_some_and(|p| !members.contains(p)) {
            g.preferred = None;
        }
        g.ranks.retain(|k, _| members.contains(k));
        if mixed && !covered {
            let mut ids = members.clone();
            ids.sort();
            g.mixed_approval = Some(MixedApproval {
                accounts: ids,
                at: now_ms(),
            });
        }
        if !mixed {
            g.mixed_approval = None;
        }
        self.record(
            inner,
            "group.members",
            json!({"group": name, "add": add, "remove": remove, "mixedAccepted": mixed && !covered}),
            true,
        );
        Ok(GroupChangeResponse {
            ok: true,
            needs_disclosure: false,
            disclosure: None,
            message: None,
        })
    }

    pub fn group_members(
        &self,
        group: &str,
        add: &[String],
        remove: &[String],
        accept: bool,
    ) -> Result<GroupChangeResponse, ApiErr> {
        let mut inner = self.lock();
        let add: Vec<AccountId> = add
            .iter()
            .map(|k| self.find_account(&inner, k))
            .collect::<Result<_, _>>()?;
        let remove: Vec<AccountId> = remove
            .iter()
            .map(|k| self.find_account(&inner, k))
            .collect::<Result<_, _>>()?;
        let r = self.apply_members(&mut inner, group, &add, &remove, accept)?;
        if r.ok {
            self.save_config(&inner).map_err(ApiErr::invalid)?;
        }
        Ok(r)
    }

    /// Register the environment's current login (FR-02). No activation, no mode change.
    pub fn register(
        &self,
        env: Option<&EnvId>,
        label: &str,
        group: &str,
        accept: bool,
    ) -> Result<(AccountId, Option<String>), ApiErr> {
        let label = valid_label(label)?;
        let env_id = env.cloned().unwrap_or_else(|| self.default_env.id.clone());
        let envx = {
            let inner = self.lock();
            if inner.config.consents.credential_storage.is_none() {
                return Err(ApiErr::refused("credential storage was not consented to (onboarding)"));
            }
            if inner.config.group(group).is_none() {
                return Err(ApiErr::invalid(format!("no group \"{group}\"; a group is required")));
            }
            inner
                .envs
                .get(&env_id)
                .map(|s| s.env.clone())
                .ok_or_else(|| ApiErr::not_found("no such environment"))?
        };
        let login = qf_adapter::read_login(&envx)
            .map_err(ApiErr::invalid)?
            .ok_or_else(|| ApiErr::refused("no login in this environment; sign in with Claude first"))?;
        let uuid = login.account_uuid().unwrap_or_default().to_string();
        if !self.offline {
            let token = login.access_token().unwrap_or_default();
            if login.expires_at().is_some_and(|e| e < now_ms() + 30_000) {
                return Err(ApiErr::refused(
                    "this login's access token has expired; send one message in Claude (it refreshes the token), then register again",
                ));
            }
            let p = qf_provider::fetch_profile(token)
                .map_err(|e| ApiErr::refused(format!("identity could not be verified: {e}")))?;
            if p.account_uuid != uuid {
                return Err(ApiErr::refused(
                    "the profile lookup returned a different identity than the stored login",
                ));
            }
        }
        let id = AccountId::from_uuid(&uuid);
        let mut inner = self.lock();
        // Inconsistent login files (credentials of one account, record of another) name no
        // account reliably: nothing is registered or stored from them.
        let mixed = inner.envs.get(&env_id).is_some_and(|s| s.mixed.is_some())
            || matches!(self.live_owner(&inner, &login), crate::core::LiveOwner::Mixed { .. });
        if mixed {
            return Err(ApiErr::refused(
                "the login files of this environment are inconsistent (credentials of one account, account record of another); switch to an account first",
            ));
        }
        if let Some(existing) = inner.config.account(&id).cloned() {
            self.capture(&mut inner, &id, &login)
                .map_err(|e| ApiErr::new(500, "vault", e))?;
            return Ok((
                id,
                Some(format!(
                    "already registered as \"{}\"; its protected copy was updated",
                    existing.label
                )),
            ));
        }
        if inner
            .config
            .accounts
            .iter()
            .any(|a| a.label.eq_ignore_ascii_case(&label))
        {
            return Err(ApiErr::invalid(format!("the label \"{label}\" is already used")));
        }
        // Check the disclosure before storing anything.
        let account = Account {
            id: id.clone(),
            label: label.clone(),
            kind: login.kind(),
            org: login.org_uuid().map(identity_hash).unwrap_or_default(),
            subscription: login.subscription().map(String::from),
            rate_limit_tier: login.rate_limit_tier().map(String::from),
            registered_at: now_ms(),
            health: Health::Ok,
            unavailable_models: vec![],
            org_name: login.org_name(),
        };
        inner.config.accounts.push(account);
        let r = self.apply_members(&mut inner, group, std::slice::from_ref(&id), &[], accept);
        match r {
            Ok(r) if r.ok => {}
            Ok(r) => {
                inner.config.accounts.retain(|a| a.id != id);
                return Err(ApiErr::new(409, "needs_disclosure", r.disclosure.unwrap_or_default()));
            }
            Err(e) => {
                inner.config.accounts.retain(|a| a.id != id);
                return Err(e);
            }
        }
        if let Err(e) = self.vault.store(id.as_str(), &login) {
            inner.config.accounts.retain(|a| a.id != id);
            for g in inner.config.groups.iter_mut() {
                g.accounts.retain(|a| a != &id);
            }
            return Err(ApiErr::new(
                500,
                "vault",
                format!("{e}; nothing was stored (no plaintext fallback)"),
            ));
        }
        inner.config.ignored_unknown.retain(|u| u != id.as_str());
        if let Err(e) = self.save_config(&inner) {
            return Err(ApiErr::invalid(e));
        }
        for st in inner.envs.values_mut() {
            if st.unknown.as_deref() == Some(id.as_str()) {
                st.unknown = None;
            }
        }
        inner.schedule.poke(&id);
        self.record(
            &inner,
            "account.registered",
            json!({"account": id, "group": group}),
            true,
        );
        let text = format!("Registered {label} in group \"{group}\". Nothing was switched.");
        self.emit(&mut inner, "state", Some(&env_id), &text);
        Ok((id, None))
    }

    /// Import the PoC vault (`~/.quayflow-poc/vault`) once, with consent. Plaintext PoC entries
    /// are sealed into the product vault and their plaintext file is deleted.
    pub fn migrate_poc(&self, group: &str, accept: bool) -> Result<Vec<String>, ApiErr> {
        let poc = qf_adapter::env::home_dir().join(".quayflow-poc").join("vault");
        self.migrate_poc_from(&poc, group, accept)
    }

    /// [`Engine::migrate_poc`] from a given PoC vault folder. Nothing is stored before the
    /// group disclosure is settled, and plaintext PoC files are deleted only after the config
    /// naming the imported accounts was saved: a refusal or failure leaves the PoC as it was.
    pub fn migrate_poc_from(&self, poc: &std::path::Path, group: &str, accept: bool) -> Result<Vec<String>, ApiErr> {
        let mut out = vec![];
        let mut inner = self.lock();
        if inner.config.consents.credential_storage.is_none() {
            return Err(ApiErr::refused("credential storage was not consented to (onboarding)"));
        }
        let gname = inner
            .config
            .group(group)
            .ok_or_else(|| ApiErr::not_found(format!("no group \"{group}\"")))?
            .name
            .clone();
        let mut new_ids = vec![];
        // (account, login, plaintext file to delete once the import is saved)
        let mut new_logins: Vec<(AccountId, qf_vault::Login, Option<std::path::PathBuf>)> = vec![];
        let before = inner.config.clone();
        let entries = qf_vault::read_poc_vault(&self.vault, poc);
        if entries.is_empty() {
            return Ok(vec![format!(
                "No PoC logins found at {}; nothing was imported.",
                poc.display()
            )]);
        }
        for entry in entries {
            match entry {
                Err((name, e)) => out.push(format!("{name}: skipped ({e})")),
                Ok(p) => {
                    let id = AccountId(p.login.account_hash().unwrap_or_default());
                    if id.0.is_empty() {
                        out.push(format!("{}: skipped (no account record)", p.name));
                        continue;
                    }
                    if inner.config.account(&id).is_some() {
                        match self.capture(&mut inner, &id, &p.login) {
                            Ok(_) => out.push(format!(
                                "{}: already registered; protected copy refreshed if newer",
                                p.name
                            )),
                            Err(e) => out.push(format!("{}: already registered; not refreshed ({e})", p.name)),
                        }
                        continue;
                    }
                    let mut label = p.name.clone();
                    if inner
                        .config
                        .accounts
                        .iter()
                        .any(|a| a.label.eq_ignore_ascii_case(&label))
                    {
                        label = format!("{label}-{}", &id.0[..4]);
                    }
                    if new_ids.contains(&id) {
                        out.push(format!("{}: skipped (same account as another PoC entry)", p.name));
                        continue;
                    }
                    inner.config.accounts.push(Account {
                        id: id.clone(),
                        label: label.clone(),
                        kind: p.login.kind(),
                        org: p.login.org_uuid().map(identity_hash).unwrap_or_default(),
                        subscription: p.login.subscription().map(String::from),
                        rate_limit_tier: p.login.rate_limit_tier().map(String::from),
                        registered_at: now_ms(),
                        health: Health::Ok,
                        unavailable_models: vec![],
                        org_name: p.login.org_name(),
                    });
                    let plain = p.was_plaintext.then(|| poc.join(&p.name).join("login.json"));
                    new_ids.push(id.clone());
                    new_logins.push((id, p.login, plain));
                    out.push(format!("{}: imported as \"{label}\"", p.name));
                }
            }
        }
        if !new_ids.is_empty() {
            // 1. The group and its disclosure, before anything is stored.
            match self.apply_members(&mut inner, &gname, &new_ids, &[], accept) {
                Ok(r) if r.ok => {}
                Ok(r) => {
                    inner.config = before;
                    return Err(ApiErr::new(409, "needs_disclosure", r.disclosure.unwrap_or_default()));
                }
                Err(e) => {
                    inner.config = before;
                    return Err(e);
                }
            }
            // 2. Seal every login; a failure undoes the import (the PoC files are untouched).
            let mut stored: Vec<&AccountId> = vec![];
            let mut failure = None;
            for (id, login, _) in &new_logins {
                match self.vault.store(id.as_str(), login) {
                    Ok(()) => stored.push(id),
                    Err(e) => {
                        failure = Some(ApiErr::new(
                            500,
                            "vault",
                            format!("{e}; nothing was imported and the PoC logins are unchanged"),
                        ));
                        break;
                    }
                }
            }
            // 3. Save the config naming them.
            if failure.is_none() {
                if let Err(e) = self.save_config(&inner) {
                    failure = Some(ApiErr::invalid(format!(
                        "{e}; nothing was imported and the PoC logins are unchanged"
                    )));
                }
            }
            if let Some(e) = failure {
                for id in stored {
                    let _ = self.vault.remove(id.as_str());
                }
                inner.config = before;
                return Err(e);
            }
            // 4. Only now is the plaintext PoC copy redundant.
            for (_, _, plain) in &new_logins {
                if let Some(path) = plain {
                    let _ = std::fs::remove_file(path);
                }
            }
            for st in inner.envs.values_mut() {
                if st.unknown.as_ref().is_some_and(|u| new_ids.iter().any(|i| &i.0 == u)) {
                    st.unknown = None;
                }
            }
        }
        self.record(&inner, "migrate.poc", json!({"imported": new_ids}), true);
        Ok(out)
    }

    pub fn signin_start(&self, reauth: Option<&str>) -> Result<SigninStatus, ApiErr> {
        let mut inner = self.lock();
        if inner.config.consents.credential_storage.is_none() {
            return Err(ApiErr::refused("credential storage was not consented to (onboarding)"));
        }
        let reauth = match reauth {
            Some(k) => Some(self.find_account(&inner, k)?),
            None => None,
        };
        let id = random_hex(6);
        let dir = self.dd.signin_dir().join(&id);
        std::fs::create_dir_all(&dir).map_err(|e| ApiErr::invalid(e.to_string()))?;
        let env = qf_adapter::Env::at(EnvId(format!("signin-{id}")), dir.clone());
        let command = format!("$env:CLAUDE_CONFIG_DIR=\"{}\"; claude auth login", dir.display());
        let mut message = None;
        let mut state = "waiting".to_string();
        match qf_adapter::process::find_claude() {
            Some(claude) => {
                let r = qf_adapter::process::spawn_new_console(
                    &claude,
                    &["auth".into(), "login".into()],
                    &[("CLAUDE_CONFIG_DIR", Some(dir.as_os_str()))],
                    None,
                );
                if let Err(e) = r {
                    message = Some(format!("could not open a sign-in window ({e}); run the command shown"));
                }
            }
            None => message = Some("claude.exe was not found; run the command shown in a terminal".into()),
        }
        if message.is_some() {
            state = "waiting".into();
        }
        let s = Signin {
            id: id.clone(),
            env,
            state,
            reauth,
            message,
            identity: None,
            login: None,
            started_at: now_ms(),
            command,
        };
        let view = signin_view(&s);
        inner.signins.insert(id, s);
        Ok(view)
    }

    pub fn signin_status(&self, id: &str) -> Result<SigninStatus, ApiErr> {
        let (env, state, reauth) = {
            let inner = self.lock();
            let s = inner
                .signins
                .get(id)
                .ok_or_else(|| ApiErr::not_found("no such sign-in"))?;
            (s.env.clone(), s.state.clone(), s.reauth.clone())
        };
        if state == "waiting" {
            if let Ok(Some(login)) = qf_adapter::read_login(&env) {
                let mut display = None;
                let mut message = None;
                let mut verified = true;
                if !self.offline {
                    match qf_provider::fetch_profile(login.access_token().unwrap_or_default()) {
                        Ok(p) if Some(p.account_uuid.as_str()) == login.account_uuid() => display = p.display,
                        Ok(_) => {
                            verified = false;
                            message = Some("the profile lookup returned a different identity".into());
                        }
                        Err(e) => {
                            verified = false;
                            message = Some(format!("identity could not be verified yet: {e}"));
                        }
                    }
                }
                let mut inner = self.lock();
                let hash = AccountId(login.account_hash().unwrap_or_default());
                let duplicate_of = inner
                    .config
                    .account(&hash)
                    .map(|a| a.label.clone())
                    .filter(|_| reauth.is_none());
                let unexpected = reauth.as_ref().is_some_and(|r| r != &hash);
                if let Some(s) = inner.signins.get_mut(id) {
                    if verified {
                        s.state = "ready".into();
                        s.identity = Some(SigninIdentity {
                            account: hash,
                            kind: login.kind(),
                            subscription: login.subscription().map(String::from),
                            duplicate_of,
                            unexpected,
                            display,
                        });
                        s.login = Some(login);
                    }
                    s.message = message;
                }
            }
        }
        let inner = self.lock();
        Ok(signin_view(
            inner
                .signins
                .get(id)
                .ok_or_else(|| ApiErr::not_found("no such sign-in"))?,
        ))
    }

    /// Save the signed-in account, then retire the temporary copy without signing out (FR-03).
    pub fn signin_confirm(
        self: &Arc<Self>,
        id: &str,
        label: Option<&str>,
        group: Option<&str>,
        accept: bool,
        as_new: bool,
    ) -> Result<String, ApiErr> {
        let mut inner = self.lock();
        let s = inner
            .signins
            .get(id)
            .cloned()
            .ok_or_else(|| ApiErr::not_found("no such sign-in"))?;
        if s.state != "ready" {
            return Err(ApiErr::refused("the sign-in has not completed yet"));
        }
        let (Some(login), Some(identity)) = (s.login.clone(), s.identity.clone()) else {
            return Err(ApiErr::refused("the sign-in has not completed yet"));
        };
        let id_acc = identity.account.clone();
        let msg;
        // A reauthentication that returned a different identity never replaces the account
        // being reauthenticated; with s_new it is registered as its own account instead.
        let reauth = s.reauth.as_ref().filter(|_| !(as_new && identity.unexpected));
        if let Some(expected) = reauth {
            if identity.unexpected {
                return Err(ApiErr::refused(
                    "this sign-in is a different identity than the account being reauthenticated; nothing was replaced",
                ));
            }
            self.vault
                .store(expected.as_str(), &login)
                .map_err(|e| ApiErr::new(500, "vault", e.to_string()))?;
            if let Some(a) = inner.config.account_mut(expected) {
                a.health = Health::Ok;
            }
            inner.schedule.poke(expected);
            msg = format!("{} signed in again", inner.config.label(expected));
        } else if let Some(existing) = inner.config.account(&id_acc).cloned() {
            self.vault
                .store(id_acc.as_str(), &login)
                .map_err(|e| ApiErr::new(500, "vault", e.to_string()))?;
            if let Some(a) = inner.config.account_mut(&id_acc) {
                a.health = Health::Ok;
            }
            msg = format!(
                "already registered as \"{}\"; its protected copy was updated (not added as extra capacity)",
                existing.label
            );
        } else {
            let label = valid_label(label.unwrap_or(""))?;
            let group = group.ok_or_else(|| ApiErr::invalid("a group is required"))?.to_string();
            if inner
                .config
                .accounts
                .iter()
                .any(|a| a.label.eq_ignore_ascii_case(&label))
            {
                return Err(ApiErr::invalid(format!("the label \"{label}\" is already used")));
            }
            inner.config.accounts.push(Account {
                id: id_acc.clone(),
                label: label.clone(),
                kind: login.kind(),
                org: login.org_uuid().map(identity_hash).unwrap_or_default(),
                subscription: login.subscription().map(String::from),
                rate_limit_tier: login.rate_limit_tier().map(String::from),
                registered_at: now_ms(),
                health: Health::Ok,
                unavailable_models: vec![],
                org_name: login.org_name(),
            });
            match self.apply_members(&mut inner, &group, std::slice::from_ref(&id_acc), &[], accept) {
                Ok(r) if r.ok => {}
                Ok(r) => {
                    inner.config.accounts.retain(|a| a.id != id_acc);
                    return Err(ApiErr::new(409, "needs_disclosure", r.disclosure.unwrap_or_default()));
                }
                Err(e) => {
                    inner.config.accounts.retain(|a| a.id != id_acc);
                    return Err(e);
                }
            }
            if let Err(e) = self.vault.store(id_acc.as_str(), &login) {
                inner.config.accounts.retain(|a| a.id != id_acc);
                for g in inner.config.groups.iter_mut() {
                    g.accounts.retain(|a| a != &id_acc);
                }
                return Err(ApiErr::new(500, "vault", format!("{e}; nothing was stored")));
            }
            inner.schedule.poke(&id_acc);
            msg = format!("added {label} to group \"{group}\"; nothing was switched");
        }
        self.save_config(&inner).map_err(ApiErr::invalid)?;
        // Retire the temporary copy: remove the keys and the folder; no sign-out call.
        let _ = qf_adapter::login::retire(&s.env, Duration::from_secs(3));
        let _ = std::fs::remove_dir_all(&s.env.config_dir);
        if let Some(x) = inner.signins.get_mut(id) {
            x.state = "saved".into();
            x.login = None;
        }
        self.record(
            &inner,
            "account.signin_saved",
            json!({"account": id_acc, "reauth": s.reauth}),
            true,
        );
        self.emit(&mut inner, "state", None, &msg);
        Ok(msg)
    }

    pub fn signin_cancel(&self, id: &str) -> Result<(), ApiErr> {
        let mut inner = self.lock();
        let s = inner
            .signins
            .remove(id)
            .ok_or_else(|| ApiErr::not_found("no such sign-in"))?;
        let _ = qf_adapter::login::retire(&s.env, Duration::from_secs(3));
        let _ = std::fs::remove_dir_all(&s.env.config_dir);
        Ok(())
    }

    pub fn remove_account(&self, key: &str) -> Result<(), ApiErr> {
        let mut inner = self.lock();
        let id = self.find_account(&inner, key)?;
        if inner.envs.values().any(|s| s.holds_login(&id)) {
            return Err(ApiErr::refused("this account is live; switch to another account first"));
        }
        if inner.envs.values().any(|s| {
            s.transition
                .as_ref()
                .is_some_and(|t| t.to == id && !t.state.is_terminal())
        }) {
            return Err(ApiErr::refused(
                "a switch to this account is in progress; try again when it ends",
            ));
        }
        // Save the new config first (from a copy), then commit it in memory, then delete the
        // protected copy: a failure leaves memory and disk agreeing, and at worst an orphaned
        // vault entry (never a registered account without its copy).
        let mut config = inner.config.clone();
        config.accounts.retain(|a| a.id != id);
        for g in config.groups.iter_mut() {
            g.accounts.retain(|a| a != &id);
            if g.preferred.as_ref() == Some(&id) {
                g.preferred = None;
            }
            g.ranks.remove(&id);
        }
        let previous = std::mem::replace(&mut inner.config, config);
        if let Err(e) = self.save_config(&inner) {
            inner.config = previous;
            return Err(ApiErr::invalid(e));
        }
        inner.observations.retain(|o| o.account != id);
        self.save_observations(&inner);
        self.record(&inner, "account.removed", json!({"account": id}), true);
        if let Err(e) = self.vault.remove(id.as_str()) {
            self.log.log(
                "error",
                "vault.remove_failed",
                json!({"account": id, "error": e.to_string()}),
                &[],
            );
            return Err(ApiErr::new(
                500,
                "vault",
                format!("removed from Quayflow, but its protected copy could not be deleted: {e}"),
            ));
        }
        Ok(())
    }

    pub fn relabel(&self, key: &str, label: &str) -> Result<(), ApiErr> {
        let label = valid_label(label)?;
        let mut inner = self.lock();
        let id = self.find_account(&inner, key)?;
        if inner
            .config
            .accounts
            .iter()
            .any(|a| a.id != id && a.label.eq_ignore_ascii_case(&label))
        {
            return Err(ApiErr::invalid("label already used"));
        }
        inner.config.account_mut(&id).expect("account").label = label;
        self.save_config(&inner).map_err(ApiErr::invalid)
    }

    pub fn group_op(&self, op: &str, body: &Value) -> Result<String, ApiErr> {
        let mut inner = self.lock();
        let s = |k: &str| body.get(k).and_then(|v| v.as_str()).map(|x| x.trim().to_string());
        let group = s("group")
            .or_else(|| s("name"))
            .filter(|g| !g.is_empty())
            // A mode change without a group applies to the configured default group.
            .or_else(|| (op == "mode").then(|| inner.config.default_group.clone()))
            .ok_or_else(|| ApiErr::invalid("group required"))?;
        let msg = match op {
            "create" => {
                if inner.config.group(&group).is_some() {
                    return Err(ApiErr::invalid("a group with that name exists"));
                }
                inner.config.groups.push(Group::new(&group));
                format!("group \"{group}\" created")
            }
            "delete" => {
                if inner.config.default_group.eq_ignore_ascii_case(&group) {
                    return Err(ApiErr::refused(
                        "the default group cannot be deleted; make another group the default first",
                    ));
                }
                if inner
                    .config
                    .projects
                    .iter()
                    .any(|p| p.group.eq_ignore_ascii_case(&group))
                {
                    return Err(ApiErr::refused(
                        "projects are assigned to this group; unassign them first",
                    ));
                }
                inner.config.groups.retain(|g| !g.name.eq_ignore_ascii_case(&group));
                format!("group \"{group}\" deleted")
            }
            "preferred" => {
                let acc = match s("account").filter(|a| !a.is_empty() && a != "none") {
                    Some(k) => Some(self.find_account(&inner, &k)?),
                    None => None,
                };
                let g = inner
                    .config
                    .group_mut(&group)
                    .ok_or_else(|| ApiErr::not_found("no such group"))?;
                if let Some(a) = &acc {
                    if !g.accounts.contains(a) {
                        return Err(ApiErr::invalid("the Preferred account must be a member of the group"));
                    }
                }
                g.preferred = acc;
                "home account updated".into()
            }
            "rank" => {
                let acc = self.find_account(&inner, &s("account").unwrap_or_default())?;
                let rank = match s("rank").as_deref() {
                    Some("preferred") => Rank::Preferred,
                    Some("normal") => Rank::Normal,
                    Some("backup") => Rank::Backup,
                    _ => return Err(ApiErr::invalid("rank is preferred, normal or backup")),
                };
                let g = inner
                    .config
                    .group_mut(&group)
                    .ok_or_else(|| ApiErr::not_found("no such group"))?;
                if !g.accounts.contains(&acc) {
                    return Err(ApiErr::invalid("not a member of the group"));
                }
                if rank == Rank::Preferred {
                    // One Preferred (home) per group: selecting another replaces it explicitly.
                    g.preferred = Some(acc.clone());
                    g.ranks.remove(&acc);
                } else {
                    if g.preferred.as_ref() == Some(&acc) {
                        g.preferred = None;
                    }
                    g.ranks.insert(acc, rank);
                }
                "rank updated".into()
            }
            "mode" => {
                let mode = Mode::parse(&s("mode").unwrap_or_default())
                    .ok_or_else(|| ApiErr::invalid("mode is manual, suggest or automatic"))?;
                let approve = body.get("approveAutomatic").and_then(|v| v.as_bool()).unwrap_or(false);
                let g = inner
                    .config
                    .group_mut(&group)
                    .ok_or_else(|| ApiErr::not_found("no such group"))?;
                if mode == Mode::Automatic && g.automatic_approved_at.is_none() {
                    if !approve {
                        return Err(ApiErr::new(
                            409,
                            "needs_approval",
                            "Automatic mode switches accounts of this group without asking, including in unattended headless jobs. Approve it explicitly (approveAutomatic).",
                        ));
                    }
                    g.automatic_approved_at = Some(now_ms());
                }
                g.mode = mode;
                format!("group \"{group}\" mode: {}", mode.as_str())
            }
            "tuning" => {
                let g = inner
                    .config
                    .group_mut(&group)
                    .ok_or_else(|| ApiErr::not_found("no such group"))?;
                if let Some(t) = body.get("threshold").and_then(|v| v.as_f64()) {
                    g.threshold = t;
                }
                if let Some(m) = body.get("margin").and_then(|v| v.as_f64()) {
                    g.margin = m;
                }
                "tuning updated".into()
            }
            "default" => {
                let name = inner
                    .config
                    .group(&group)
                    .ok_or_else(|| ApiErr::not_found("no such group"))?
                    .name
                    .clone();
                inner.config.default_group = name.clone();
                format!("\"{name}\" is now the default group for unassigned projects")
            }
            _ => return Err(ApiErr::not_found("unknown group operation")),
        };
        if let Err(e) = self.save_config(&inner) {
            // Reload the saved config: the change is not kept.
            if let Ok(c) = crate::core::load_config(&self.dd) {
                inner.config = c;
            }
            return Err(ApiErr::invalid(e));
        }
        self.record(&inner, "group.change", json!({"op": op, "group": group}), false);
        self.emit(&mut inner, "state", None, &msg);
        Ok(msg)
    }

    pub fn project_op(&self, op: &str, path: &str, group: Option<&str>) -> Result<String, ApiErr> {
        let mut inner = self.lock();
        let p = path.trim().trim_end_matches(['\\', '/']).to_string();
        if p.len() < 3 {
            return Err(ApiErr::invalid("an absolute project folder is required"));
        }
        let norm = normalize_path(&p);
        // Validate before changing anything: a refused assign keeps the existing assignment.
        let assign_to = if op == "assign" {
            let g = group.ok_or_else(|| ApiErr::invalid("group required"))?;
            Some(
                inner
                    .config
                    .group(g)
                    .ok_or_else(|| ApiErr::not_found("no such group"))?
                    .name
                    .clone(),
            )
        } else {
            None
        };
        let before = inner.config.projects.clone();
        inner.config.projects.retain(|x| normalize_path(&x.path) != norm);
        let msg = if let Some(name) = assign_to {
            inner.config.projects.push(ProjectAssignment {
                path: p.clone(),
                group: name.clone(),
            });
            format!("{p} uses group \"{name}\" (replaces the default)")
        } else {
            format!("{p} uses the default group")
        };
        if let Err(e) = self.save_config(&inner) {
            inner.config.projects = before;
            return Err(ApiErr::invalid(e));
        }
        self.record(&inner, "project.change", json!({"op": op}), false);
        self.emit(&mut inner, "state", None, &msg);
        Ok(msg)
    }

    pub fn ignore_unknown(&self, env: &EnvId) -> Result<String, ApiErr> {
        let mut inner = self.lock();
        let u = inner
            .envs
            .get(env)
            .and_then(|s| s.unknown.clone())
            .ok_or_else(|| ApiErr::not_found("no unknown login here"))?;
        if !inner.config.ignored_unknown.contains(&u) {
            inner.config.ignored_unknown.push(u);
        }
        self.save_config(&inner).map_err(ApiErr::invalid)?;
        Ok("Notice acknowledged. Automatic switching and recovery stay paused while this login is live; they resume when a registered login is live again.".into())
    }

    /// Managed-login override setting (FR-06a): record the setting with its confirmation. This build contains no
    /// code path that lets a switch through managed login enforcement.
    pub fn set_dorg(
        &self,
        enable: bool,
        confirm_org: Option<&str>,
        acknowledge: Option<&str>,
    ) -> Result<String, ApiErr> {
        let mut inner = self.lock();
        if enable {
            let enforced: Vec<String> = inner.envs.values().flat_map(|s| s.managed.force_org.clone()).collect();
            let org = confirm_org.unwrap_or("");
            if !enforced.iter().any(|o| o == org) {
                return Err(ApiErr::invalid("name the enforcing organization exactly as shown"));
            }
            if !acknowledge.is_some_and(|a| a.trim().to_ascii_lowercase().starts_with("i accept the risk")) {
                return Err(ApiErr::invalid("type \"I accept the risk\" to confirm"));
            }
            inner.config.d_org = DOrgSetting {
                enabled: true,
                confirmed_org: Some(org.into()),
                confirmed_at: Some(now_ms()),
            };
        } else {
            inner.config.d_org = DOrgSetting::default();
        }
        self.save_config(&inner).map_err(ApiErr::invalid)?;
        self.record(
            &inner,
            "dorg.setting",
            json!({"enabled": enable, "org": confirm_org}),
            true,
        );
        Ok(if enable {
            "Setting recorded. This build does not implement the override: managed login enforcement is still respected.".into()
        } else {
            "Override setting off.".into()
        })
    }
}

fn signin_view(s: &Signin) -> SigninStatus {
    SigninStatus {
        id: s.id.clone(),
        state: s.state.clone(),
        config_dir: s.env.config_dir.display().to_string(),
        command: s.command.clone(),
        identity: s.identity.clone(),
        message: s.message.clone(),
    }
}
