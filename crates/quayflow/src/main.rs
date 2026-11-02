//! `quayflow`: the Quayflow engine and command line.

mod canary;
mod client;
mod local;
mod print;

use client::{Client, ClientError};
use qf_engine::DataDir;
use serde_json::{json, Value};
use std::time::Duration;

const HELP: &str = "Quayflow: account groups, usage-aware switching and recovery for Claude Code.

Usage: quayflow <command> [options]

Engine
  start [--if-consented]       start the engine in the background (no window)
  stop | pause | resume        stop the engine; pause or resume automation
  status [--json]              environments, accounts, usage and decisions
  serve                        run the engine in the foreground

Accounts and groups
  onboard                      first-use consents (interactive)
  register --label L --group G [--accept-disclosure]
  add --label L --group G [--accept-disclosure]    sign in to another account (new window)
  reauth <account>             sign in again for an account that was set aside
  remove <account> [--yes] | label <account> <new label>
  use <account> [--idle-wait] [--env E]            switch now (pins your choice)
  accept | unpin
  mode <manual|suggest|automatic> [--group G] [--approve]
  group list | create G | delete G | default G | add G <acct..> [--accept-disclosure]
        | remove G <acct..> | preferred G <acct|none> | rank G <acct> <preferred|normal|backup>
        | tuning G [--threshold N] [--margin N]
  project list | assign <path> <G> | unassign <path>
  unknown ignore [--env E]     acknowledge an unregistered live login (automation stays paused)

Lanes and sessions
  lanes list | create N | check N | delete N
  handoff prepare|open|replace --session ID --to ENV
  shell install | uninstall    PowerShell prompt hook for same-terminal Replace

Maintenance
  canary [--live]              version check (static); --live runs the synthetic adoption job
  policy [--json]              managed login enforcement per environment
  report                       local support report for review
  migrate-poc [--group G] [--accept-disclosure]
  consent <engine-start|signin-startup|credential-storage|terms|prompt-hook|live-canary> <on|off>
  update [--check] [--version vX.Y.Z]
  uninstall [--delete-data] [--yes]
  version
";

struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(raw: Vec<String>) -> Args {
        let mut pos = vec![];
        let mut flags = vec![];
        let mut it = raw.into_iter().peekable();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                let takes = matches!(
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
                );
                let val = if takes { it.next() } else { None };
                flags.push((name.to_string(), val));
            } else {
                pos.push(a);
            }
        }
        Args { pos, flags }
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

fn fail(msg: impl std::fmt::Display) -> i32 {
    eprintln!("quayflow: {msg}");
    1
}

fn connect(dd: &DataDir) -> Result<Client, ClientError> {
    Client::connect(dd)
}

