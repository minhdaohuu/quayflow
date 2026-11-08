//! `quayflow`: the Quayflow engine and command line.

/// `println!` that keeps piped output readable in Windows PowerShell (see `term`).
macro_rules! outln {
    () => { println!() };
    ($($arg:tt)*) => { println!("{}", crate::term::fit(&format!($($arg)*))) };
}

mod canary;
mod client;
mod last;
mod local;
mod print;
mod term;

use client::{Client, ClientError};
use qf_engine::DataDir;
use serde_json::{json, Value};
use std::time::Duration;

const HELP: &str =
    "Quayflow: switch Claude Code between your approved accounts by usage, and recover from usage limits.

Usage: quayflow <command> [options]

Engine
  start [--if-consented]       start the engine in the background (no window)
  stop | pause | resume        stop the engine; pause or resume automation
  status [--json]              show environments, accounts, usage and decisions
  last --session ID [--json]   show what Quayflow did for one Claude session: recovered, every
                               account at its limit, or why not, and when to run again
  serve                        run the engine in the foreground

Accounts and groups
  onboard                      give the first-use consents (interactive)
  register --label L --group G [--accept-disclosure]
                               register the login that is live now
  add --label L --group G [--accept-disclosure]
                               sign in to another account (opens a window)
  reauth <account>             sign in again for an account that was set aside
  remove <account> [--yes]     remove an account and its saved copy
  label <account> <new label>  rename an account
  use <account> [--idle-wait] [--env E]
                               switch now and pin your choice
  accept                       make the suggested switch
  unpin                        clear your pin
  mode <manual|suggest|automatic> [--group G] [--approve]
                               set a group's mode (Automatic needs --approve once)
  group list | create G | delete G | default G
  group add G <account..> [--accept-disclosure] | remove G <account..>
  group preferred G <account|none> | rank G <account> <preferred|normal|backup>
  group tuning G [--threshold N] [--margin N]
  project list | assign <path> <G> | unassign <path>
                               choose the group a project folder uses
  unknown ignore [--env E]     acknowledge an unregistered live login (automation stays paused)

Lanes and sessions
  lanes list | create N | check N | delete N
                               separate Claude environments with their own login
  handoff prepare|open|replace --session ID --to ENV
                               move a session and its history to another environment
  shell install | uninstall | refresh
                               PowerShell prompt hook for same-terminal Replace (Windows)

Maintenance
  canary [--live]              check this Claude version's compatibility; --live also runs a
                               test job that switches accounts (needs the live-canary consent)
  policy [--json]              show managed login enforcement per environment
  report [--preview]           save a support report locally to review (--preview: print only)
  migrate-poc [--group G] [--accept-disclosure]
  consent <engine-start|signin-startup|credential-storage|terms|prompt-hook|live-canary> <on|off>
  update [--check] [--version vX.Y.Z | --source DIR] [--skip-attestation]
                               install a newer release after verifying it
  uninstall [--delete-vault] [--delete-logs] [--delete-data] [--yes]
  version                      print the version
";

struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
    /// Usage errors found while parsing (a value-taking flag without a value, ...).
    errors: Vec<String>,
}

/// Every option the commands read. Anything else is refused: a misspelled `--check` or
/// `--if-consented` must not run the command without it.
fn known_flag(name: &str) -> bool {
    takes_value(name)
        || matches!(
            name,
            "accept-disclosure"
                | "approve"
                | "check"
                | "delete-data"
                | "delete-logs"
                | "delete-vault"
                | "help"
                | "idle-wait"
                | "if-consented"
                | "json"
                | "live"
                | "preview"
                | "skip-attestation"
                | "yes"
        )
}

fn takes_value(name: &str) -> bool {
    matches!(
        name,
        "label"
            | "group"
            | "env"
            | "session"
            | "to"
            | "threshold"
            | "margin"
            | "pid"
            | "wait-pid"
            | "cwd"
            | "version"
            | "source"
    )
}

impl Args {
    fn parse(raw: Vec<String>) -> Args {
        let mut pos = vec![];
        let mut flags = vec![];
        let mut errors = vec![];
        let mut it = raw.into_iter().peekable();
        while let Some(a) = it.next() {
            let Some(flag) = a.strip_prefix("--") else {
                pos.push(a);
                continue;
            };
            let (name, inline) = match flag.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (flag.to_string(), None),
            };
            if !takes_value(&name) {
                if inline.is_some() {
                    errors.push(format!("--{name} does not take a value"));
                }
                flags.push((name, None));
                continue;
            }
            // A value never starts with "--": `--label --group G` must not make "--group" a label.
            let val = match inline {
                Some(v) => Some(v),
                None => it.next_if(|n| !n.starts_with("--")),
            };
            match val {
                Some(v) if !v.is_empty() && !v.starts_with("--") => flags.push((name, Some(v))),
                _ => {
                    errors.push(format!("--{name} needs a value"));
                    flags.push((name, None));
                }
            }
        }
        Args { pos, flags, errors }
    }
    fn has(&self, f: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == f)
    }
    fn val(&self, f: &str) -> Option<String> {
        self.flags.iter().find(|(n, _)| n == f).and_then(|(_, v)| v.clone())
    }
    fn p(&self, i: usize) -> Option<&str> {
        self.pos.get(i).map(|s| s.as_str())
    }
}

