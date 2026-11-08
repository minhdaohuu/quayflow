//! Engine state and the automation loop: watcher, observation, decisions.

use crate::journal::{Journal, Logger};
use crate::paths::DataDir;
use qf_adapter::Env;
use qf_model::api::{ActivityItem, EngineEvent, InPlace};
use qf_model::time::now_ms;
use qf_model::*;
use qf_provider::{ProviderError, Schedule};
use qf_vault::{Login, Vault};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

pub type Shared = Arc<Engine>;

#[derive(Debug)]
pub struct EnvState {
    pub env: Env,
    pub live: Option<AccountId>,
    pub signed_out: bool,
    /// Live login is not a registered account (hash of its account UUID).
    pub unknown: Option<String>,
    pub last_stamp: Option<(u128, u64)>,
    pub own_stamp: Option<(u128, u64)>,
    pub transition: Option<Transition>,
    pub cancel: Arc<AtomicBool>,
    pub last_switch: Option<Transition>,
    pub last_proactive: Option<i64>,
    pub pin: Option<Pin>,
    /// An unpin received while this transition was still in progress: it must not pin on success.
    pub unpinned_transition: Option<String>,
    pub suggestion: Option<Suggestion>,
    pub adoption: Vec<ClientAdoption>,
    pub sessions: Vec<SessionInfo>,
    pub decision: String,
    pub external_changes: Vec<i64>,
    pub other_switcher: bool,
    pub managed: ManagedLogin,
    pub policy_fp: Option<String>,
    pub in_place: InPlace,
    pub lane_canary: bool,
    /// Automation is held here until a switch writes a consistent login (or, unless `mixed`
    /// is set, until automation is resumed): why.
    pub hold: Option<String>,
    /// The login files are inconsistent: the credentials belong to this registered account
    /// while `.claude.json` names another. No token is stored from this environment under the
    /// account `.claude.json` names, and the account counts as live here.
    pub mixed: Option<AccountId>,
}

impl EnvState {
    pub fn new(env: Env) -> Self {
        EnvState {
            env,
            live: None,
            signed_out: false,
            unknown: None,
            last_stamp: None,
            own_stamp: None,
            transition: None,
            cancel: Arc::new(AtomicBool::new(false)),
            last_switch: None,
            last_proactive: None,
            pin: None,
            unpinned_transition: None,
            suggestion: None,
            adoption: vec![],
            sessions: vec![],
            decision: "starting".into(),
            external_changes: vec![],
            other_switcher: false,
            managed: ManagedLogin::default(),
            policy_fp: None,
            in_place: InPlace {
                allowed: false,
                reason: "not checked yet".into(),
                governing_version: None,
            },
            lane_canary: false,
            hold: None,
            mixed: None,
        }
    }

    /// Whether `id`'s login is in use in this environment (live, or its credentials are).
    pub fn holds_login(&self, id: &AccountId) -> bool {
        self.live.as_ref() == Some(id) || self.mixed.as_ref() == Some(id)
    }

    /// A switch wrote a consistent login: holds no longer apply.
    pub fn clear_hold(&mut self) {
        self.hold = None;
        self.mixed = None;
    }

    pub fn blocking_transition(&self) -> bool {
        self.transition.as_ref().is_some_and(|t| {
            matches!(
                t.state,
                TransitionState::Requested
                    | TransitionState::Preparing
                    | TransitionState::IdleWait
                    | TransitionState::Applying
            )
        })
    }

    /// Why automation (automatic switching and recovery) is paused here, if it is.
    pub fn pause_reason(&self, cfg: &Config) -> Option<String> {
        if cfg.automation_paused {
            return Some("automation paused by you (/quay resume)".into());
        }
        if let Some(h) = &self.hold {
            return Some(h.clone());
        }
        if self.signed_out {
            return Some("no login in this environment".into());
        }
        if let Some(u) = &self.unknown {
            return Some(format!(
                "the live login #{u} is not registered: register it (/quay register) or switch manually{}",
                if cfg.ignored_unknown.contains(u) {
                    " (notice acknowledged)"
                } else {
                    ""
                }
            ));
        }
        if self.other_switcher {
            return Some(
                "the login changed repeatedly without Quayflow: another tool may be switching accounts".into(),
            );
        }
        if !self.in_place.allowed {
            return Some(format!("in-place switching disabled: {}", self.in_place.reason));
        }
        None
    }

    /// The cause behind [`EnvState::pause_reason`], as a machine word (same order). Only
    /// "user" and "hold" end when automation is resumed.
    pub fn pause_kind(&self, cfg: &Config) -> Option<&'static str> {
        if cfg.automation_paused {
            Some("user")
        } else if self.hold.is_some() {
            Some(if self.mixed.is_some() { "repair" } else { "hold" })
        } else if self.signed_out {
            Some("signed_out")
        } else if self.unknown.is_some() {
            Some("unknown_login")
        } else if self.other_switcher {
            Some("other_switcher")
        } else if !self.in_place.allowed {
            Some("incompatible")
        } else {
            None
        }
    }
}

/// Whose login an environment's files hold (see [`Engine::live_owner`]).
#[derive(Debug, Clone)]
pub enum LiveOwner {
    /// A registered account, consistent as far as can be told.
    Registered(AccountId),
    /// Not a registered account.
    Unknown(AccountId),
    /// The credentials are `owner`'s (its stored refresh token) while `.claude.json` names
    /// `claimed`. `login` is the credentials with `owner`'s own account record.
    Mixed {
        owner: AccountId,
        claimed: AccountId,
        login: Login,
    },
}

#[derive(Debug, Clone)]
pub struct Signin {
    pub id: String,
    pub env: Env,
    pub state: String,
    pub reauth: Option<AccountId>,
    pub message: Option<String>,
    pub identity: Option<api::SigninIdentity>,
    pub login: Option<Login>,
    pub started_at: i64,
    pub command: String,
}

#[derive(Debug, Clone, Default)]
pub struct CanaryRun {
    pub since: i64,
    /// (ts, session, index, agent, account, stopReason)
    pub steps: Vec<Value>,
}

pub struct Inner {
    pub config: Config,
    pub observations: Vec<Observation>,
    pub schedule: Schedule,
    pub envs: BTreeMap<EnvId, EnvState>,
    pub compat: BTreeMap<String, CompatRecord>,
    /// session id -> (account, ts) of its last verified request
    pub verified: HashMap<String, (AccountId, i64)>,
    /// recovery incident keys already handled
    pub incidents: HashMap<String, i64>,
    /// session id -> genuine progress since its last recovery
    pub progressed: HashMap<String, bool>,
    pub signins: HashMap<String, Signin>,
    pub canary: Option<CanaryRun>,
    /// (method path requestId) -> response; None while the first request is still running.
    pub request_cache: VecDeque<(String, Option<Value>)>,
    pub activity: VecDeque<ActivityItem>,
    /// Last model a session reported.
    pub session_model: HashMap<String, String>,
    /// macOS: session id -> (the Keychain service it reads, whether it uses a separate login
    /// store through CLAUDE_SECURESTORAGE_CONFIG_DIR, when it said so), from its handshake.
    pub session_keychain: HashMap<String, (String, bool, i64)>,
    /// A support report built for review and not saved yet: (id, text).
    pub pending_report: Option<(String, String)>,
}

