//! Commands that act locally: engine start/stop, consents and the startup entry, PowerShell
//! integration, handoff resume, update and uninstall.

use crate::client::Client;
use qf_engine::DataDir;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const REPO: &str = "minhdaohuu/quayflow";
pub const PSM1: &str = include_str!("../../../shell/Quayflow.psm1");
pub const BLOCK_START: &str = "# >>> quayflow >>>";
pub const BLOCK_END: &str = "# <<< quayflow <<<";

pub fn profile_block() -> String {
    [
        BLOCK_START,
        "# Managed by `quayflow shell install`; removed by `quayflow shell uninstall`. Do not edit inside this block.",
        "$QuayflowModule = Join-Path $env:LOCALAPPDATA 'Quayflow\\shell\\Quayflow.psm1'",
        "if (Test-Path -LiteralPath $QuayflowModule) { Import-Module $QuayflowModule -DisableNameChecking; Enable-QuayflowPromptHook }",
        "Remove-Variable QuayflowModule -ErrorAction SilentlyContinue",
        BLOCK_END,
    ]
    .join("\r\n")
}

fn engine_alive(dd: &DataDir) -> Option<Client> {
    Client::connect(dd).ok().filter(|c| c.alive())
}

/// `quayflow start [--if-consented]`: prints one JSON line.
pub fn start(dd: &DataDir, if_consented: bool) -> i32 {
    let out = |state: &str, extra: Value| {
        let mut v = json!({"state": state});
        if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
            o.extend(e);
        }
        println!("{v}");
    };
    if engine_alive(dd).is_some() {
        out("running", json!({}));
        return 0;
    }
    if if_consented {
        let cfg = qf_engine::core::load_config(dd).unwrap_or_default();
        if cfg.consents.engine_start.is_none() {
            out("no_consent", json!({}));
            return 0;
        }
        if dd.stopped_marker().exists() {
            out("stopped_by_user", json!({}));
            return 0;
        }
    } else {
        let _ = std::fs::remove_file(dd.stopped_marker());
    }
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            out("failed", json!({"error": e.to_string()}));
            return 1;
        }
    };
    if let Err(e) = qf_adapter::process::spawn_detached(&exe, &["serve"]) {
        out("failed", json!({"error": e.to_string()}));
        return 1;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(c) = engine_alive(dd) {
            out("started", json!({"pid": c.pid()}));
            return 0;
        }
    }
    out("failed", json!({"log": dd.logs().display().to_string()}));
    1
}

pub fn consent_field(name: &str) -> Option<&'static str> {
    Some(match name {
        "engine-start" => "engineStart",
        "signin-startup" => "signinStartup",
        "credential-storage" => "credentialStorage",
        "terms" => "terms",
        "prompt-hook" => "promptHook",
        "live-canary" => "liveCanary",
        _ => return None,
    })
}