/// `0.1.0` -> `v0.1.0`; refuses anything that is not a release tag (as the installer does).
fn normalize_version(v: &str) -> Result<String, String> {
    let v = v.trim();
    let tag = if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    };
    let (core, suffix) = match tag[1..].find(['-', '+']) {
        Some(i) => (&tag[1..1 + i], &tag[2 + i..]),
        None => (&tag[1..], ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let core_ok = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let suffix_ok = tag[1..].find(['-', '+']).is_none()
        || (!suffix.is_empty()
            && suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'));
    if core_ok && suffix_ok {
        Ok(tag)
    } else {
        Err(format!("not a release version: \"{v}\" (expected vX.Y.Z)"))
    }
}

fn fail(msg: impl std::fmt::Display) -> i32 {
    eprintln!("quayflow: {}", term::fit(&msg.to_string()));
    1
}

fn connect(dd: &DataDir) -> Result<Client, ClientError> {
    Client::connect(dd)
}

fn say(v: Result<Value, ClientError>) -> i32 {
    match v {
        Ok(v) => {
            if v.get("ok") == Some(&json!(false)) {
                outln!("{}", v["message"].as_str().unwrap_or("refused"));
                return 1;
            } else if let Some(m) = v["message"].as_str() {
                outln!("{m}");
            } else {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            0
        }
        Err(ClientError::Api { code, message, .. }) if code == "needs_disclosure" => {
            outln!("{message}\n\nTo accept this once for the group, run the command again with --accept-disclosure.");
            2
        }
        Err(ClientError::Api { code, message, .. }) if code == "needs_approval" => {
            outln!("{message}\n\nTo turn it on for this group, run the command again with --approve.");
            2
        }
        Err(e) => fail(e),
    }
}

fn ask_yes(q: &str) -> bool {
    use std::io::Write;
    print!("{q} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    matches!(s.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn switch_result(v: Result<Value, ClientError>) -> i32 {
    match v {
        Ok(v) if v["ok"] == json!(true) => {
            let t = &v["transition"];
            outln!(
                "Switch requested ({}). Each session uses the new account from its next request.",
                t["state"].as_str().unwrap_or("?")
            );
            0
        }
        Ok(v) => {
            outln!("{}", v["message"].as_str().unwrap_or("refused"));
            if let Some(b) = v["blockedBy"].as_object() {
                outln!(
                    "Blocking session: {} (pid {}). Move it into a lane: quayflow handoff replace --session {} --to <lane>",
                    b.get("name").and_then(|x| x.as_str()).unwrap_or("?"),
                    b.get("pid").and_then(|x| x.as_u64()).unwrap_or(0),
                    b.get("sessionId").and_then(|x| x.as_str()).unwrap_or("?")
                );
            }
            1
        }
        Err(e) => fail(e),
    }
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let a = Args::parse(raw);
    let dd = DataDir::from_env();
    let code = run(&a, &dd);
    std::process::exit(code);
}

fn run(a: &Args, dd: &DataDir) -> i32 {
    // `quayflow --version` and `quayflow --help` behave like the subcommands.
    let cmd = match a.p(0) {
        Some(c) => c,
        None if a.has("version") => "version",
        None => "help",
    };
    if !matches!(cmd, "help" | "-h" | "/?" | "version" | "-V") {
        if let Some(e) = a.errors.first() {
            return fail(e);
        }
        if let Some((f, _)) = a.flags.iter().find(|(f, _)| !known_flag(f)) {
            return fail(format!("unknown option --{f} (quayflow help lists the options)"));
        }
    }
    match cmd {
        "help" | "-h" | "/?" => {
            outln!("{HELP}");
            0
        }
        "version" | "-V" => {
            outln!(
                "quayflow {} (protocol {})",
                qf_model::ENGINE_VERSION,
                qf_model::PROTOCOL
            );
            0
        }
        "serve" => match qf_engine::serve(dd.clone(), qf_adapter::Env::default_env(), Duration::from_secs(2)) {
            Ok(true) => 0,
            Ok(false) => {
                outln!("A Quayflow engine is already running for this user.");
                0
            }
            Err(e) => fail(e),
        },
        "start" => local::start(dd, a.has("if-consented")),
        "last" => last_cmd(dd, a),
        "stop" => {
            let r = connect(dd).and_then(|c| c.post("/v1/engine", json!({"action": "stop"})));
            say(r)
        }
        "pause" | "resume" => say(connect(dd).and_then(|c| c.post("/v1/engine", json!({"action": cmd})))),
        "status" => match connect(dd).and_then(|c| c.get("/v1/state")) {
            Ok(v) if a.has("json") => {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                0
            }
            Ok(v) => {
                print::status(&v);
                0
            }
            Err(e) => fail(e),
        },
        "onboard" => onboard(dd),
        "register" => {
            let (Some(label), Some(group)) = (a.val("label"), a.val("group")) else {
                return fail("usage: quayflow register --label L --group G");
            };
            let r = connect(dd).and_then(|c| {
                c.post(
                    "/v1/accounts/register",
                    json!({"label": label, "group": group, "env": a.val("env"), "acceptDisclosure": a.has("accept-disclosure")}),
                )
            });
            match r {
                Ok(v) => {
                    outln!(
                        "{}",
                        v["note"].as_str().map(String::from).unwrap_or_else(|| format!(
                            "Registered {} in group \"{group}\". Nothing was switched.",
                            v["label"].as_str().unwrap_or(&label)
                        ))
                    );
                    0
                }
                Err(e) => say(Err(e)),
            }
        }
        "add" | "reauth" => signin(dd, a),
        "remove" => {
            let Some(acct) = a.p(1) else {
                return fail("usage: quayflow remove <account> [--yes]");
            };
            if !a.has("yes")
                && !ask_yes(&format!(
                    "Remove {acct} from Quayflow (its protected copy is deleted; history stays)?"
                ))
            {
                outln!("Cancelled; nothing was removed.");
                return 1;
            }
            say(connect(dd).and_then(|c| c.post("/v1/accounts/remove", json!({"account": acct}))))
        }
        "label" => {
            let (Some(acct), Some(l)) = (a.p(1), a.p(2)) else {
                return fail("usage: quayflow label <account> <new label>");
            };
            say(connect(dd).and_then(|c| c.post("/v1/accounts/label", json!({"account": acct, "label": l}))))
        }
        "use" => {
            let Some(to) = a.p(1) else {
                return fail("usage: quayflow use <account> [--idle-wait] [--env E]");
            };
            switch_result(connect(dd).and_then(|c| {
                c.post(
                    "/v1/switch",
                    json!({"to": to, "env": a.val("env"), "idleWait": a.has("idle-wait")}),
                )
            }))
        }
        "accept" => switch_result(connect(dd).and_then(|c| c.post("/v1/accept", json!({"env": a.val("env")})))),
        "unpin" => say(connect(dd).and_then(|c| c.post("/v1/unpin", json!({"env": a.val("env")})))),
        "mode" => {
            let Some(m) = a.p(1) else {
                return fail("usage: quayflow mode <manual|suggest|automatic> [--group G] [--approve]");
            };
            // No --group: the engine applies the configured default group.
            say(connect(dd).and_then(|c| {
                c.post(
                    "/v1/groups/mode",
                    json!({"group": a.val("group"), "mode": m, "approveAutomatic": a.has("approve")}),
                )
            }))
        }
        "group" => group_cmd(dd, a),
        "project" => match a.p(1) {
            Some("assign") => {
                let (Some(p), Some(g)) = (a.p(2), a.p(3)) else {
                    return fail("usage: quayflow project assign <path> <group>");
                };
                let p = std::path::absolute(p)
                    .map(|x| x.display().to_string())
                    .unwrap_or_else(|_| p.into());
                say(connect(dd).and_then(|c| c.post("/v1/projects/assign", json!({"path": p, "group": g}))))
            }
            Some("unassign") => {
                let Some(p) = a.p(2) else {
                    return fail("usage: quayflow project unassign <path>");
                };
                let p = std::path::absolute(p)
                    .map(|x| x.display().to_string())
                    .unwrap_or_else(|_| p.into());
                say(connect(dd).and_then(|c| c.post("/v1/projects/unassign", json!({"path": p}))))
            }
            _ => match connect(dd).and_then(|c| c.get("/v1/state")) {
                Ok(v) => {
                    outln!(
                        "Unassigned projects use \"{}\".",
                        v["defaultGroup"].as_str().unwrap_or("?")
                    );
                    for p in v["projects"].as_array().into_iter().flatten() {
                        outln!(
                            "  {}  ->  {}",
                            p["path"].as_str().unwrap_or(""),
                            p["group"].as_str().unwrap_or("")
                        );
                    }
                    0
                }
                Err(e) => fail(e),
            },
        },
        "unknown" => say(connect(dd).and_then(|c| {
            c.post(
                "/v1/unknown",
                json!({"action": a.p(1).unwrap_or("ignore"), "env": a.val("env")}),
            )
        })),
        "lanes" | "lane" => match a.p(1).unwrap_or("list") {
            "list" => match connect(dd).and_then(|c| c.get("/v1/lanes")) {
                Ok(v) => {
                    print::lanes(&v);
                    0
                }
                Err(e) => fail(e),
            },
            op @ ("create" | "check" | "delete") => {
                let Some(n) = a.p(2) else {
                    return fail(format!("usage: quayflow lanes {op} <name>"));
                };
                let r = connect(dd).and_then(|c| c.post(&format!("/v1/lanes/{op}"), json!({"name": n})));
                match r {
                    Ok(v) if op != "delete" => {
                        print::lanes(&json!([v]));
                        0
                    }
                    r => say(r),
                }
            }
            _ => fail("usage: quayflow lanes list | create <name> | check <name> | delete <name>"),
        },
        "handoff" => {
            let op = a.p(1).unwrap_or("");
            if op == "resume" {
                let wait = a.val("wait-pid").and_then(|p| p.parse().ok());
                return local::handoff_resume(
                    dd,
                    &a.val("session").unwrap_or_default(),
                    &a.val("to").unwrap_or_default(),
                    wait,
                    a.val("cwd").as_deref(),
                );
            }
            if !matches!(op, "prepare" | "open" | "replace") {
                return fail("usage: quayflow handoff prepare|open|replace --session ID --to ENV");
            }
            let r = connect(dd).and_then(|c| {
                c.post(
                    &format!("/v1/handoff/{op}"),
                    json!({"sessionId": a.val("session"), "toEnv": a.val("to").unwrap_or_default()}),
                )
            });
            match r {
                Ok(v) => {
                    outln!("{}", v["message"].as_str().unwrap_or(""));
                    outln!("Files copied and verified: {}", v["fileCount"]);
                    outln!("Command: {}", v["command"].as_str().unwrap_or(""));
                    0
                }
                Err(e) => fail(e),
            }
        }
        "shell" => match a.p(1) {
            Some("install") if !cfg!(windows) => {
                eprintln!(
                    "quayflow: the prompt hook is for PowerShell on Windows; on Linux and macOS use Open New Session in /quay"
                );
                1
            }
            Some("install") => match local::shell_install(dd, &local::profile_paths()) {
                Ok(r) => {
                    for e in &r.problems {
                        eprintln!("quayflow: {}", term::fit(&format!("prompt hook NOT installed: {e}")));
                    }
                    if r.done.is_empty() {
                        return 1;
                    }
                    let _ = local::set_consent(dd, "promptHook", true);
                    outln!(
                        "Prompt hook installed in:
  {}
Open a new PowerShell window to load it.",
                        r.done.join(
                            "
  "
                        )
                    );
                    let policy = local::ps51_execution_policy();
                    if !local::profiles_load(&policy) {
                        outln!("Note: Windows PowerShell 5.1's execution policy is {policy}, so it does not load profiles. To use the hook there, run in Windows PowerShell:
  Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
Quayflow does not change it for you. PowerShell 7 is not affected.");
                    }
                    i32::from(!r.problems.is_empty())
                }
                Err(e) => fail(e),
            },
            Some("uninstall") => {
                let r = local::shell_uninstall(dd, &local::profile_paths());
                for e in &r.problems {
                    eprintln!("quayflow: {}", term::fit(&format!("prompt hook NOT removed: {e}")));
                }
                if r.problems.is_empty() {
                    let _ = local::set_consent(dd, "promptHook", false);
                }
                let p = &r.done;
                outln!(
                    "Prompt hook removed{}.",
                    if p.is_empty() {
                        " (none found)".to_string()
                    } else {
                        format!(" from {}", p.join(", "))
                    }
                );
                i32::from(!r.problems.is_empty())
            }
            Some("claim") => {
                let pid = a.val("pid").and_then(|p| p.parse().ok()).unwrap_or(0);
                local::shell_claim(dd, pid)
            }
            Some("refresh") => match local::shell_refresh(dd) {
                Ok(true) => {
                    outln!("Shell module refreshed.");
                    0
                }
                Ok(false) => {
                    outln!("Shell module not installed, or already up to date.");
                    0
                }
                Err(e) => fail(e),
            },
            _ => fail("usage: quayflow shell install | uninstall | refresh"),
        },
        "canary" => {
            if a.has("live") {
                canary::live(dd)
            } else {
                match connect(dd).and_then(|c| c.post("/v1/compat/check", json!({}))) {
                    Ok(v) => {
                        print::compat(&v);
                        0
                    }
                    Err(e) => fail(e),
                }
            }
        }
        "policy" => match connect(dd).and_then(|c| c.get("/v1/policy")) {
            Ok(v) if a.has("json") => {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                0
            }
            Ok(v) => {
                for e in v["envs"].as_array().into_iter().flatten() {
                    let m = &e["managed"];
                    let enforced =
                        !m["forceOrg"].as_array().map(|x| x.is_empty()).unwrap_or(true) || !m["forceMethod"].is_null();
                    outln!(
                        "{}: {}",
                        e["env"].as_str().unwrap_or("?"),
                        if enforced {
                            let strs = |x: &Value| -> Vec<String> {
                                x.as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(Value::as_str)
                                    .map(String::from)
                                    .collect()
                            };
                            let orgs: Vec<String> = strs(&m["forceOrg"]).iter().map(|o| format!("#{o}")).collect();
                            format!(
                                "enforced by {} (organization {}; login method {})",
                                strs(&m["sources"]).join(", "),
                                if orgs.is_empty() { "any".into() } else { orgs.join(", ") },
                                m["forceMethod"].as_str().unwrap_or("any")
                            )
                        } else {
                            "no managed login enforcement".into()
                        }
                    );
                }
                outln!(
                    "Managed-login override setting: {}",
                    if v["dOrg"]["enabled"] == json!(true) {
                        "ON (recorded only; enforcement still respected)"
                    } else {
                        "off"
                    }
                );
                0
            }
            Err(e) => fail(e),
        },
        "report" if a.has("preview") => match connect(dd).and_then(|c| c.post("/v1/report/preview", json!({}))) {
            Ok(v) => {
                outln!(
                    "{}\n\nNothing was saved. Run quayflow report to save it locally for sharing.",
                    v["text"].as_str().unwrap_or("")
                );
                0
            }
            Err(e) => fail(e),
        },
        "report" => {
            match connect(dd).and_then(|c| c.post("/v1/report", json!({}))) {
                Ok(v) => {
                    outln!("{}\n...\nSaved for your review: {}\nNothing was sent anywhere. Share it only if you choose to.", v["preview"].as_str().unwrap_or(""), v["path"].as_str().unwrap_or(""));
                    0
                }
                Err(e) => fail(e),
            }
        }
        "migrate-poc" => say(connect(dd).and_then(|c| {
            c.post(
                "/v1/migrate-poc",
                json!({"group": a.val("group"), "acceptDisclosure": a.has("accept-disclosure")}),
            )
        })),
        "consent" => {
            let (Some(name), Some(state)) = (a.p(1), a.p(2)) else {
                return fail("usage: quayflow consent <name> <on|off>");
            };
            let Some(field) = local::consent_field(name) else {
                return fail(format!("unknown consent \"{name}\" (quayflow help lists them)"));
            };
            let on = match state {
                "on" => true,
                "off" => false,
                _ => return fail(format!("consent {name} takes on or off, not \"{state}\"")),
            };
            if let Err(e) = local::set_consent(dd, field, on) {
                return fail(e);
            }
            if field == "signinStartup" {
                if let Err(e) = local::set_startup(on) {
                    return fail(format!("startup entry: {e}"));
                }
            }
            outln!("{name}: {}", if on { "on" } else { "off" });
            0
        }
        "update" => {
            let version = match a.val("version").map(|v| normalize_version(&v)).transpose() {
                Ok(v) => v,
                Err(e) => return fail(e),
            };
            if version.is_some() && a.has("source") {
                return fail("--version and --source cannot be combined");
            }
            let source = a.val("source").map(std::path::PathBuf::from);
            match local::update(
                dd,
                a.has("check"),
                version.as_deref(),
                a.has("skip-attestation"),
                source.as_deref(),
            ) {
                Ok(m) => {
                    outln!("{m}");
                    0
                }
                Err(e) => fail(e),
            }
        }
        "uninstall" => {
            let delete =
                local::DeleteChoice::from_flags(a.has("delete-vault"), a.has("delete-logs"), a.has("delete-data"));
            let delete_data = delete.all;
            if delete_data {
                let busy = local::busy_lanes(dd);
                if !busy.is_empty() {
                    return fail(format!(
                        "Claude sessions are running in lane(s) {}; end them before deleting data. Nothing was changed.",
                        busy.join(", ")
                    ));
                }
            }
            let question = if delete_data {
                let lanes = local::lane_names(dd);
                let lanes_text = if lanes.is_empty() {
                    "no lanes exist".to_string()
                } else {
                    format!("lanes: {}", lanes.join(", "))
                };
                format!(
                    "Uninstall Quayflow and permanently delete:
  - its vault (the protected copies of your registered logins), logs and settings
  - every lane in {} ({lanes_text}): each is a separate Claude folder with its own login
    and its own conversation history, which is deleted with it
Your default Claude folder and its current login are not touched. Continue?",
                    dd.lanes().display()
                )
            } else {
                format!(
                    "Uninstall Quayflow for your user?
  Removed: the Mod, the startup entry, the prompt hook and the engine
  Left as it is: Claude's live login and its conversation history
  Protected vault copies: {}
  Routine logs and reports: {}
  Settings and lanes: kept
Continue?",
                    if delete.vault { "DELETED" } else { "kept" },
                    if delete.logs { "DELETED" } else { "kept" }
                )
            };
            if !a.has("yes") && !ask_yes(&question) {
                return 1;
            }
            match local::uninstall(dd, delete) {
                Ok(notes) => {
                    for n in notes {
                        outln!("- {n}");
                    }
                    0
                }
                Err(e) => fail(e),
            }
        }
        _ => {
            eprintln!("quayflow: unknown command \"{cmd}\"\n");
            outln!("{HELP}");
            2
        }
    }
}

/// `last --session ID [--json]`: read from the journal, so it works with the engine stopped.
fn last_cmd(dd: &DataDir, a: &Args) -> i32 {
    let Some(session) = a.val("session").or_else(|| a.p(1).map(str::to_string)) else {
        return fail("usage: quayflow last --session ID (the session_id in Claude's JSON output)");
    };
    let events: Vec<Value> = std::fs::read_to_string(dd.journal())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let labels = std::fs::read_to_string(dd.config())
        .ok()
        .and_then(|t| serde_json::from_str::<qf_model::Config>(t.trim_start_matches('\u{feff}')).ok())
        .map(|c| c.accounts.into_iter().map(|a| (a.id.0, a.label)).collect())
        .unwrap_or_default();
    let v = last::summarize(&events, &labels, &session);
    if a.has("json") {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    } else {
        outln!("{}", last::text(&v));
    }
    if v["found"] == true {
        0
    } else {
        last::NOT_FOUND
    }
}

fn group_cmd(dd: &DataDir, a: &Args) -> i32 {
    let op = a.p(1).unwrap_or("list");
    let g = a.p(2).unwrap_or("");
    let c = match connect(dd) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    match op {
        "list" => match c.get("/v1/state") {
            Ok(v) => {
                print::groups(&v);
                0
            }
            Err(e) => fail(e),
        },
        "create" | "delete" | "default" => say(c.post(&format!("/v1/groups/{op}"), json!({"group": g}))),
        "add" | "remove" => {
            let accts: Vec<String> = a.pos.iter().skip(3).cloned().collect();
            let key = if op == "add" { "add" } else { "remove" };
            match c.post(
                "/v1/groups/members",
                json!({"group": g, key: accts, "acceptDisclosure": a.has("accept-disclosure")}),
            ) {
                Ok(v) if v["needsDisclosure"] == json!(true) => {
                    outln!(
                        "{}\n\nTo accept this once for the group, run the command again with --accept-disclosure.",
                        v["disclosure"].as_str().unwrap_or("")
                    );
                    2
                }
                r => say(r.map(|_| json!({"message": format!("Group \"{g}\" membership updated.")}))),
            }
        }
        "preferred" => say(c.post("/v1/groups/preferred", json!({"group": g, "account": a.p(3)}))),
        "rank" => say(c.post(
            "/v1/groups/rank",
            json!({"group": g, "account": a.p(3), "rank": a.p(4)}),
        )),
        "tuning" => {
            let num = |f: &str| -> Result<Option<f64>, String> {
                match a.val(f) {
                    None if a.has(f) => Err(format!("--{f} needs a number")),
                    None => Ok(None),
                    Some(x) => match x.parse::<f64>() {
                        Ok(n) if n.is_finite() => Ok(Some(n)),
                        _ => Err(format!("--{f} must be a number, not \"{x}\"")),
                    },
                }
            };
            let (threshold, margin) = match (num("threshold"), num("margin")) {
                (Ok(t), Ok(m)) => (t, m),
                (Err(e), _) | (_, Err(e)) => return fail(e),
            };
            if threshold.is_none() && margin.is_none() {
                return fail("usage: quayflow group tuning G [--threshold N] [--margin N]");
            }
            say(c.post(
                "/v1/groups/tuning",
                json!({"group": g, "threshold": threshold, "margin": margin}),
            ))
        }
        _ => fail("usage: quayflow group list|create|delete|default|add|remove|preferred|rank|tuning (quayflow help shows each)"),
    }
}

fn signin(dd: &DataDir, a: &Args) -> i32 {
    let c = match connect(dd) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let reauth = (a.p(0) == Some("reauth")).then(|| a.p(1).unwrap_or("").to_string());
    if reauth.is_none() && (a.val("label").is_none() || a.val("group").is_none()) {
        return fail("usage: quayflow add --label L --group G [--accept-disclosure]");
    }
    let s = match c.post("/v1/accounts/signin/start", json!({"reauth": reauth})) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let id = s["id"].as_str().unwrap_or("").to_string();
    outln!(
        "A sign-in window opened. Sign in there with the account to {}.",
        if reauth.is_some() { "reauthenticate" } else { "add" }
    );
    if let Some(m) = s["message"].as_str() {
        outln!("{m}");
    }
    outln!(
        "If no window appeared, run this in another terminal:\n  {}",
        s["command"].as_str().unwrap_or("")
    );
    outln!("Waiting for the sign-in to finish (Ctrl+C cancels; your current login is not touched)...");
    let deadline = std::time::Instant::now() + Duration::from_secs(15 * 60);
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let st = match c.get(&format!("/v1/accounts/signin/status?id={id}")) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        if st["state"] == "ready" {
            let ident = &st["identity"];
            outln!(
                "Signed in: {} account #{}{}",
                ident["kind"].as_str().unwrap_or("?"),
                ident["account"].as_str().unwrap_or("?"),
                ident["display"].as_str().map(|d| format!(" ({d})")).unwrap_or_default()
            );
            if ident["unexpected"] == json!(true) {
                outln!("This is not the account being reauthenticated. Nothing was replaced.");
                let _ = c.post("/v1/accounts/signin/cancel", json!({"id": id}));
                return 1;
            }
            if let Some(d) = ident["duplicateOf"].as_str() {
                outln!("This account is already registered as \"{d}\". Its protected copy will be updated; it adds no capacity.");
            }
            if !ask_yes("Store this account in Quayflow's protected vault?") {
                let _ = c.post("/v1/accounts/signin/cancel", json!({"id": id}));
                outln!("Cancelled. Your current login is unchanged.");
                return 1;
            }
            let confirm = |accept: bool| {
                c.post(
                    "/v1/accounts/signin/confirm",
                    json!({"id": id, "label": a.val("label"), "group": a.val("group"), "acceptDisclosure": accept}),
                )
            };
            let mut r = confirm(a.has("accept-disclosure"));
            // A mixed group needs a one-time disclosure: ask here, so the sign-in is not lost.
            if let Err(ClientError::Api { code, message, .. }) = &r {
                if code == "needs_disclosure" {
                    outln!("{message}");
                    if ask_yes("Accept this once for the group?") {
                        r = confirm(true);
                    } else {
                        let _ = c.post("/v1/accounts/signin/cancel", json!({"id": id}));
                        outln!("Cancelled. Nothing was added; your current login is unchanged.");
                        return 1;
                    }
                }
            }
            return say(r);
        }
        if let Some(m) = st["message"].as_str() {
            if st["state"] == "waiting" && m.contains("verified") {
                outln!("{m}");
            }
        }
        if std::time::Instant::now() > deadline {
            let _ = c.post("/v1/accounts/signin/cancel", json!({"id": id}));
            return fail("the sign-in did not finish within 15 minutes; your current login is unchanged");
        }
    }
}

fn onboard(dd: &DataDir) -> i32 {
    outln!(
        "Quayflow keeps several Claude accounts and switches between them inside running sessions.

- The Mod (inside Claude) shows status and asks the engine to act. The engine (quayflow,
  one per user, no window) observes usage, decides, and swaps Claude's login in place.
- Logins are stored locally in a vault for your user account only (Windows: DPAPI;
  Linux and macOS: an owner-only file).
  Claude refreshes the live login; Quayflow refreshes only the saved copy of an idle account
  (so its usage stays readable) and sends tokens nowhere except Anthropic's own endpoints.
- A switch moves every session that uses the same Claude login, at each one's next request.
- Suggest mode is the default. Unattended headless jobs (claude -p) only switch in Automatic mode,
  which you enable per group.
- You are responsible for complying with the provider's terms and your organization's rules.
  Quayflow is not affiliated with or endorsed by Anthropic.
"
    );
    let items = [
        (
            "terms",
            "I understand and accept responsibility for the provider's terms and my organization's rules",
        ),
        (
            "engineStart",
            "Let Claude sessions start the Quayflow engine when it is not running",
        ),
        (
            "credentialStorage",
            "Let Quayflow store reusable copies of my Claude logins in its protected vault",
        ),
        ("signinStartup", "Start the engine when I sign in to this computer"),
    ];
    let mut consents = serde_json::Map::new();
    for (k, q) in items {
        consents.insert(k.into(), json!(ask_yes(q)));
    }
    if consents["terms"] != json!(true) {
        outln!(
            "Onboarding stopped: Quayflow needs the terms notice accepted. Run quayflow onboard again to accept it."
        );
        return 1;
    }
    let startup = consents["signinStartup"] == json!(true);
    let r = match connect(dd) {
        Ok(c) => c
            .post("/v1/onboarding", json!({"consents": consents, "complete": true}))
            .map_err(|e| e.to_string()),
        Err(_) => {
            for (k, v) in &consents {
                if let Err(e) = local::set_consent(dd, k, v == &json!(true)) {
                    return fail(e);
                }
            }
            Ok(json!({}))
        }
    };
    if let Err(e) = r {
        return fail(e);
    }
    if startup {
        if let Err(e) = local::set_startup(true) {
            eprintln!("quayflow: startup entry not created: {e}");
        }
    }
    outln!("Saved. Next, register the login that is live now: quayflow register --label <name> --group Default");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &[&str]) -> Args {
        Args::parse(s.iter().map(|x| x.to_string()).collect())
    }

    #[test]
    fn every_option_in_the_help_is_known_and_others_are_refused() {
        let mut named = vec![];
        for (i, _) in HELP.match_indices("--") {
            let name: String = HELP[i + 2..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                .collect();
            if !name.is_empty() {
                named.push(name);
            }
        }
        assert!(named.len() > 10, "{named:?}");
        for n in &named {
            assert!(known_flag(n), "--{n} is in the help but not accepted");
        }
        for typo in ["chek", "if-consnted", "skip-attestaton", "delete-dat"] {
            assert!(!known_flag(typo), "--{typo} must be refused");
        }
        assert_eq!(
            run(
                &parse(&["update", "--chek"]),
                &DataDir::at(std::path::Path::new("/nonexistent-qf"))
            ),
            1
        );
    }

    #[test]
    fn value_flags_never_swallow_the_next_flag() {
        let a = parse(&["add", "--label", "--group", "G"]);
        assert_eq!(a.val("label"), None);
        assert_eq!(a.val("group").as_deref(), Some("G"));
        assert_eq!(a.errors, vec!["--label needs a value"]);
        let a = parse(&["update", "--version"]);
        assert_eq!(a.errors, vec!["--version needs a value"]);
        let a = parse(&["register", "--label", "a", "--group", "Default", "--accept-disclosure"]);
        assert!(a.errors.is_empty());
        assert_eq!(a.val("label").as_deref(), Some("a"));
        assert!(a.has("accept-disclosure"));
        // Negative numbers are values, not flags.
        let a = parse(&["group", "tuning", "G", "--margin", "-5"]);
        assert_eq!(a.val("margin").as_deref(), Some("-5"));
        assert_eq!(a.pos, vec!["group", "tuning", "G"]);
    }

    #[test]
    fn flag_equals_value() {
        let a = parse(&["register", "--label=work one", "--group=Default", "--yes"]);
        assert!(a.errors.is_empty(), "{:?}", a.errors);
        assert_eq!(a.val("label").as_deref(), Some("work one"));
        assert_eq!(a.val("group").as_deref(), Some("Default"));
        assert!(a.has("yes"));
        assert_eq!(parse(&["x", "--label="]).errors, vec!["--label needs a value"]);
        assert_eq!(parse(&["x", "--label=--group"]).errors, vec!["--label needs a value"]);
        assert_eq!(parse(&["x", "--yes=1"]).errors, vec!["--yes does not take a value"]);
    }

    #[test]
    fn usage_errors_are_refused_but_bare_version_still_works() {
        let dd = DataDir::at(std::path::Path::new("Z:\\qf-test-does-not-exist"));
        assert_eq!(run(&parse(&["update", "--version"]), &dd), 1);
        assert_eq!(run(&parse(&["add", "--label", "--group", "G"]), &dd), 1);
        assert_eq!(run(&parse(&["--version"]), &dd), 0);
    }

    #[test]
    fn versions_are_normalized() {
        assert_eq!(normalize_version("0.1.0").unwrap(), "v0.1.0");
        assert_eq!(normalize_version("v0.1.0").unwrap(), "v0.1.0");
        assert_eq!(normalize_version(" v1.2.3-rc.1 ").unwrap(), "v1.2.3-rc.1");
        for bad in ["", "v", "latest", "1.2", "v1.2.3.4", "v1.x.3", "v1.2.3-", "1.2.3 ; rm"] {
            assert!(normalize_version(bad).is_err(), "{bad}");
        }
    }
}