pub struct Events {
    pub seq: u64,
    pub list: VecDeque<EngineEvent>,
}

pub struct Engine {
    pub dd: DataDir,
    pub vault: Vault,
    pub journal: Journal,
    pub log: Logger,
    pub inner: Mutex<Inner>,
    /// Serializes credential operations across environments.
    pub cred_lock: Mutex<()>,
    pub events: Mutex<Events>,
    pub events_cv: Condvar,
    /// Event long-polls waiting now (bounded, see `http::MAX_LONG_POLLS`).
    pub long_polls: AtomicUsize,
    pub started_at: i64,
    pub token: String,
    pub shutdown: AtomicBool,
    pub exe_sha256: String,
    pub default_env: Env,
    /// Disable network calls (tests).
    pub offline: bool,
}

/// `n` random bytes as hex, or an error when the OS random source fails (never zeros).
pub fn try_random_hex(n: usize) -> Result<String, String> {
    let mut b = vec![0u8; n];
    getrandom::getrandom(&mut b).map_err(|e| format!("the OS random source is unavailable: {e}"))?;
    Ok(hex::encode(b))
}

/// Random id as hex. Ids must be unpredictable: a failing OS random source panics rather
/// than producing a predictable value.
pub fn random_hex(n: usize) -> String {
    try_random_hex(n).expect("random source")
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ObsFile {
    observations: Vec<Observation>,
    schedule: Schedule,
}

impl Engine {
    pub fn new(dd: DataDir, default_env: Env, offline: bool) -> Result<Shared, String> {
        dd.ensure()
            .map_err(|e| format!("data folder {}: {e}", dd.root.display()))?;
        // The API token must be unpredictable: no engine without a working random source.
        let token = try_random_hex(24)?;
        let config = load_config(&dd)?;
        let obs: ObsFile = std::fs::read_to_string(dd.observations())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mut schedule = obs.schedule;
        schedule.after_restart(&obs.observations);
        let mut compat = BTreeMap::new();
        if let Ok(d) = std::fs::read_dir(dd.compat_dir()) {
            for e in d.flatten() {
                if let Some(r) = std::fs::read_to_string(e.path())
                    .ok()
                    .and_then(|t| serde_json::from_str::<CompatRecord>(&t).ok())
                {
                    compat.insert(r.version.clone(), r);
                }
            }
        }
        let mut envs = BTreeMap::new();
        envs.insert(default_env.id.clone(), EnvState::new(default_env.clone()));
        if let Ok(d) = std::fs::read_dir(dd.lanes()) {
            for e in d.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if e.path().is_dir() && qf_adapter::lanes::valid_name(&name) {
                    let env = qf_adapter::lanes::lane_env(&dd.lanes(), &name);
                    let mut st = EnvState::new(env);
                    st.lane_canary = lane_meta(&dd, &name).get("canaryPassedAt").is_some_and(|v| v.is_i64());
                    envs.insert(EnvId(name), st);
                }
            }
        }
        let log = Logger::new(dd.logs(), config.tuning.log_retention_days, config.tuning.log_max_mb);
        let exe_sha256 = std::env::current_exe()
            .ok()
            .and_then(|p| std::fs::read(p).ok())
            .map(|b| {
                use sha2::Digest;
                hex::encode(sha2::Sha256::digest(&b))
            })
            .unwrap_or_default();
        let engine = Arc::new(Engine {
            vault: Vault::new(dd.vault()),
            journal: Journal::new(dd.journal()),
            log,
            inner: Mutex::new(Inner {
                config,
                observations: obs.observations,
                schedule,
                envs,
                compat,
                verified: HashMap::new(),
                incidents: HashMap::new(),
                progressed: HashMap::new(),
                signins: HashMap::new(),
                canary: None,
                request_cache: VecDeque::new(),
                activity: VecDeque::new(),
                session_model: HashMap::new(),
                session_keychain: HashMap::new(),
                pending_report: None,
            }),
            cred_lock: Mutex::new(()),
            events: Mutex::new(Events {
                seq: 0,
                list: VecDeque::new(),
            }),
            events_cv: Condvar::new(),
            long_polls: AtomicUsize::new(0),
            started_at: now_ms(),
            token,
            shutdown: AtomicBool::new(false),
            exe_sha256,
            default_env,
            offline,
            dd,
        });
        crate::reconcile::reconcile(&engine);
        {
            let mut inner = engine.lock();
            let ids: Vec<EnvId> = inner.envs.keys().cloned().collect();
            for id in ids {
                // Mixed login files are held in memory only: find them again after a restart,
                // so another account's tokens are never read or stored as the named account's.
                let login = inner
                    .envs
                    .get(&id)
                    .and_then(|st| qf_adapter::read_login(&st.env).ok().flatten());
                if let Some(login) = login {
                    if let LiveOwner::Mixed { owner, claimed, .. } = engine.live_owner(&inner, &login) {
                        engine.hold_mixed(&mut inner, &id, &owner, Some(&claimed));
                    }
                }
                engine.read_live(&mut inner, &id, true);
                engine.refresh_env(&mut inner, &id);
            }
        }
        engine.log(
            "info",
            "engine.start",
            json!({"version": ENGINE_VERSION, "protocol": PROTOCOL}),
        );
        Ok(engine)
    }

    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn labels(inner: &Inner) -> Vec<(String, String)> {
        inner
            .config
            .accounts
            .iter()
            .map(|a| (a.label.clone(), a.id.0.clone()))
            .collect()
    }

    pub fn log(&self, level: &str, event: &str, data: Value) {
        let labels = Engine::labels(&self.lock());
        self.log.log(level, event, data, &labels);
    }

    /// Journal a record; `durable` fsyncs. Returns false when it could not be recorded.
    pub fn record(&self, inner: &Inner, event: &str, data: Value, durable: bool) -> bool {
        let labels = Engine::labels(inner);
        let ok = self.journal.append(event, data.clone(), &labels, durable).is_ok();
        self.log.log("info", event, data, &labels);
        ok
    }

    pub fn emit(&self, inner: &mut Inner, kind: &str, env: Option<&EnvId>, text: &str) {
        inner.activity.push_back(ActivityItem {
            ts: now_ms(),
            kind: kind.into(),
            text: text.into(),
        });
        while inner.activity.len() > 200 {
            inner.activity.pop_front();
        }
        let mut ev = self.events.lock().unwrap_or_else(|e| e.into_inner());
        ev.seq += 1;
        let seq = ev.seq;
        ev.list.push_back(EngineEvent {
            seq,
            ts: now_ms(),
            kind: kind.into(),
            env: env.cloned(),
            text: Some(text.into()),
        });
        while ev.list.len() > 500 {
            ev.list.pop_front();
        }
        self.events_cv.notify_all();
    }

    /// Wake event watchers without a message or an activity entry: the state changed in a way
    /// clients should redraw (new usage figures), but there is nothing to tell the user.
    pub fn signal(&self, kind: &str) {
        let mut ev = self.events.lock().unwrap_or_else(|e| e.into_inner());
        ev.seq += 1;
        let seq = ev.seq;
        ev.list.push_back(EngineEvent {
            seq,
            ts: now_ms(),
            kind: kind.into(),
            env: None,
            text: None,
        });
        while ev.list.len() > 500 {
            ev.list.pop_front();
        }
        self.events_cv.notify_all();
    }

    pub fn seq(&self) -> u64 {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).seq
    }

    pub fn save_config(&self, inner: &Inner) -> Result<(), String> {
        inner.config.validate()?;
        let text = serde_json::to_string_pretty(&inner.config).map_err(|e| e.to_string())?;
        qf_vault::atomic_write(&self.dd.config(), text.as_bytes()).map_err(|e| e.to_string())
    }

    pub fn save_observations(&self, inner: &Inner) {
        let f = ObsFile {
            observations: inner.observations.clone(),
            schedule: inner.schedule.clone(),
        };
        if let Ok(t) = serde_json::to_string(&f) {
            let _ = qf_vault::atomic_write(&self.dd.observations(), t.as_bytes());
        }
    }

    pub fn save_compat(&self, rec: &CompatRecord) {
        // The version string comes from clients; it names a file only when it is a plain version.
        if !safe_version_name(&rec.version) {
            self.log.log(
                "warn",
                "compat.unsafe_version",
                json!({"length": rec.version.len()}),
                &[],
            );
            return;
        }
        if let Ok(t) = serde_json::to_string_pretty(rec) {
            let _ = qf_vault::atomic_write(
                &self.dd.compat_dir().join(format!("{}.json", rec.version)),
                t.as_bytes(),
            );
        }
    }

    /// The environment a session runs in (registry lookup), default when unknown.
    pub fn env_of_session(&self, inner: &Inner, session_id: &str) -> EnvId {
        inner
            .envs
            .iter()
            .find(|(_, st)| st.sessions.iter().any(|s| s.session_id == session_id))
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| self.default_env.id.clone())
    }

    /// macOS: why a session cannot follow a switch of its environment in place, when it reads
    /// its login from another Keychain item than the one Quayflow writes there (it was started
    /// with another spelling of `CLAUDE_CONFIG_DIR`, or with `CLAUDE_SECURESTORAGE_CONFIG_DIR`).
    /// Only sessions listed in an environment's registry are checked.
    pub fn keychain_mismatch(&self, inner: &Inner, session_id: &str) -> Option<String> {
        let (service, separate, _) = inner.session_keychain.get(session_id)?;
        let st = inner
            .envs
            .values()
            .find(|st| st.sessions.iter().any(|s| s.session_id == session_id))?;
        let ours = qf_adapter::keychain::service_name(&st.env)?;
        if *separate {
            return Some(
                "it was started with CLAUDE_SECURESTORAGE_CONFIG_DIR, so its login is stored outside this environment"
                    .into(),
            );
        }
        (*service != ours).then(|| {
            if ours == qf_adapter::keychain::BASE_SERVICE {
                "it reads another Keychain item than Quayflow writes here; start it without CLAUDE_CONFIG_DIR".into()
            } else {
                format!(
                    "it reads another Keychain item than Quayflow writes here; start it with CLAUDE_CONFIG_DIR set to exactly {}",
                    st.env.config_dir.display()
                )
            }
        })
    }

    /// Groups governing an environment: the running sessions' groups, or the default group.
    pub fn governing<'a>(inner: &'a Inner, env: &EnvId) -> Vec<&'a Group> {
        let cfg = &inner.config;
        let mut names: Vec<&str> = inner
            .envs
            .get(env)
            .map(|st| st.sessions.iter().filter_map(|s| s.group.as_deref()).collect())
            .unwrap_or_default();
        names.sort();
        names.dedup();
        if names.is_empty() {
            names.push(&cfg.default_group);
        }
        names.iter().filter_map(|n| cfg.group(n)).collect()
    }

    pub fn ctx<'a>(inner: &'a Inner, env: &EnvId, model: Option<&'a str>) -> Option<qf_policy::Ctx<'a>> {
        let st = inner.envs.get(env)?;
        Some(qf_policy::Ctx {
            now: now_ms(),
            accounts: &inner.config.accounts,
            governing: Engine::governing(inner, env),
            sessions: &st.sessions,
            all_groups: &inner.config.groups,
            observations: &inner.observations,
            live: st.live.as_ref().filter(|_| st.unknown.is_none()),
            managed: &st.managed,
            tuning: &inner.config.tuning,
            pin: st.pin.as_ref(),
            last_proactive_switch: st.last_proactive,
            model,
            busy_transition: st.blocking_transition(),
        })
    }

    /// Read the live identity of an environment from `.claude.json` (no token read).
    pub fn read_live(&self, inner: &mut Inner, env_id: &EnvId, initial: bool) {
        let Some(st) = inner.envs.get(env_id) else { return };
        let env = st.env.clone();
        let uuid = qf_adapter::read_identity(&env);
        let has_login = qf_adapter::read_login(&env).ok().flatten().is_some();
        let known_ids: Vec<AccountId> = inner.config.accounts.iter().map(|a| a.id.clone()).collect();
        let st = inner.envs.get_mut(env_id).expect("env");
        st.last_stamp = qf_adapter::login::credentials_stamp(&env);
        if st.mixed.is_some() {
            // `.claude.json` does not name the credentials' account: it says nothing reliable.
            st.live = None;
            st.unknown = None;
            st.signed_out = false;
            return;
        }
        match uuid.filter(|_| has_login) {
            None => {
                st.live = None;
                st.signed_out = true;
                st.unknown = None;
            }
            Some(u) => {
                let id = AccountId::from_uuid(&u);
                st.signed_out = false;
                if known_ids.contains(&id) {
                    st.unknown = None;
                } else {
                    st.unknown = Some(id.0.clone());
                }
                st.live = Some(id);
            }
        }
        let _ = initial;
    }

    /// Sessions, policy, version gating for one environment.
    pub fn refresh_env(&self, inner: &mut Inner, env_id: &EnvId) {
        let Some(st) = inner.envs.get(env_id) else { return };
        let env = st.env.clone();
        let mut sessions = qf_adapter::sessions::read_sessions(&env);
        for s in &mut sessions {
            s.group = Some(inner.config.resolve_group(s.cwd.as_deref()).to_string());
        }
        // Policy: re-check when the policy files change (FR-06a).
        let fp = qf_adapter::policy::fingerprint(&env);
        let recheck = inner
            .envs
            .get(env_id)
            .map(|s| s.policy_fp.as_ref() != Some(&fp))
            .unwrap_or(true);
        let managed = recheck.then(|| qf_adapter::policy::detect(&env));
        // Version gating (FR-24): every running version must pass; the oldest governs.
        let mut versions: Vec<String> = sessions.iter().filter_map(|s| s.version.clone()).collect();
        versions.sort_by(|a, b| compare_versions(a, b));
        versions.dedup();
        if versions.is_empty() {
            if let Some(v) = installed_claude_version() {
                versions.push(v);
            }
        }
        for v in &versions {
            if !inner.compat.contains_key(v) {
                let rec = static_check(v);
                self.save_compat(&rec);
                let _ = self.record(
                    inner,
                    "compat.static",
                    json!({"version": v, "pass": rec.static_pass, "detail": rec.static_detail}),
                    false,
                );
                inner.compat.insert(v.clone(), rec);
            }
        }
        let governing_version = versions.first().cloned();
        let failing = versions
            .iter()
            .find(|v| !inner.compat.get(*v).is_some_and(|r| r.in_place_allowed()));
        let is_lane = !env_id.is_default();
        let lane_canary = inner.envs.get(env_id).map(|s| s.lane_canary).unwrap_or(false);
        let in_place = match (governing_version.as_ref(), failing) {
            (None, _) => InPlace {
                allowed: false,
                reason: "no Claude version found to check".into(),
                governing_version: None,
            },
            (_, Some(v)) => {
                let r = inner.compat.get(v);
                let why = match r {
                    Some(r) if !r.static_pass => {
                        format!("Claude {v} failed the static check ({})", r.static_detail.join("; "))
                    }
                    Some(r) => format!(
                        "Claude {v} did not adopt a switched login ({})",
                        r.live_detail.clone().unwrap_or_default()
                    ),
                    None => format!("Claude {v} not checked"),
                };
                InPlace {
                    allowed: false,
                    reason: why,
                    governing_version: governing_version.clone(),
                }
            }
            (Some(v), None) if is_lane && !lane_canary => InPlace {
                allowed: false,
                reason: "this lane has not passed its check yet (quayflow lanes check)".into(),
                governing_version: Some(v.clone()),
            },
            (Some(v), None) => InPlace {
                allowed: true,
                reason: format!("Claude {v} passed the static check"),
                governing_version: Some(v.clone()),
            },
        };
        let st = inner.envs.get_mut(env_id).expect("env");
        // Adoption entries of clients that ended no longer wait.
        let alive: Vec<String> = sessions.iter().map(|s| s.session_id.clone()).collect();
        for a in st.adoption.iter_mut() {
            if a.state == AdoptionState::PendingNextRequest && !alive.contains(&a.session_id) {
                a.state = AdoptionState::Adopted;
                a.at = Some(now_ms());
                a.last_request_account = None;
            }
        }
        st.sessions = sessions;
        if let Some(m) = managed {
            st.managed = m;
            st.policy_fp = Some(fp);
        }
        st.in_place = in_place;
    }

    // ---------------- watcher ----------------

    /// Classify login file changes: our own write, a native refresh (captured into the
    /// vault), or a different identity (unknown login or another switcher).
    pub fn watch(&self) {
        let ids: Vec<EnvId> = self.lock().envs.keys().cloned().collect();
        for id in ids {
            let (env, last, own) = {
                let inner = self.lock();
                let Some(st) = inner.envs.get(&id) else { continue };
                (st.env.clone(), st.last_stamp, st.own_stamp)
            };
            let stamp = qf_adapter::login::credentials_stamp(&env);
            if stamp == last {
                continue;
            }
            if stamp.is_some() && stamp == own {
                if let Some(s) = self.lock().envs.get_mut(&id) {
                    s.last_stamp = stamp;
                }
                continue;
            }
            // Read a consistent login under Claude's read locks; skip this round if held.
            let _cred = self.cred_lock.lock().unwrap_or_else(|e| e.into_inner());
            let login = match qf_adapter::lock::acquire_read(&env, Duration::from_secs(3)) {
                Ok(held) => {
                    let l = qf_adapter::read_login(&env);
                    drop(held);
                    l
                }
                Err(_) => continue,
            };
            let mut inner = self.lock();
            let Some(st) = inner.envs.get_mut(&id) else { continue };
            // Re-check under the engine lock: a Quayflow switch may have written these files and
            // completed since the snapshot above. Our own write is never an external change.
            if stamp.is_some() && stamp == st.own_stamp {
                st.last_stamp = stamp;
                continue;
            }
            // Another caller (the tick, or a switch request) already handled this change.
            if stamp == st.last_stamp {
                continue;
            }
            if st.blocking_transition() {
                continue; // our own switch is writing; it records the new stamp when it completes
            }
            let prev_live = st.live.clone();
            let was_mixed = st.mixed.is_some();
            st.last_stamp = stamp;
            match login {
                // Inconsistent login files: nothing here is stored under any account until a
                // switch writes a consistent login.
                Ok(Some(_)) if was_mixed => {}
                Ok(Some(login)) => {
                    let h = AccountId(login.account_hash().unwrap_or_default());
                    if let LiveOwner::Mixed {
                        owner, login: fixed, ..
                    } = self.live_owner(&inner, &login)
                    {
                        // The credentials are another registered account's: store them under that
                        // account only, and hold this environment.
                        if let Err(e) = self.capture(&mut inner, &owner, &fixed) {
                            self.log.log(
                                "error",
                                "vault.capture_failed",
                                json!({"account": owner, "error": e}),
                                &[],
                            );
                        }
                        self.hold_mixed(&mut inner, &id, &owner, Some(&h));
                        continue;
                    }
                    let registered = inner.config.account(&h).is_some();
                    let same = prev_live.as_ref() == Some(&h);
                    if registered {
                        let captured = match self.capture(&mut inner, &h, &login) {
                            Ok(c) => c,
                            Err(e) => {
                                self.log
                                    .log("error", "vault.capture_failed", json!({"account": h, "error": e}), &[]);
                                false
                            }
                        };
                        if let Some(a) = inner.config.account_mut(&h) {
                            if a.health == Health::StaleAccess {
                                a.health = Health::Ok;
                            }
                        }
                        let st = inner.envs.get_mut(&id).expect("env");
                        st.unknown = None;
                        st.signed_out = false;
                        st.live = Some(h.clone());
                        if same {
                            if captured {
                                self.record(&inner, "vault.capture", json!({"account": h, "env": id}), false);
                            }
                        } else {
                            self.external_change(&mut inner, &id, Some(&h));
                        }
                        inner.schedule.poke(&h);
                    } else {
                        let st = inner.envs.get_mut(&id).expect("env");
                        st.unknown = Some(h.0.clone());
                        st.live = Some(h.clone());
                        st.signed_out = false;
                        if !same {
                            self.external_change(&mut inner, &id, None);
                        }
                    }
                }
                Ok(None) => {
                    let st = inner.envs.get_mut(&id).expect("env");
                    // No credentials left, so none are mixed (a hold stays until resumed).
                    st.mixed = None;
                    if !st.signed_out {
                        st.signed_out = true;
                        st.live = None;
                        st.unknown = None;
                        self.record(&inner, "login.signed_out", json!({"env": id}), false);
                        self.emit(
                            &mut inner,
                            "notice",
                            Some(&id),
                            "No login in this environment: Claude was signed out. Automation is paused.",
                        );
                    }
                }
                Err(_) => {}
            }
        }
    }

    fn external_change(&self, inner: &mut Inner, env: &EnvId, to: Option<&AccountId>) {
        let now = now_ms();
        let st = inner.envs.get_mut(env).expect("env");
        st.external_changes.retain(|t| now - t < 10 * 60_000);
        st.external_changes.push(now);
        st.suggestion = None;
        st.pin = None;
        let repeated = st.external_changes.len() >= 2;
        if repeated {
            st.other_switcher = true;
        }
        let unknown = st.unknown.clone();
        self.record(
            inner,
            "login.external_change",
            json!({"env": env, "to": to, "unknown": unknown, "repeated": repeated}),
            true,
        );
        let text = match (to, repeated) {
            (_, true) => "The login changed repeatedly without Quayflow: another tool may be switching accounts. Automation is paused.".to_string(),
            (Some(id), false) => format!("The login changed outside Quayflow to {}.", inner.config.label(id)),
            (None, false) => "The live login is not a registered account. Automation is paused: register it (/quay register) or ignore.".into(),
        };
        self.emit(inner, "notice", Some(env), &text);
    }

    /// Store a login in its vault entry when it is newer than the stored copy.
    /// Never restores an old snapshot over a newer refresh (FR-21).
    ///
    /// `Ok(true)`: stored. `Ok(false)`: not needed (not registered, same tokens, or the stored
    /// copy is newer). `Err`: it should have been stored but could not be (or the stored copy
    /// could not be read to compare): the login in the environment may be the only fresh copy.
    pub fn capture(&self, inner: &mut Inner, id: &AccountId, login: &Login) -> Result<bool, String> {
        if inner.config.account(id).is_none() {
            return Ok(false);
        }
        let loaded = self.vault.load(id.as_str());
        if loaded.is_ok() {
            clear_vault_error(inner, id);
        }
        match loaded {
            Ok(stored) if stored.same_tokens(login) => Ok(false),
            Ok(stored) if stored.expires_at().unwrap_or(0) > login.expires_at().unwrap_or(0) => Ok(false),
            Ok(_) | Err(qf_vault::VaultError::NotFound) => match self.vault.store(id.as_str(), login) {
                Ok(()) => Ok(true),
                Err(e) => Err(format!("protected copy not stored: {e}")),
            },
            Err(e) => {
                if let Some(a) = inner.config.account_mut(id) {
                    a.health = Health::VaultError;
                }
                self.log.log(
                    "error",
                    "vault.read_failed",
                    json!({"account": id, "error": e.to_string()}),
                    &[],
                );
                Err(format!("protected copy unreadable: {e}"))
            }
        }
    }

    /// Whose login an environment's files hold, judged by the credentials and not only by the
    /// account record in `.claude.json`: when the refresh token is the one stored for another
    /// registered account, the files are mixed (a half-written switch).
    pub fn live_owner(&self, inner: &Inner, login: &Login) -> LiveOwner {
        let claimed = AccountId(login.account_hash().unwrap_or_default());
        if let Some(rt) = login.refresh_token().filter(|r| !r.is_empty()) {
            for a in inner.config.accounts.iter().filter(|a| a.id != claimed) {
                if let Ok(stored) = self.vault.load(a.id.as_str()) {
                    if stored.refresh_token() == Some(rt) {
                        return LiveOwner::Mixed {
                            owner: a.id.clone(),
                            claimed,
                            login: Login {
                                oauth: login.oauth.clone(),
                                account: stored.account,
                            },
                        };
                    }
                }
            }
        }
        if inner.config.account(&claimed).is_some() {
            LiveOwner::Registered(claimed)
        } else {
            LiveOwner::Unknown(claimed)
        }
    }

    /// Hold an environment whose credentials belong to `owner` while `.claude.json` names
    /// `claimed`: nothing is stored from it under `claimed`, and automation waits for a switch.
    pub fn hold_mixed(&self, inner: &mut Inner, env: &EnvId, owner: &AccountId, claimed: Option<&AccountId>) {
        let Some(st) = inner.envs.get_mut(env) else { return };
        let first = st.mixed.as_ref() != Some(owner);
        st.mixed = Some(owner.clone());
        st.live = None;
        st.unknown = None;
        st.signed_out = false;
        st.suggestion = None;
        st.pin = None;
        st.hold = Some(
            "the login files are inconsistent (credentials of one account, account record of another); switch to an account to repair them".into(),
        );
        if first {
            self.record(
                inner,
                "login.mixed",
                json!({"env": env, "credentials": owner, "record": claimed}),
                true,
            );
            let text = format!(
                "The login files here are inconsistent: the credentials are {}'s but the account record names {}. Quayflow stores no token from them and automation is paused. Switch to an account (quayflow use <account>) to repair them.",
                inner.config.label(owner),
                claimed.map(|c| inner.config.label(c)).unwrap_or_else(|| "another account".into())
            );
            self.emit(inner, "notice", Some(env), &text);
        }
    }

    /// Keep the outgoing login of `env` safe before it is overwritten or removed: stored under
    /// the account its credentials belong to, or as a protected backup when that is not known.
    /// An error means it is not safe and the overwrite must not happen.
    pub fn preserve_live(&self, inner: &mut Inner, env: &EnvId, login: &Login) -> Result<(), String> {
        let mixed = inner.envs.get(env).is_some_and(|s| s.mixed.is_some());
        let backup = |inner: &Inner, kind: &str, h: &AccountId| -> Result<(), String> {
            self.vault
                .backup(&self.dd.backups(), &format!("{kind}-{}", h.0), login)
                .map_err(|e| format!("cannot back up the {kind} live login: {e}"))?;
            self.record(
                inner,
                "login.backup",
                json!({"env": env, "account": h, "kind": kind}),
                true,
            );
            Ok(())
        };
        match self.live_owner(inner, login) {
            LiveOwner::Mixed {
                owner, login: fixed, ..
            } => self.capture(inner, &owner, &fixed).map(drop),
            // Credentials of the mixed state whose owner is no longer recognizable (refreshed
            // since): never stored under the account `.claude.json` names.
            LiveOwner::Registered(h) | LiveOwner::Unknown(h) if mixed => backup(inner, "mixed", &h),
            LiveOwner::Registered(h) => self.capture(inner, &h, login).map(drop),
            // Unknown login: keep a protected backup; inability blocks the overwrite (FR-04).
            LiveOwner::Unknown(h) => backup(inner, "unknown", &h),
        }
    }

    /// The account's protected copy loaded successfully: a vault error no longer applies.
    pub fn vault_ok(&self, id: &AccountId) {
        clear_vault_error(&mut self.lock(), id);
    }

    /// Accounts marked with a vault error: clear the mark when their copy loads again.
    pub fn recheck_vault_errors(&self) {
        let ids: Vec<AccountId> = self
            .lock()
            .config
            .accounts
            .iter()
            .filter(|a| a.health == Health::VaultError)
            .map(|a| a.id.clone())
            .collect();
        for id in ids {
            if self.vault.load(id.as_str()).is_ok() {
                self.vault_ok(&id);
            }
        }
    }

    // ---------------- observation ----------------

    pub fn observe(&self, force: bool) {
        if self.offline {
            return;
        }
        let now = now_ms();
        let jobs: Vec<(AccountId, Option<Env>)> = {
            let inner = self.lock();
            inner
                .config
                .accounts
                .iter()
                .filter(|a| a.health != Health::RefreshRejected)
                .filter(|a| (force && inner.schedule.may_force(&a.id, now)) || inner.schedule.due(&a.id, now))
                .map(|a| {
                    let live_env = inner
                        .envs
                        .values()
                        .find(|s| s.live.as_ref() == Some(&a.id) && s.unknown.is_none())
                        .map(|s| s.env.clone());
                    (a.id.clone(), live_env)
                })
                .collect()
        };
        let mut observed = false;
        for (id, live_env) in jobs {
            // The live account's freshest token is in the environment, not the vault.
            let login = match &live_env {
                Some(env) => qf_adapter::read_login(env)
                    .ok()
                    .flatten()
                    .filter(|l| l.account_hash().as_deref() == Some(id.as_str())),
                None => None,
            }
            .or_else(|| {
                let l = self.vault.load(id.as_str()).ok();
                if l.is_some() {
                    self.vault_ok(&id);
                }
                l
            });
            let Some(mut login) = login else {
                let mut inner = self.lock();
                if let Some(a) = inner.config.account_mut(&id) {
                    a.health = Health::VaultError;
                }
                continue;
            };
            // An idle account's expired access token is refreshed in its saved copy, so its
            // usage stays readable; a live one is Claude's to refresh.
            if live_env.is_none() && needs_refresh(&login, now_ms()) {
                if let Some(fresh) = self.refresh_idle(&id) {
                    login = fresh;
                }
            }
            let token = login.access_token().unwrap_or_default().to_string();
            let r = qf_provider::fetch_usage(&id, &token, login.expires_at(), now_ms());
            let mut inner = self.lock();
            let t = inner.config.tuning.clone();
            match r {
                Ok(o) => {
                    let live = live_env.is_some();
                    inner
                        .schedule
                        .on_success(&id, now_ms(), live, t.live_poll_s, t.idle_poll_s);
                    merge_observation(&mut inner.observations, o, false);
                    observed = true;
                    if let Some(a) = inner.config.account_mut(&id) {
                        if a.health == Health::StaleAccess {
                            a.health = Health::Ok;
                        }
                    }
                }
                Err(e) => {
                    if e == ProviderError::Expired && live_env.is_none() {
                        if let Some(a) = inner.config.account_mut(&id) {
                            if a.health == Health::Ok {
                                a.health = Health::StaleAccess;
                            }
                        }
                    }
                    inner
                        .schedule
                        .on_error(&id, now_ms(), &e, t.live_poll_s, t.backoff_max_s);
                    // A single 429 is the endpoint pacing a shared budget (handled by the
                    // slower pace); only 429s in a row, or other errors, are warnings.
                    let limited = matches!(e, ProviderError::Status { code: 429, .. });
                    let level = if limited && !inner.schedule.repeated_limit(&id) {
                        "info"
                    } else {
                        "warn"
                    };
                    self.log.log(
                        level,
                        "usage.error",
                        json!({"account": id, "error": e.to_string()}),
                        &[],
                    );
                }
            }
            self.save_observations(&inner);
        }
        // Status lines and panes redraw from the state only when an event arrives.
        if observed {
            self.signal("usage");
        }
    }

    /// Refreshes the saved login of an idle account whose access token has expired.
    ///
    /// Safe only when the vault holds the only copy of that refresh token (the service may
    /// rotate it): under `cred_lock`, which every switch holds while it writes a login, the
    /// files of every environment are read and the refresh is skipped when any of them holds
    /// the same refresh token, or cannot be read. The new login is stored before it is used;
    /// a refused refresh token marks the account for a new sign-in. Returns the stored login.
    pub fn refresh_idle(&self, id: &AccountId) -> Option<Login> {
        let _cred = self.cred_lock.lock().unwrap_or_else(|e| e.into_inner());
        let login = self.vault.load(id.as_str()).ok()?;
        if !needs_refresh(&login, now_ms()) {
            // Stored meanwhile (a capture of Claude's own refresh, or another refresh).
            return Some(login);
        }
        let rt = login.refresh_token().filter(|r| !r.is_empty())?.to_string();
        let envs: Vec<Env> = self.lock().envs.values().map(|s| s.env.clone()).collect();
        for env in &envs {
            match qf_adapter::read_login(env) {
                Ok(Some(l)) if l.refresh_token() == Some(rt.as_str()) => return None,
                Ok(_) => {}
                // Unreadable (mid-write, say): this copy could be live there. Not now.
                Err(_) => return None,
            }
        }
        let result = qf_provider::refresh_access(&rt, &login.scopes(), now_ms());
        let mut inner = self.lock();
        match result {
            Ok(r) => {
                let fresh = login.with_tokens(&r.access_token, &r.refresh_token, r.expires_at, r.scopes.as_deref());
                let stored = self
                    .vault
                    .store(id.as_str(), &fresh)
                    .or_else(|_| self.vault.store(id.as_str(), &fresh));
                match stored {
                    Ok(()) => {
                        self.record(&inner, "login.refreshed", json!({"account": id}), true);
                        if let Some(a) = inner.config.account_mut(id) {
                            if a.health == Health::StaleAccess {
                                a.health = Health::Ok;
                            }
                        }
                        Some(fresh)
                    }
                    Err(e) => {
                        // The service may have retired the old refresh token: this login
                        // needs a new sign-in.
                        if let Some(a) = inner.config.account_mut(id) {
                            a.health = Health::VaultError;
                        }
                        self.log.log(
                            "error",
                            "vault.refresh_store_failed",
                            json!({"account": id, "error": e.to_string()}),
                            &[],
                        );
                        let text = format!(
                            "{}'s refreshed login could not be saved ({e}). Sign in to it again (/quay → Accounts → Sign in again).",
                            inner.config.label(id)
                        );
                        self.emit(&mut inner, "notice", None, &text);
                        None
                    }
                }
            }
            Err(qf_provider::RefreshError::Rejected(m)) => {
                if let Some(a) = inner.config.account_mut(id) {
                    a.health = Health::RefreshRejected;
                }
                self.record(
                    &inner,
                    "login.refresh_rejected",
                    json!({"account": id, "reason": m}),
                    true,
                );
                let text = format!(
                    "{}'s saved login was refused by the sign-in service. Sign in to it again (/quay → Accounts → Sign in again).",
                    inner.config.label(id)
                );
                self.emit(&mut inner, "notice", None, &text);
                None
            }
            Err(qf_provider::RefreshError::Failed(e)) => {
                self.log.log(
                    "warn",
                    "token.refresh_failed",
                    json!({"account": id, "error": e.to_string()}),
                    &[],
                );
                None
            }
        }
    }

    /// Fold a client's rate-limit view into the live account's observation.
    pub fn session_observation(&self, inner: &mut Inner, account: &AccountId, limits: &[api::SessionRateLimit]) {
        let windows: Vec<Window> = limits
            .iter()
            .filter_map(|l| {
                let pct = l.percent_used?;
                let key = match l.kind.as_deref()? {
                    "five_hour" => "5h".to_string(),
                    "seven_day" => "7d".to_string(),
                    k if k.starts_with("seven_day_") => format!("7d:{}", capitalize(&k["seven_day_".len()..])),
                    _ => return None,
                };
                let resets_at = match &l.resets_at {
                    Some(Value::Number(n)) => n.as_i64().map(|v| if v < 10_000_000_000 { v * 1000 } else { v }),
                    Some(Value::String(s)) => qf_model::time::parse_rfc3339_ms(s),
                    _ => None,
                };
                Some(Window { key, pct, resets_at })
            })
            .collect();
        if windows.is_empty() {
            return;
        }
        merge_observation(
            &mut inner.observations,
            Observation {
                account: account.clone(),
                ts: now_ms(),
                windows,
                source: "session".into(),
            },
            true,
        );
    }

    // ---------------- decisions ----------------

    pub fn tick(self: &Arc<Self>) {
        self.watch();
        let ids: Vec<EnvId> = self.lock().envs.keys().cloned().collect();
        if cfg!(target_os = "macos") {
            // A switch on a Keychain environment leaves a marker so running sessions re-read at
            // once; it goes when every session has read the new login anyway (also leftovers of
            // an earlier engine run).
            let envs: Vec<qf_adapter::Env> = {
                let inner = self.lock();
                ids.iter()
                    .filter_map(|id| inner.envs.get(id).map(|s| s.env.clone()))
                    .collect()
            };
            for env in envs {
                if qf_adapter::login::sweep_marker(&env, qf_adapter::login::MARKER_LIFETIME) {
                    self.log.log("info", "marker.removed", json!({"env": env.id}), &[]);
                }
            }
        }
        {
            let mut inner = self.lock();
            for id in &ids {
                self.refresh_env(&mut inner, id);
                self.finish_adoption(&mut inner, id);
            }
            prune_session_state(&mut inner);
            self.expire_signins(&mut inner);
        }
        self.recheck_vault_errors();
        self.observe(false);
        for id in ids {
            self.decide(&id);
        }
    }

    pub fn finish_adoption(&self, inner: &mut Inner, env: &EnvId) {
        let Some(st) = inner.envs.get_mut(env) else { return };
        let Some(t) = st.transition.as_mut() else { return };
        if t.state != TransitionState::VerifyingAdoption {
            return;
        }
        let tid = t.id.clone();
        let pending = st
            .adoption
            .iter()
            .any(|a| a.transition == tid && a.state == AdoptionState::PendingNextRequest);
        let timed_out = t.written_at.is_some_and(|w| now_ms() - w > 30 * 60_000);
        if !pending || timed_out {
            let mismatch = st
                .adoption
                .iter()
                .any(|a| a.transition == tid && a.state == AdoptionState::Mismatch);
            t.state = if mismatch {
                TransitionState::Uncertain
            } else {
                TransitionState::Completed
            };
            t.ended_at = Some(now_ms());
            if timed_out && pending {
                t.detail = Some("some clients made no request yet; they adopt at their next request".into());
            }
            let t = t.clone();
            st.last_switch = Some(t.clone());
            self.record(
                inner,
                "transition.completed",
                json!({"id": t.id, "env": env, "state": t.state}),
                false,
            );
        }
    }

    /// Clear the pin. A pinning switch still in flight is marked so that it does not pin again
    /// when it completes.
    pub fn unpin(&self, env: &EnvId) {
        let mut inner = self.lock();
        if let Some(st) = inner.envs.get_mut(env) {
            st.pin = None;
            if st.blocking_transition() {
                st.unpinned_transition = st.transition.as_ref().map(|t| t.id.clone());
            }
        }
        self.record(&inner, "unpin", json!({"env": env}), false);
        self.emit(
            &mut inner,
            "state",
            Some(env),
            "Pin cleared; suggestions and automatic switching resume.",
        );
    }

    pub fn decide(self: &Arc<Self>, env: &EnvId) {
        let mut inner = self.lock();
        let Some(st) = inner.envs.get(env) else { return };
        if let Some(why) = st.pause_reason(&inner.config) {
            let st = inner.envs.get_mut(env).expect("env");
            st.decision = format!("automation paused: {why}");
            if st.suggestion.as_ref().is_some_and(|s| s.how != SwitchHow::Recovery) {
                st.suggestion = None;
            }
            return;
        }
        let governing = Engine::governing(&inner, env);
        let mode = qf_policy::effective_mode(&governing);
        let Some(ctx) = Engine::ctx(&inner, env, None) else {
            return;
        };
        let decision = qf_policy::proactive(&ctx);
        drop(ctx);
        match decision {
            qf_policy::Proactive::None(why) => {
                let st = inner.envs.get_mut(env).expect("env");
                st.decision = why;
                if st
                    .suggestion
                    .as_ref()
                    .is_some_and(|s| s.how != SwitchHow::Recovery || now_ms() - s.ts > 15 * 60_000)
                {
                    st.suggestion = None;
                }
            }
            qf_policy::Proactive::Switch {
                to,
                reason,
                return_home,
            } => {
                let how = if return_home {
                    SwitchHow::ReturnHome
                } else {
                    SwitchHow::Automatic
                };
                match mode {
                    Mode::Manual => {
                        inner.envs.get_mut(env).expect("env").decision = format!("Manual mode: {reason}");
                    }
                    Mode::Suggest => {
                        let st = inner.envs.get(env).expect("env");
                        if st.suggestion.as_ref().map(|s| &s.to) != Some(&to) {
                            let from = st.live.clone();
                            let s = Suggestion {
                                id: random_hex(6),
                                ts: now_ms(),
                                env: env.clone(),
                                from,
                                to: to.clone(),
                                how: if return_home {
                                    SwitchHow::ReturnHome
                                } else {
                                    SwitchHow::Accepted
                                },
                                reason: reason.clone(),
                            };
                            self.record(
                                &inner,
                                "suggest",
                                json!({"env": env, "to": to, "reason": reason}),
                                false,
                            );
                            let text = format!(
                                "Suggest switching to {}: {reason}. /quay accept",
                                inner.config.label(&to)
                            );
                            let st = inner.envs.get_mut(env).expect("env");
                            st.suggestion = Some(s);
                            self.emit(&mut inner, "suggestion", Some(env), &text);
                        }
                        inner.envs.get_mut(env).expect("env").decision = format!("suggesting: {reason}");
                    }
                    Mode::Automatic => {
                        inner.envs.get_mut(env).expect("env").decision = reason.clone();
                        drop(inner);
                        let r = self.start_switch(env, &to, how, &reason, true, false);
                        if let Err(e) = r {
                            if let Some(s) = self.lock().envs.get_mut(env) {
                                s.decision = format!("automatic switch not started: {}", e.message);
                            }
                        }
                    }
                }
            }
        }
    }

    fn expire_signins(&self, inner: &mut Inner) {
        let now = now_ms();
        let stale: Vec<String> = inner
            .signins
            .values()
            .filter(|s| now - s.started_at > 30 * 60_000 && s.state != "saved")
            .map(|s| s.id.clone())
            .collect();
        for id in stale {
            if let Some(s) = inner.signins.remove(&id) {
                retire_signin(&s.env);
            }
        }
        // Saved sign-ins are answered for a while, then forgotten.
        inner
            .signins
            .retain(|_, s| !(s.state == "saved" && now - s.started_at > 30 * 60_000));
        // Folders left by an earlier retire that failed (or an earlier engine run): retire
        // their login, then remove them.
        if let Ok(rd) = std::fs::read_dir(self.dd.signin_dir()) {
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().into_owned();
                let old = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| m.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(30 * 60));
                if old && !inner.signins.contains_key(&id) && e.path().is_dir() {
                    retire_signin(&qf_adapter::Env::at(EnvId(format!("signin-{id}")), e.path()));
                }
            }
        }
    }

    pub fn wait_events(&self, since: u64, wait: Duration) -> api::EventsResponse {
        let mut ev = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if ev.seq <= since && !wait.is_zero() {
            let (g, _) = self
                .events_cv
                .wait_timeout_while(ev, wait, |e| e.seq <= since && !self.shutdown.load(Ordering::SeqCst))
                .unwrap_or_else(|e| e.into_inner());
            ev = g;
        }
        api::EventsResponse {
            seq: ev.seq,
            events: ev.list.iter().filter(|e| e.seq > since).cloned().collect(),
        }
    }
}