/// Record a consent through the engine when it runs, else directly in config.json.
pub fn set_consent(dd: &DataDir, field: &str, on: bool) -> Result<(), String> {
    if let Some(c) = engine_alive(dd) {
        return c
            .post("/v1/onboarding", json!({"consents": {field: on}}))
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    let mut cfg = qf_engine::core::load_config(dd)?;
    let now = qf_model::time::now_ms();
    let slot = match field {
        "engineStart" => &mut cfg.consents.engine_start,
        "signinStartup" => &mut cfg.consents.signin_startup,
        "credentialStorage" => &mut cfg.consents.credential_storage,
        "terms" => &mut cfg.consents.terms,
        "promptHook" => &mut cfg.consents.prompt_hook,
        "liveCanary" => &mut cfg.consents.live_canary,
        _ => return Err("unknown consent".into()),
    };
    *slot = if on { Some(slot.unwrap_or(now)) } else { None };
    let text = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    qf_vault::atomic_write(&dd.config(), text.as_bytes()).map_err(|e| e.to_string())
}

/// Per-user startup entry (no administrator rights): HKCU Run -> quayflow-bg.exe start.
#[cfg(windows)]
pub fn set_startup(on: bool) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    let run = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Software\\Microsoft\\Windows\\CurrentVersion\\Run", KEY_SET_VALUE)
        .map_err(|e| e.to_string())?;
    if on {
        let bg = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("quayflow-bg.exe");
        if !bg.is_file() {
            return Err(format!("{} not found", bg.display()));
        }
        run.set_value("Quayflow", &format!("\"{}\" start", bg.display()))
            .map_err(|e| e.to_string())
    } else {
        match run.delete_value("Quayflow") {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(not(windows))]
pub fn set_startup(_on: bool) -> Result<(), String> {
    Err("startup entries are Windows-only".into())
}

/// The user's Documents folder (honors OneDrive redirection).
pub fn documents_dir() -> PathBuf {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        if let Ok(k) = winreg::RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders")
        {
            if let Ok(v) = k.get_value::<String, _>("Personal") {
                return PathBuf::from(expand_env(&v));
            }
        }
    }
    qf_adapter::env::home_dir().join("Documents")
}

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let name = &after[..j];
                out.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Effective Windows PowerShell 5.1 execution policy (Group Policy, then user, then machine;
/// Windows client default is Restricted). Profiles only load under RemoteSigned, Unrestricted or Bypass.
#[cfg(windows)]
pub fn ps51_execution_policy() -> String {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let read = |hive, path: &str, value: &str| -> Option<String> {
        RegKey::predef(hive)
            .open_subkey(path)
            .ok()?
            .get_value::<String, _>(value)
            .ok()
    };
    let gpo = r"Software\Policies\Microsoft\Windows\PowerShell";
    let ids = r"Software\Microsoft\PowerShell\1\ShellIds\Microsoft.PowerShell";
    read(HKEY_LOCAL_MACHINE, gpo, "ExecutionPolicy")
        .or_else(|| read(HKEY_CURRENT_USER, gpo, "ExecutionPolicy"))
        .or_else(|| read(HKEY_CURRENT_USER, ids, "ExecutionPolicy"))
        .or_else(|| read(HKEY_LOCAL_MACHINE, ids, "ExecutionPolicy"))
        .unwrap_or_else(|| "Restricted".into())
}

#[cfg(not(windows))]
pub fn ps51_execution_policy() -> String {
    "Unrestricted".into()
}

pub fn profiles_load(policy: &str) -> bool {
    matches!(
        policy.to_ascii_lowercase().as_str(),
        "remotesigned" | "unrestricted" | "bypass"
    )
}

pub fn profile_paths() -> Vec<PathBuf> {
    let d = documents_dir();
    vec![
        d.join("PowerShell").join("Microsoft.PowerShell_profile.ps1"),
        d.join("WindowsPowerShell").join("Microsoft.PowerShell_profile.ps1"),
    ]
}

/// Remove the managed block (inclusive); keeps everything else byte for byte.
pub fn strip_block(text: &str) -> String {
    let mut out = vec![];
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_end_matches(['\r', '\n']);
        if t == BLOCK_START {
            inside = true;
            continue;
        }
        if inside {
            if t == BLOCK_END {
                inside = false;
            }
            continue;
        }
        out.push(line);
    }
    out.concat()
}

pub fn add_block(text: &str) -> String {
    let mut base = strip_block(text);
    if !base.is_empty() && !base.ends_with('\n') {
        base.push_str("\r\n");
    }
    format!("{base}{}\r\n", profile_block())
}

pub fn shell_install(dd: &DataDir, profiles: &[PathBuf]) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dd.shell()).map_err(|e| e.to_string())?;
    qf_vault::atomic_write(&dd.shell().join("Quayflow.psm1"), PSM1.as_bytes()).map_err(|e| e.to_string())?;
    let mut done = vec![];
    for p in profiles {
        let text = std::fs::read_to_string(p).unwrap_or_default();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        qf_vault::atomic_write(p, add_block(&text).as_bytes()).map_err(|e| format!("{}: {e}", p.display()))?;
        done.push(p.display().to_string());
    }
    Ok(done)
}

pub fn shell_uninstall(dd: &DataDir, profiles: &[PathBuf]) -> Result<Vec<String>, String> {
    let mut done = vec![];
    for p in profiles {
        if let Ok(text) = std::fs::read_to_string(p) {
            if text.contains(BLOCK_START) {
                qf_vault::atomic_write(p, strip_block(&text).as_bytes()).map_err(|e| e.to_string())?;
                done.push(p.display().to_string());
            }
        }
    }
    let _ = std::fs::remove_file(dd.shell().join("Quayflow.psm1"));
    Ok(done)
}

/// `shell claim --pid N`: print and consume this terminal's pending replacement.
pub fn shell_claim(dd: &DataDir, pid: u32) -> i32 {
    let path = dd.claims().join(format!("{pid}.json"));
    if !path.exists() {
        return 0;
    }
    let taken = dd.claims().join(format!("{pid}.{}.taken", std::process::id()));
    if std::fs::rename(&path, &taken).is_err() {
        return 0; // consumed by someone else
    }
    let text = std::fs::read_to_string(&taken).unwrap_or_default();
    let _ = std::fs::remove_file(&taken);
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => {
            println!("{v}");
            0
        }
        Err(_) => 2,
    }
}