fn say(v: Result<Value, ClientError>) -> i32 {
    match v {
        Ok(v) => {
            if v.get("ok") == Some(&json!(false)) {
                println!("{}", v["message"].as_str().unwrap_or("refused"));
                return 1;
            } else if let Some(m) = v["message"].as_str() {
                println!("{m}");
            } else {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            0
        }
        Err(ClientError::Api { code, message, .. }) if code == "needs_disclosure" => {
            println!("{message}\n\nRepeat the command with --accept-disclosure to accept this once for the group.");
            2
        }
        Err(ClientError::Api { code, message, .. }) if code == "needs_approval" => {
            println!("{message}\n\nRepeat with --approve to enable it.");
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
            println!(
                "Switch requested ({}). Each session uses the new account from its next request.",
                t["state"].as_str().unwrap_or("?")
            );
            0
        }
        Ok(v) => {
            println!("{}", v["message"].as_str().unwrap_or("refused"));
            if let Some(b) = v["blockedBy"].as_object() {
                println!(
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
    match cmd {
        "help" | "-h" | "/?" => {
            println!("{HELP}");
            0
        }
        "version" | "-V" => {
            println!("quayflow {} (protocol {})", qf_model::ENGINE_VERSION, qf_model::PROTOCOL);
            0
        }
        "serve" => match qf_engine::serve(dd.clone(), qf_adapter::Env::default_env(), Duration::from_secs(2)) {
            Ok(true) => 0,
            Ok(false) => {
                println!("A Quayflow engine is already running for this user.");
                0
            }
            Err(e) => fail(e),
        },
        "start" => local::start(dd, a.has("if-consented")),
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
            let (Some(label), Some(group)) = (a.val("label"), a.val("group")) else { return fail("register --label L --group G") };
            let r = connect(dd).and_then(|c| {
                c.post(
                    "/v1/accounts/register",
                    json!({"label": label, "group": group, "env": a.val("env"), "acceptDisclosure": a.has("accept-disclosure")}),
                )
            });
            match r {
                Ok(v) => {
                    println!("{}", v["note"].as_str().map(String::from).unwrap_or_else(|| format!("Registered {} in group \"{group}\". Nothing was switched.", v["label"].as_str().unwrap_or(&label))));
                    0
                }
                Err(e) => say(Err(e)),
            }
        }
        "add" | "reauth" => signin(dd, a),
        "remove" => {
            let Some(acct) = a.p(1) else { return fail("remove <account>") };
            if !a.has("yes") && !ask_yes(&format!("Remove {acct} from Quayflow (its protected copy is deleted; history stays)?")) {
                println!("Cancelled; nothing was removed.");
                return 1;
            }
            say(connect(dd).and_then(|c| c.post("/v1/accounts/remove", json!({"account": acct}))))
        }
        "label" => {
            let (Some(acct), Some(l)) = (a.p(1), a.p(2)) else { return fail("label <account> <new label>") };
            say(connect(dd).and_then(|c| c.post("/v1/accounts/label", json!({"account": acct, "label": l}))))
        }
        "use" => {
            let Some(to) = a.p(1) else { return fail("use <account>") };
            switch_result(connect(dd).and_then(|c| c.post("/v1/switch", json!({"to": to, "env": a.val("env"), "idleWait": a.has("idle-wait")}))))
        }
        "accept" => switch_result(connect(dd).and_then(|c| c.post("/v1/accept", json!({"env": a.val("env")})))),
        "unpin" => say(connect(dd).and_then(|c| c.post("/v1/unpin", json!({"env": a.val("env")})))),
        "mode" => {
            let Some(m) = a.p(1) else { return fail("mode <manual|suggest|automatic>") };
            let group = a.val("group").unwrap_or_else(|| "Default".into());
            say(connect(dd).and_then(|c| c.post("/v1/groups/mode", json!({"group": group, "mode": m, "approveAutomatic": a.has("approve")}))))
        }
        "group" => group_cmd(dd, a),
        "project" => match a.p(1) {
            Some("assign") => {
                let (Some(p), Some(g)) = (a.p(2), a.p(3)) else { return fail("project assign <path> <group>") };
                let p = std::path::absolute(p).map(|x| x.display().to_string()).unwrap_or_else(|_| p.into());
                say(connect(dd).and_then(|c| c.post("/v1/projects/assign", json!({"path": p, "group": g}))))
            }
            Some("unassign") => {
                let Some(p) = a.p(2) else { return fail("project unassign <path>") };
                let p = std::path::absolute(p).map(|x| x.display().to_string()).unwrap_or_else(|_| p.into());
                say(connect(dd).and_then(|c| c.post("/v1/projects/unassign", json!({"path": p}))))
            }
            _ => match connect(dd).and_then(|c| c.get("/v1/state")) {
                Ok(v) => {
                    println!("Unassigned projects use \"{}\".", v["defaultGroup"].as_str().unwrap_or("?"));
                    for p in v["projects"].as_array().into_iter().flatten() {
                        println!("  {}  ->  {}", p["path"].as_str().unwrap_or(""), p["group"].as_str().unwrap_or(""));
                    }
                    0
                }
                Err(e) => fail(e),
            },
        },
        "unknown" => say(connect(dd).and_then(|c| c.post("/v1/unknown", json!({"action": a.p(1).unwrap_or("ignore"), "env": a.val("env")})))),
        "lanes" | "lane" => match a.p(1).unwrap_or("list") {
            "list" => match connect(dd).and_then(|c| c.get("/v1/lanes")) {
                Ok(v) => {
                    print::lanes(&v);
                    0
                }
                Err(e) => fail(e),
            },
            op @ ("create" | "check" | "delete") => {
                let Some(n) = a.p(2) else { return fail(format!("lanes {op} <name>")) };
                let r = connect(dd).and_then(|c| c.post(&format!("/v1/lanes/{op}"), json!({"name": n})));
                match r {
                    Ok(v) if op != "delete" => {
                        print::lanes(&json!([v]));
                        0
                    }
                    r => say(r),
                }
            }
            _ => fail("lanes list|create|check|delete"),
        },
        "handoff" => {
            let op = a.p(1).unwrap_or("");
            if op == "resume" {
                let wait = a.val("wait-pid").and_then(|p| p.parse().ok());
                return local::handoff_resume(dd, &a.val("session").unwrap_or_default(), &a.val("to").unwrap_or_default(), wait, a.val("cwd").as_deref());
            }
            if !matches!(op, "prepare" | "open" | "replace") {
                return fail("handoff prepare|open|replace --session ID --to ENV");
            }
            let r = connect(dd).and_then(|c| {
                c.post(&format!("/v1/handoff/{op}"), json!({"sessionId": a.val("session"), "toEnv": a.val("to").unwrap_or_default()}))
            });
            match r {
                Ok(v) => {
                    println!("{}", v["message"].as_str().unwrap_or(""));
                    println!("Files copied with integrity check: {}", v["fileCount"]);
                    println!("Command: {}", v["command"].as_str().unwrap_or(""));
                    0
                }
                Err(e) => fail(e),
            }
        }
        "shell" => match a.p(1) {
            Some("install") => match local::shell_install(dd, &local::profile_paths()) {
                Ok(p) => {
                    let _ = local::set_consent(dd, "promptHook", true);
                    println!("Prompt hook installed in:
  {}
Open a new PowerShell window to load it.", p.join("
  "));
                    let policy = local::ps51_execution_policy();
                    if !local::profiles_load(&policy) {
                        println!("Note: Windows PowerShell 5.1's execution policy is {policy}, so it does not load profiles. To use the hook there, run in Windows PowerShell:
  Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
Quayflow does not change it for you. PowerShell 7 is not affected.");
                    }
                    0
                }
                Err(e) => fail(e),
            },
            Some("uninstall") => match local::shell_uninstall(dd, &local::profile_paths()) {
                Ok(p) => {
                    let _ = local::set_consent(dd, "promptHook", false);
                    println!("Prompt hook removed{}.", if p.is_empty() { " (none found)".to_string() } else { format!(" from {}", p.join(", ")) });
                    0
                }
                Err(e) => fail(e),
            },
            Some("claim") => {
                let pid = a.val("pid").and_then(|p| p.parse().ok()).unwrap_or(0);
                local::shell_claim(dd, pid)
            }
            _ => fail("shell install|uninstall|claim --pid N"),
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
                    let enforced = !m["forceOrg"].as_array().map(|x| x.is_empty()).unwrap_or(true) || !m["forceMethod"].is_null();
                    println!(
                        "{}: {}",
                        e["env"].as_str().unwrap_or("?"),
                        if enforced { format!("enforced by {} (organization #{:?}, method {})", m["sources"], m["forceOrg"], m["forceMethod"]) } else { "no managed login enforcement".into() }
                    );
                }
                println!("Managed-login override setting: {}", if v["dOrg"]["enabled"] == json!(true) { "ON (recorded only; enforcement still respected)" } else { "off" });
                0
            }
            Err(e) => fail(e),
        },
        "report" => match connect(dd).and_then(|c| c.post("/v1/report", json!({}))) {
            Ok(v) => {
                println!("{}\n...\nSaved for your review: {}\nNothing was sent anywhere. Share it only if you choose to.", v["preview"].as_str().unwrap_or(""), v["path"].as_str().unwrap_or(""));
                0
            }
            Err(e) => fail(e),
        },
        "migrate-poc" => say(connect(dd).and_then(|c| {
            c.post("/v1/migrate-poc", json!({"group": a.val("group").unwrap_or_else(|| "Default".into()), "acceptDisclosure": a.has("accept-disclosure")}))
        })),
        "consent" => {
            let (Some(name), Some(state)) = (a.p(1), a.p(2)) else { return fail("consent <name> <on|off>") };
            let Some(field) = local::consent_field(name) else { return fail("unknown consent name") };
            let on = match state {
                "on" => true,
                "off" => false,
                _ => return fail("consent <name> <on|off>"),
            };
            if let Err(e) = local::set_consent(dd, field, on) {
                return fail(e);
            }
            if field == "signinStartup" {
                if let Err(e) = local::set_startup(on) {
                    return fail(format!("startup entry: {e}"));
                }
            }
            println!("{name}: {}", if on { "on" } else { "off" });
            0
        }
        "update" => match local::update(dd, a.has("check"), a.val("version").as_deref()) {
            Ok(m) => {
                println!("{m}");
                0
            }
            Err(e) => fail(e),
        },
        "uninstall" => {
            if !a.has("yes")
                && !ask_yes(if a.has("delete-data") {
                    "Uninstall Quayflow and delete its vault, logs and settings?"
                } else {
                    "Uninstall Quayflow (vault and logs are kept)?"
                })
            {
                return 1;
            }
            for n in local::uninstall(dd, a.has("delete-data")) {
                println!("- {n}");
            }
            0
        }
        _ => {
            eprintln!("unknown command \"{cmd}\"\n");
            println!("{HELP}");
            2
        }
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
                    println!(
                        "{}\n\nRepeat with --accept-disclosure to accept this once for the group.",
                        v["disclosure"].as_str().unwrap_or("")
                    );
                    2
                }
                r => say(r.map(|_| json!({"message": "membership updated"}))),
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
                return fail("group tuning G [--threshold N] [--margin N]");
            }
            say(c.post(
                "/v1/groups/tuning",
                json!({"group": g, "threshold": threshold, "margin": margin}),
            ))
        }
        _ => fail("group list|create|delete|default|add|remove|preferred|rank|tuning"),
    }
}

fn signin(dd: &DataDir, a: &Args) -> i32 {
    let c = match connect(dd) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let reauth = (a.p(0) == Some("reauth")).then(|| a.p(1).unwrap_or("").to_string());
    if reauth.is_none() && (a.val("label").is_none() || a.val("group").is_none()) {
        return fail("add --label L --group G");
    }
    let s = match c.post("/v1/accounts/signin/start", json!({"reauth": reauth})) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let id = s["id"].as_str().unwrap_or("").to_string();
    println!("A sign-in window opened. Sign in there with the account to add.");
    if let Some(m) = s["message"].as_str() {
        println!("{m}");
    }
    println!(
        "If no window appeared, run this in another terminal:\n  {}",
        s["command"].as_str().unwrap_or("")
    );
    println!("Waiting for the sign-in to finish (Ctrl+C cancels; your current login is not touched)...");
    let deadline = std::time::Instant::now() + Duration::from_secs(15 * 60);
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let st = match c.get(&format!("/v1/accounts/signin/status?id={id}")) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        if st["state"] == "ready" {
            let ident = &st["identity"];
            println!(
                "Signed in: {} account #{}{}",
                ident["kind"].as_str().unwrap_or("?"),
                ident["account"].as_str().unwrap_or("?"),
                ident["display"].as_str().map(|d| format!(" ({d})")).unwrap_or_default()
            );
            if ident["unexpected"] == json!(true) {
                println!("This is not the account being reauthenticated. Nothing was replaced.");
                let _ = c.post("/v1/accounts/signin/cancel", json!({"id": id}));
                return 1;
            }
            if let Some(d) = ident["duplicateOf"].as_str() {
                println!("This account is already registered as \"{d}\"; its protected copy will be updated (not added as extra capacity).");
            }
            if !ask_yes("Store this account in Quayflow's protected vault?") {
                let _ = c.post("/v1/accounts/signin/cancel", json!({"id": id}));
                println!("Cancelled. Your current login is unchanged.");
                return 1;
            }
            let r = c.post(
                "/v1/accounts/signin/confirm",
                json!({"id": id, "label": a.val("label"), "group": a.val("group"), "acceptDisclosure": a.has("accept-disclosure")}),
            );
            if let Err(ClientError::Api { code, .. }) = &r {
                if code == "needs_disclosure" {
                    let _ = c.post("/v1/accounts/signin/cancel", json!({"id": id}));
                }
            }
            return say(r);
        }
        if let Some(m) = st["message"].as_str() {
            if st["state"] == "waiting" && m.contains("verified") {
                println!("{m}");
            }
        }
        if std::time::Instant::now() > deadline {
            let _ = c.post("/v1/accounts/signin/cancel", json!({"id": id}));
            return fail("sign-in timed out; your current login is unchanged");
        }
    }
}

fn onboard(dd: &DataDir) -> i32 {
    println!(
        "Quayflow keeps several Claude accounts and switches between them inside running sessions.

- The Mod (inside Claude) shows status and asks the engine to act. The engine (quayflow.exe,
  one per Windows user, no window) observes usage, decides, and swaps Claude's login in place.
- Logins are stored locally in a vault protected by Windows (DPAPI, your user account).
  Quayflow never refreshes tokens and never sends them anywhere except Anthropic's own endpoints.
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
        ("signinStartup", "Start the engine when I sign in to Windows"),
    ];
    let mut consents = serde_json::Map::new();
    for (k, q) in items {
        consents.insert(k.into(), json!(ask_yes(q)));
    }
    if consents["terms"] != json!(true) {
        println!("Onboarding stopped: the terms notice must be accepted.");
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
            eprintln!("startup entry not created: {e}");
        }
    }
    println!("Saved. Next: quayflow register --label <name> --group Default");
    0
}