/// A Claude version usable as a file name: `[0-9A-Za-z.+-]`, at most 64 characters, starting
/// with a digit (so never a dot or a reserved device name like `CON`), no `..`.
pub fn safe_version_name(v: &str) -> bool {
    v.len() <= 64
        && v.starts_with(|c: char| c.is_ascii_digit())
        && !v.contains("..")
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

/// Forget per-session state of sessions no longer running in any environment.
fn prune_session_state(inner: &mut Inner) {
    let alive: std::collections::HashSet<String> = inner
        .envs
        .values()
        .flat_map(|st| st.sessions.iter().map(|s| s.session_id.clone()))
        .collect();
    inner.session_model.retain(|sid, _| alive.contains(sid));
    // A handshake can come before the session shows in its registry: keep it a while.
    let now = now_ms();
    inner
        .session_keychain
        .retain(|sid, (_, _, at)| alive.contains(sid) || now - *at < 10 * 60_000);
    inner.progressed.retain(|sid, _| alive.contains(sid));
    inner.verified.retain(|sid, _| alive.contains(sid));
}

/// A saved login whose access token has expired or expires within five minutes.
pub fn needs_refresh(login: &Login, now: i64) -> bool {
    login.expires_at().is_some_and(|e| e < now + 5 * 60_000) && login.refresh_token().is_some_and(|r| !r.is_empty())
}

fn clear_vault_error(inner: &mut Inner, id: &AccountId) {
    if let Some(a) = inner.config.account_mut(id) {
        if a.health == Health::VaultError {
            a.health = Health::Ok;
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Keep one observation per account; a session view overrides the windows it carries
/// and keeps the engine's per-model windows.
pub fn merge_observation(list: &mut Vec<Observation>, o: Observation, partial: bool) {
    match list.iter_mut().find(|x| x.account == o.account) {
        Some(existing) if partial => {
            let mut windows = o.windows.clone();
            for w in &existing.windows {
                if !windows.iter().any(|n| n.key == w.key) {
                    windows.push(w.clone());
                }
            }
            existing.windows = windows;
            existing.ts = o.ts;
            existing.source = o.source;
        }
        Some(existing) => *existing = o,
        None => list.push(o),
    }
}

pub fn load_config(dd: &DataDir) -> Result<Config, String> {
    match std::fs::read_to_string(dd.config()) {
        Ok(t) => {
            let c: Config = serde_json::from_str(&t).map_err(|e| format!("config.json is not valid: {e}"))?;
            c.validate().map_err(|e| format!("config.json: {e}"))?;
            Ok(c)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let c = Config::default();
            let text = serde_json::to_string_pretty(&c).map_err(|e| e.to_string())?;
            qf_vault::atomic_write(&dd.config(), text.as_bytes()).map_err(|e| e.to_string())?;
            Ok(c)
        }
        Err(e) => Err(format!("config.json: {e}")),
    }
}

pub fn lane_meta(dd: &DataDir, name: &str) -> Value {
    std::fs::read_to_string(dd.lanes().join(name).join(".quayflow-lane.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

pub fn installed_claude_version() -> Option<String> {
    let dir = qf_adapter::env::home_dir()
        .join(".local")
        .join("share")
        .join("claude")
        .join("versions");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .max_by(|a, b| compare_versions(a, b))
}

pub fn static_check(version: &str) -> CompatRecord {
    let mut rec = CompatRecord {
        version: version.into(),
        checked_at: now_ms(),
        static_pass: false,
        static_detail: vec![],
        live_pass: None,
        live_detail: None,
    };
    match qf_adapter::canary::version_binary(version) {
        None => rec.static_detail.push(format!(
            "binary for Claude {version} not found (native install expected)"
        )),
        Some(p) => match qf_adapter::canary::scan(&p) {
            Ok(found) => {
                rec.static_pass = found.iter().all(|(_, ok)| *ok);
                rec.static_detail = found
                    .iter()
                    .filter(|(_, ok)| !ok)
                    .map(|(l, _)| format!("missing {l}"))
                    .collect();
            }
            Err(e) => rec.static_detail.push(format!("binary unreadable: {e}")),
        },
    }
    rec
}

/// Retire a sign-in's temporary login (no sign-out call), then remove its folder. When the
/// login cannot be removed (a lock, a Keychain that does not answer), the folder is kept, so
/// the login can still be found and retired later instead of staying behind unreachable.
pub(crate) fn retire_signin(env: &qf_adapter::Env) {
    if qf_adapter::login::retire(env, Duration::from_secs(3)).is_ok() {
        let _ = std::fs::remove_dir_all(&env.config_dir);
    }
}