/// Run inside the window opened by Open New Session: wait for the source session to end
/// (one executor), then resume the conversation under the destination environment.
pub fn handoff_resume(dd: &DataDir, session: &str, to: &str, wait_pid: Option<u32>, cwd: Option<&str>) -> i32 {
    if let Some(pid) = wait_pid {
        if qf_adapter::process::is_alive(pid) {
            outln!("Quayflow: waiting for the original session to end (type /exit there). The conversation continues here.");
            while qf_adapter::process::is_alive(pid) {
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
    let Some(claude) = qf_adapter::process::find_claude() else {
        eprintln!("claude.exe not found on PATH");
        return 1;
    };
    let mut c = std::process::Command::new(claude);
    c.args(["--resume", session]);
    if to == "default" || to.is_empty() {
        c.env_remove("CLAUDE_CONFIG_DIR");
    } else {
        c.env("CLAUDE_CONFIG_DIR", dd.lanes().join(to));
    }
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    match c.status() {
        Ok(s) => s.code().unwrap_or(0),
        Err(e) => {
            eprintln!("could not start claude: {e}");
            1
        }
    }
}

// ---------------- update ----------------

pub fn sha256_file(p: &Path) -> Result<String, String> {
    use sha2::Digest;
    let b = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(hex::encode(sha2::Sha256::digest(&b)))
}

/// Parse `SHA256SUMS` ("<hash>  <name>" or "<hash> *<name>").
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let h = it.next()?.to_ascii_lowercase();
            let n = it.next()?.trim_start_matches('*').to_string();
            (h.len() == 64).then_some((h, n))
        })
        .collect()
}

pub fn verify_against(sums: &[(String, String)], file: &Path) -> Result<(), String> {
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let expected = sums
        .iter()
        .find(|(_, n)| n == name)
        .map(|(h, _)| h)
        .ok_or_else(|| format!("{name} is not listed in SHA256SUMS"))?;
    let actual = sha256_file(file)?;
    if &actual != expected {
        return Err(format!(
            "hash mismatch for {name}: expected {expected}, got {actual}; refusing to install"
        ));
    }
    Ok(())
}

fn have(cmd: &str) -> bool {
    qf_adapter::process::hidden("where.exe")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn download(assets: &[&str], dest: &Path, version: Option<&str>) -> Result<(), String> {
    if std::env::var_os("QUAYFLOW_OFFLINE").is_some() {
        return Err("network access is disabled (QUAYFLOW_OFFLINE is set)".into());
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    if have("gh") {
        let mut c = qf_adapter::process::hidden("gh");
        c.args(["release", "download"]);
        if let Some(v) = version {
            c.arg(v);
        }
        c.args(["--repo", REPO, "--clobber", "-D"]).arg(dest);
        for a in assets {
            c.args(["-p", a]);
        }
        let o = c.output().map_err(|e| e.to_string())?;
        if o.status.success() {
            return Ok(());
        }
    }
    let agent = ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(
            native_tls::TlsConnector::new().map_err(|e| e.to_string())?,
        ))
        .timeout(Duration::from_secs(300))
        .build();
    for a in assets {
        let url = match version {
            Some(v) => format!("https://github.com/{REPO}/releases/download/{v}/{a}"),
            None => format!("https://github.com/{REPO}/releases/latest/download/{a}"),
        };
        let resp = agent.get(&url).call().map_err(|e| match e {
            ureq::Error::Status(404, _) => format!(
                "no published release found at https://github.com/{REPO}/releases{} (no release yet, or the repository is private and the GitHub CLI is not signed in)",
                version.map(|v| format!("/tag/{v}")).unwrap_or_default()
            ),
            e => format!("{a}: {e}"),
        })?;
        let mut bytes = vec![];
        std::io::Read::read_to_end(&mut resp.into_reader(), &mut bytes).map_err(|e| e.to_string())?;
        std::fs::write(dest.join(a), bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn update(dd: &DataDir, check_only: bool, version: Option<&str>) -> Result<String, String> {
    let tmp = dd.root.join("update");
    let _ = std::fs::remove_dir_all(&tmp);
    let assets = ["quayflow.exe", "quayflow-bg.exe", "SHA256SUMS"];
    download(&assets, &tmp, version)?;
    let sums = parse_sums(&std::fs::read_to_string(tmp.join("SHA256SUMS")).map_err(|e| e.to_string())?);
    for a in &assets[..2] {
        verify_against(&sums, &tmp.join(a))?;
    }
    let attest = if have("gh") {
        let o = qf_adapter::process::hidden("gh")
            .args(["attestation", "verify"])
            .arg(tmp.join("quayflow.exe"))
            .args(["--repo", REPO])
            .output()
            .map_err(|e| e.to_string())?;
        if !o.status.success() {
            return Err("GitHub attestation verification failed; refusing to install".into());
        }
        "attestation verified"
    } else {
        "attestation not checked (GitHub CLI not installed)"
    };
    let bin = dd.bin();
    let current = sha256_file(&bin.join("quayflow.exe")).unwrap_or_default();
    let new = sha256_file(&tmp.join("quayflow.exe"))?;
    if current == new {
        let _ = std::fs::remove_dir_all(&tmp);
        return Ok("Quayflow is up to date.".into());
    }
    if check_only {
        return Ok(format!(
            "An update is available (hash verified, {attest}). Run `quayflow update` to install it."
        ));
    }
    // Never replace components mid-transition.
    if let Some(c) = engine_alive(dd) {
        let st = c.get("/v1/state").map_err(|e| e.to_string())?;
        let busy = st["envs"].as_array().into_iter().flatten().any(|e| {
            matches!(
                e["transition"]["state"].as_str(),
                Some("requested" | "preparing" | "idle_wait" | "applying")
            )
        });
        if busy {
            return Err("a switch is in progress; try again in a minute".into());
        }
        let _ = c.post("/v1/engine", json!({"action": "stop"}));
        let deadline = Instant::now() + Duration::from_secs(15);
        while qf_adapter::process::is_alive(c.pid()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    for a in &assets[..2] {
        let target = bin.join(a);
        let old = bin.join(format!("{a}.old"));
        let _ = std::fs::remove_file(&old);
        if target.exists() {
            std::fs::rename(&target, &old).map_err(|e| format!("{a}: {e}"))?;
        }
        std::fs::copy(tmp.join(a), &target).map_err(|e| format!("{a}: {e}"))?;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    // The Mod, as the matched pair, through Claude's plugin manager.
    let mut notes = vec![format!("engine updated (hash verified, {attest})")];
    for args in [
        vec!["plugin", "marketplace", "update", "quayflow"],
        vec!["plugin", "update", "quayflow@quayflow"],
    ] {
        match qf_adapter::process::find_claude().map(|c| qf_adapter::process::hidden(c).args(&args).output()) {
            Some(Ok(o)) if o.status.success() => {}
            _ => notes.push(format!("run `claude {}` yourself", args.join(" "))),
        }
    }
    let _ = std::fs::remove_file(dd.stopped_marker());
    let _ = qf_adapter::process::spawn_detached(&bin.join("quayflow.exe"), &["start"]);
    notes.push("restart running Claude sessions to load the updated Mod".into());
    Ok(notes.join("; "))
}

// ---------------- uninstall ----------------

#[cfg(windows)]
fn remove_from_user_path(dir: &Path) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    if let Ok(k) =
        winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Environment", KEY_READ | KEY_SET_VALUE)
    {
        if let Ok(p) = k.get_value::<String, _>("Path") {
            let d = qf_model::normalize_path(&dir.to_string_lossy());
            let kept: Vec<&str> = p
                .split(';')
                .filter(|x| !x.is_empty() && qf_model::normalize_path(x) != d)
                .collect();
            let _ = k.set_value("Path", &kept.join(";"));
        }
    }
}

#[cfg(not(windows))]
fn remove_from_user_path(_dir: &Path) {}

pub fn uninstall(dd: &DataDir, delete_data: bool) -> Vec<String> {
    let mut notes = vec![];
    if let Some(c) = engine_alive(dd) {
        let _ = c.post("/v1/engine", json!({"action": "stop"}));
        let deadline = Instant::now() + Duration::from_secs(15);
        while qf_adapter::process::is_alive(c.pid()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        notes.push("engine stopped".into());
    }
    if let Some(claude) = qf_adapter::process::find_claude() {
        for args in [
            vec!["plugin", "uninstall", "quayflow@quayflow"],
            vec!["plugin", "marketplace", "remove", "quayflow"],
        ] {
            let ok = qf_adapter::process::hidden(&claude)
                .args(&args)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            notes.push(format!(
                "claude {}: {}",
                args.join(" "),
                if ok { "done" } else { "not needed or failed" }
            ));
        }
    }
    match set_startup(false) {
        Ok(()) => notes.push("startup entry removed".into()),
        Err(e) => notes.push(format!("startup entry: {e}")),
    }
    match shell_uninstall(dd, &profile_paths()) {
        Ok(p) if !p.is_empty() => notes.push(format!("prompt hook removed from {}", p.join(", "))),
        _ => {}
    }
    remove_from_user_path(&dd.bin());
    notes.push("Claude's current login is left as it is: your active account keeps working".into());
    if delete_data {
        let journal = qf_engine::journal::Journal::new(dd.journal());
        let unresolved = journal.read_all().iter().any(|r| r["event"] == "transition.requested")
            && !journal
                .read_all()
                .iter()
                .any(|r| r["event"] == "transition.reconciled" || r["event"] == "transition.written");
        for p in [
            dd.vault(),
            dd.logs(),
            dd.compat_dir(),
            dd.reports(),
            dd.signin_dir(),
            dd.root.join("handoff"),
            dd.shell(),
            dd.lanes(),
        ] {
            let _ = std::fs::remove_dir_all(p);
        }
        for f in [
            dd.config(),
            dd.observations(),
            dd.engine_json(),
            dd.engine_lock(),
            dd.stopped_marker(),
        ] {
            let _ = std::fs::remove_file(f);
        }
        if unresolved {
            notes.push(format!("kept {} (unresolved recovery record)", dd.journal().display()));
        } else {
            let _ = std::fs::remove_file(dd.journal());
        }
        notes.push(format!(
            "vault, logs and settings deleted; protected backups of unknown logins are kept in {} (delete them yourself when no longer needed)",
            dd.backups().display()
        ));
    } else {
        notes.push(format!(
            "vault and logs kept in {} (quayflow uninstall --delete-data removes them)",
            dd.root.display()
        ));
    }
    // The running executable cannot delete itself: remove the program folder after exit.
    let bin = dd.bin();
    if bin.exists() {
        let script = format!("ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{}\"", bin.display());
        let _ = qf_adapter::process::hidden("cmd.exe").args(["/c", &script]).spawn();
        notes.push("program files removed".into());
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_block_round_trip_preserves_user_content() {
        let user = "Set-Alias ll Get-ChildItem\r\nfunction prompt { 'x> ' }\r\n";
        let with = add_block(user);
        assert!(with.starts_with(user));
        assert!(with.contains("Enable-QuayflowPromptHook"));
        assert_eq!(add_block(&with), with, "idempotent");
        assert_eq!(strip_block(&with), user);
        assert_eq!(strip_block(user), user);
        // The psm1 documents the same block.
        for line in profile_block().split("\r\n") {
            assert!(PSM1.contains(line), "psm1 contract out of sync: {line}");
        }
    }

    #[test]
    fn shell_install_and_uninstall() {
        let d = tempfile::tempdir().unwrap();
        let dd = DataDir::at(&d.path().join("qf"));
        let profiles = vec![d.path().join("a").join("p.ps1"), d.path().join("b").join("p.ps1")];
        std::fs::create_dir_all(d.path().join("a")).unwrap();
        std::fs::write(&profiles[0], "# mine\r\n").unwrap();
        shell_install(&dd, &profiles).unwrap();
        assert!(dd.shell().join("Quayflow.psm1").is_file());
        shell_uninstall(&dd, &profiles).unwrap();
        assert_eq!(std::fs::read_to_string(&profiles[0]).unwrap(), "# mine\r\n");
        assert_eq!(std::fs::read_to_string(&profiles[1]).unwrap(), "");
    }

    #[test]
    fn sums_and_verification() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("quayflow.exe");
        std::fs::write(&f, b"hello").unwrap();
        let h = sha256_file(&f).unwrap();
        let sums = parse_sums(&format!("{h}  quayflow.exe\n{}  *other\n", "0".repeat(64)));
        assert_eq!(sums.len(), 2);
        verify_against(&sums, &f).unwrap();
        std::fs::write(&f, b"tampered").unwrap();
        assert!(verify_against(&sums, &f).unwrap_err().contains("mismatch"));
    }

    #[test]
    fn claim_is_consumed_once() {
        let d = tempfile::tempdir().unwrap();
        let dd = DataDir::at(d.path());
        std::fs::create_dir_all(dd.claims()).unwrap();
        std::fs::write(dd.claims().join("42.json"), r#"{"sessionId":"x"}"#).unwrap();
        assert_eq!(shell_claim(&dd, 42), 0);
        assert!(!dd.claims().join("42.json").exists());
        assert_eq!(shell_claim(&dd, 42), 0);
    }

    #[test]
    fn env_expansion() {
        std::env::set_var("QF_TEST_X", "C:\\Users\\u");
        assert_eq!(expand_env("%QF_TEST_X%\\Documents"), "C:\\Users\\u\\Documents");
        assert_eq!(expand_env("plain"), "plain");
    }
}
