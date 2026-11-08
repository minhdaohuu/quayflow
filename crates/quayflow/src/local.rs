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

/// macOS: a LaunchAgent that runs `quayflow start` when the user signs in. The engine runs in
/// the user's login session, which it needs to use the Keychain.
#[cfg(target_os = "macos")]
pub fn set_startup(on: bool) -> Result<(), String> {
    let plist = qf_adapter::env::home_dir()
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCH_AGENT}.plist"));
    // SAFETY: getuid has no preconditions.
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    let launchctl = |args: &[&str]| -> Result<(), String> {
        let o = std::process::Command::new("/bin/launchctl")
            .args(args)
            .output()
            .map_err(|e| format!("launchctl: {e}"))?;
        if o.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
        }
    };
    // Unload first: a loaded agent keeps the old definition.
    let _ = launchctl(&["bootout", &format!("{domain}/{LAUNCH_AGENT}")]);
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        if let Some(d) = plist.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        qf_vault::atomic_write(&plist, launch_agent(&exe).as_bytes()).map_err(|e| e.to_string())?;
        launchctl(&["bootstrap", &domain, &plist.display().to_string()])
    } else {
        if plist.exists() {
            std::fs::remove_file(&plist).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
const LAUNCH_AGENT: &str = "com.quayflow.engine";

/// The LaunchAgent that starts the engine at sign-in. `start` returns once the engine runs in
/// its own process group; `AbandonProcessGroup` keeps launchd from stopping anything when it
/// returns.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn launch_agent(exe: &Path) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>com.quayflow.engine</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{}</string>\n    <string>start</string>\n  </array>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>AbandonProcessGroup</key>\n  <true/>\n  <key>ProcessType</key>\n  <string>Background</string>\n</dict>\n</plist>\n",
        esc(&exe.display().to_string())
    )
}

/// Linux: a systemd user unit that runs `quayflow start` when the user's session manager
/// starts (a desktop or SSH sign-in; with `loginctl enable-linger`, at boot).
#[cfg(all(unix, not(target_os = "macos")))]
pub fn set_startup(on: bool) -> Result<(), String> {
    let unit_dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| qf_adapter::env::home_dir().join(".config"))
        .join("systemd")
        .join("user");
    let unit = unit_dir.join(STARTUP_UNIT);
    let systemctl = |args: &[&str]| -> Result<(), String> {
        let o = std::process::Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .map_err(|e| format!("systemctl: {e}"))?;
        if o.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
        }
    };
    if on {
        if qf_adapter::process::which("systemctl").is_none() {
            return Err(
                "no systemd user session here; add `quayflow start --if-consented` to your shell profile instead"
                    .into(),
            );
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        qf_vault::atomic_write(&unit, startup_unit(&exe).as_bytes()).map_err(|e| e.to_string())?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", STARTUP_UNIT])
    } else {
        if !unit.exists() {
            return Ok(());
        }
        let _ = systemctl(&["disable", STARTUP_UNIT]);
        std::fs::remove_file(&unit).map_err(|e| e.to_string())?;
        let _ = systemctl(&["daemon-reload"]);
        Ok(())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
const STARTUP_UNIT: &str = "quayflow.service";

/// The systemd user unit that starts the engine at sign-in. `start` returns once the engine
/// runs on its own; the unit then stays active until the engine is stopped.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
pub fn startup_unit(exe: &Path) -> String {
    format!(
        "[Unit]\nDescription=Quayflow engine (Claude account switching)\n\n[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=\"{}\" start\n\n[Install]\nWantedBy=default.target\n",
        exe.display()
    )
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

#[cfg_attr(not(windows), allow(dead_code))]
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

/// Whether a profile holds the managed block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockState {
    Absent,
    Complete,
    /// A start marker with no end marker after it: never edited automatically.
    Incomplete,
}

fn marker_line(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

/// Remove every complete managed block (inclusive) and report what was found. A start
/// marker with no end marker after it leaves the rest of the text exactly as it is.
pub fn strip_block_checked(text: &str) -> (String, BlockState) {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    let mut state = BlockState::Absent;
    let mut i = 0;
    while i < lines.len() {
        if marker_line(lines[i]) == BLOCK_START {
            match (i + 1..lines.len()).find(|&j| marker_line(lines[j]) == BLOCK_END) {
                Some(j) => {
                    state = BlockState::Complete;
                    i = j + 1;
                    continue;
                }
                None => {
                    lines[i..].iter().for_each(|l| out.push_str(l));
                    return (out, BlockState::Incomplete);
                }
            }
        }
        out.push_str(lines[i]);
        i += 1;
    }
    (out, state)
}

/// Remove the managed block (inclusive); keeps everything else byte for byte.
#[cfg(test)]
pub fn strip_block(text: &str) -> String {
    strip_block_checked(text).0
}

fn incomplete_block_message() -> String {
    format!(
        "has a Quayflow block start line ({BLOCK_START}) without its end line ({BLOCK_END}); left untouched. Remove the incomplete block by hand, then run this again"
    )
}

pub fn add_block(text: &str) -> Result<String, String> {
    let (mut base, state) = strip_block_checked(text);
    if state == BlockState::Incomplete {
        return Err(incomplete_block_message());
    }
    if !base.is_empty() && !base.ends_with('\n') {
        base.push_str("\r\n");
    }
    Ok(format!("{base}{}\r\n", profile_block()))
}

/// How a profile file is encoded on disk (kept when it is written back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

fn decode_utf16(bytes: &[u8], le: bool) -> Result<String, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err("starts with a UTF-16 byte order mark but has an odd number of bytes".into());
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| {
            if le {
                u16::from_le_bytes(c)
            } else {
                u16::from_be_bytes(c)
            }
        })
        .collect();
    String::from_utf16(&units).map_err(|_| "starts with a UTF-16 byte order mark but is not valid UTF-16".into())
}

/// Decode a text file by its byte order mark (UTF-8 with or without BOM, UTF-16 LE/BE with BOM).
/// Anything else (a legacy ANSI code page, UTF-16 without BOM) is refused, never guessed.
pub fn decode_text(bytes: &[u8]) -> Result<(String, TextEncoding), String> {
    let (text, enc) = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let s = std::str::from_utf8(rest).map_err(|_| "starts with a UTF-8 byte order mark but is not valid UTF-8")?;
        (s.to_string(), TextEncoding::Utf8Bom)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (decode_utf16(rest, true)?, TextEncoding::Utf16Le)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (decode_utf16(rest, false)?, TextEncoding::Utf16Be)
    } else {
        let s = std::str::from_utf8(bytes)
            .map_err(|_| "is neither UTF-8 nor UTF-16 with a byte order mark (probably a legacy ANSI code page)")?;
        (s.to_string(), TextEncoding::Utf8)
    };
    if text.contains('\0') {
        return Err("contains NUL characters (an encoding Quayflow does not recognize)".into());
    }
    Ok((text, enc))
}

pub fn encode_text(text: &str, enc: TextEncoding) -> Vec<u8> {
    match enc {
        TextEncoding::Utf8 => text.as_bytes().to_vec(),
        TextEncoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        TextEncoding::Utf16Le => [0xFF, 0xFE]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect(),
        TextEncoding::Utf16Be => [0xFE, 0xFF]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_be_bytes))
            .collect(),
    }
}

/// Read a profile. Only a missing file counts as empty; any other read or decode problem is
/// an error, so the file is never overwritten from a misread.
fn read_profile(p: &Path) -> Result<Option<(String, TextEncoding)>, String> {
    match std::fs::read(p) {
        Ok(b) => decode_text(&b).map(Some).map_err(|e| {
            format!(
                "{}: the file {e}; left untouched. Save it as UTF-8 and run this again",
                p.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: cannot read it ({e}); left untouched", p.display())),
    }
}

/// The file to write for `p`: the final target when `p` is a symbolic link, so the link stays.
fn write_target(p: &Path) -> PathBuf {
    let mut cur = p.to_path_buf();
    for _ in 0..32 {
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => match std::fs::read_link(&cur) {
                Ok(t) if t.is_absolute() => cur = t,
                Ok(t) => cur = cur.parent().map(|d| d.join(&t)).unwrap_or(t),
                Err(_) => break,
            },
            _ => break,
        }
    }
    cur
}

fn write_profile(p: &Path, text: &str, enc: TextEncoding) -> Result<(), String> {
    qf_vault::atomic_write(&write_target(p), &encode_text(text, enc)).map_err(|e| format!("{}: {e}", p.display()))
}

/// What a profile operation did: profiles handled, and profiles left untouched with the reason.
#[derive(Debug, Default)]
pub struct ProfileReport {
    pub done: Vec<String>,
    pub problems: Vec<String>,
}

fn install_profile(p: &Path) -> Result<(), String> {
    let existing = read_profile(p)?;
    let (text, enc) = existing.clone().unwrap_or((String::new(), TextEncoding::Utf8));
    let new = add_block(&text).map_err(|e| format!("{}: {e}", p.display()))?;
    if existing.is_some() && new == text {
        return Ok(());
    }
    write_profile(p, &new, enc)
}

pub fn shell_install(dd: &DataDir, profiles: &[PathBuf]) -> Result<ProfileReport, String> {
    std::fs::create_dir_all(dd.shell()).map_err(|e| e.to_string())?;
    qf_vault::atomic_write(&dd.shell().join("Quayflow.psm1"), PSM1.as_bytes()).map_err(|e| e.to_string())?;
    let mut r = ProfileReport::default();
    for p in profiles {
        match install_profile(p) {
            Ok(()) => r.done.push(p.display().to_string()),
            Err(e) => r.problems.push(e),
        }
    }
    Ok(r)
}

/// `quayflow shell refresh`: rewrite the installed shell module from this build (after an
/// upgrade). Profiles are not touched. Returns whether the module was rewritten.
pub fn shell_refresh(dd: &DataDir) -> Result<bool, String> {
    let p = dd.shell().join("Quayflow.psm1");
    if !p.exists() || std::fs::read(&p).map(|b| b == PSM1.as_bytes()).unwrap_or(false) {
        return Ok(false);
    }
    qf_vault::atomic_write(&p, PSM1.as_bytes()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Returns whether the block was removed.
fn uninstall_profile(p: &Path) -> Result<bool, String> {
    let (text, enc) = match read_profile(p) {
        Ok(Some(x)) => x,
        Ok(None) => return Ok(false),
        // A file Quayflow cannot decode that does not even hold the marker has nothing to remove.
        Err(e) => match std::fs::read(p) {
            Ok(b) if !b.windows(BLOCK_START.len()).any(|w| w == BLOCK_START.as_bytes()) => return Ok(false),
            _ => return Err(e),
        },
    };
    match strip_block_checked(&text) {
        (_, BlockState::Absent) => Ok(false),
        (_, BlockState::Incomplete) => Err(format!("{}: {}", p.display(), incomplete_block_message())),
        (stripped, BlockState::Complete) => write_profile(p, &stripped, enc).map(|()| true),
    }
}

pub fn shell_uninstall(dd: &DataDir, profiles: &[PathBuf]) -> ProfileReport {
    let mut r = ProfileReport::default();
    for p in profiles {
        match uninstall_profile(p) {
            Ok(true) => r.done.push(p.display().to_string()),
            Ok(false) => {}
            Err(e) => r.problems.push(e),
        }
    }
    // Keep the module while a profile still references it.
    if r.problems.is_empty() {
        let _ = std::fs::remove_file(dd.shell().join("Quayflow.psm1"));
    }
    r
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
        eprintln!("quayflow: Claude Code (claude) was not found on PATH");
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
            eprintln!("quayflow: could not start Claude Code: {e}");
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
    qf_adapter::process::which(cmd).is_some()
}

/// `gh attestation verify` of a downloaded release file: built by this repository's release
/// workflow from the tag `tag`.
fn verify_attestation(file: &Path, tag: &str) -> Result<(), String> {
    let o = qf_adapter::process::hidden("gh")
        .args(["attestation", "verify"])
        .arg(file)
        .args(["--repo", REPO, "--signer-workflow"])
        .arg(format!("{REPO}/.github/workflows/release.yml"))
        .arg("--source-ref")
        .arg(format!("refs/tags/{tag}"))
        .output()
        .map_err(|e| format!("could not run gh attestation verify: {e}"))?;
    if o.status.success() {
        return Ok(());
    }
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Err(format!(
        "REFUSED: attestation verification failed for {name} of {tag}. Nothing was installed.\n{}",
        String::from_utf8_lossy(&o.stderr).trim()
    ))
}

fn gh_signed_in() -> bool {
    qf_adapter::process::hidden("gh")
        .args(["auth", "status", "--hostname", "github.com"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn http_agent() -> Result<ureq::Agent, String> {
    Ok(ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(
            native_tls::TlsConnector::new().map_err(|e| e.to_string())?,
        ))
        .timeout(Duration::from_secs(300))
        .build())
}

/// The tag of the latest release (the Mod is pinned to the same tag as the engine).
fn latest_tag(gh_ready: bool) -> Result<String, String> {
    if gh_ready {
        if let Ok(o) = qf_adapter::process::hidden("gh")
            .args([
                "release", "view", "--repo", REPO, "--json", "tagName", "--jq", ".tagName",
            ])
            .output()
        {
            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if o.status.success() && !t.is_empty() {
                return Ok(t);
            }
        }
    }
    let fail = |e: String| {
        format!("could not find the latest release of {REPO} ({e}). If the repository is private, sign in with `gh auth login` and run this again")
    };
    let v: Value = http_agent()?
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("User-Agent", "quayflow-cli")
        .call()
        .map_err(|e| fail(e.to_string()))?
        .into_json()
        .map_err(|e| fail(e.to_string()))?;
    v["tag_name"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| fail("GitHub's answer names no release tag".into()))
}

fn download(assets: &[&str], dest: &Path, tag: &str, gh_ready: bool) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    if gh_ready {
        let mut c = qf_adapter::process::hidden("gh");
        c.args(["release", "download", tag, "--repo", REPO, "--clobber", "-D"])
            .arg(dest);
        for a in assets {
            c.args(["-p", a]);
        }
        let o = c.output().map_err(|e| e.to_string())?;
        if o.status.success() {
            return Ok(());
        }
    }
    let agent = http_agent()?;
    for a in assets {
        let url = format!("https://github.com/{REPO}/releases/download/{tag}/{a}");
        let resp = agent.get(&url).call().map_err(|e| match e {
            ureq::Error::Status(404, _) => format!(
                "no published release found at https://github.com/{REPO}/releases/tag/{tag} (no such release, or the repository is private and the GitHub CLI is not signed in)"
            ),
            e => format!("{a}: {e}"),
        })?;
        let mut bytes = vec![];
        std::io::Read::read_to_end(&mut resp.into_reader(), &mut bytes).map_err(|e| e.to_string())?;
        std::fs::write(dest.join(a), bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The release assets this platform installs, and their names in the program folder.
#[cfg(windows)]
pub const PROGRAMS: &[(&str, &str)] = &[("quayflow.exe", "quayflow.exe"), ("quayflow-bg.exe", "quayflow-bg.exe")];
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const PROGRAMS: &[(&str, &str)] = &[("quayflow-linux-x86_64", "quayflow")];
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const PROGRAMS: &[(&str, &str)] = &[("quayflow-macos-arm64", "quayflow")];
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const PROGRAMS: &[(&str, &str)] = &[("quayflow-macos-x86_64", "quayflow")];
#[cfg(not(any(
    windows,
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", any(target_arch = "aarch64", target_arch = "x86_64"))
)))]
pub const PROGRAMS: &[(&str, &str)] = &[];

/// The release's installer for this platform.
pub const INSTALLER: &str = if cfg!(windows) { "install.ps1" } else { "install.sh" };

/// Arguments for the release's installer in update mode: no consent prompt, no PATH change,
/// consents left as they are. A local `source` is installed engine-only (the Mod is not moved).
/// `posix` selects `install.sh`'s options (`--update`), else `install.ps1`'s (`-Update`).
pub fn installer_args(
    script: &Path,
    tag: Option<&str>,
    source: Option<&Path>,
    skip_attestation: bool,
    posix: bool,
) -> Vec<String> {
    let opt = |win: &str, sh: &str| if posix { sh.to_string() } else { win.to_string() };
    let mut v: Vec<String> = if posix {
        vec![]
    } else {
        ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    };
    v.push(script.display().to_string());
    v.push(opt("-Update", "--update"));
    if let Some(t) = tag {
        v.extend([opt("-Version", "--version"), t.to_string()]);
    }
    if let Some(s) = source {
        v.extend([
            opt("-Source", "--source"),
            s.display().to_string(),
            opt("-NoPlugin", "--no-plugin"),
        ]);
    }
    if skip_attestation {
        v.push(opt("-SkipAttestation", "--skip-attestation"));
    }
    v
}

/// The folder the installer installs into on this platform.
fn installer_root() -> Option<PathBuf> {
    if cfg!(windows) && std::env::var_os("LOCALAPPDATA").is_none() {
        return None;
    }
    Some(qf_engine::paths::default_root())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// `quayflow update`: compare a release with the installed programs, then hand the install to
/// that release's own `install.ps1` (hash-checked against its SHA256SUMS). The installer verifies
/// the files and their attestation, replaces the programs, moves the Mod to the same release and
/// restarts the engine. `source` is a local release folder (testing, private use).
pub fn update(
    dd: &DataDir,
    check_only: bool,
    version: Option<&str>,
    skip_attestation: bool,
    source: Option<&Path>,
) -> Result<String, String> {
    if source.is_none() && std::env::var_os("QUAYFLOW_OFFLINE").is_some() {
        return Err("network access is disabled (QUAYFLOW_OFFLINE is set)".into());
    }
    if !check_only {
        match installer_root() {
            Some(r) if same_dir(&r, &dd.root) => {}
            r => {
                return Err(format!(
                "quayflow update installs into {}, but this command uses {} (QUAYFLOW_HOME); run the installer instead",
                r.map(|r| r.display().to_string())
                    .unwrap_or_else(|| "the default data folder".into()),
                dd.root.display()
            ))
            }
        }
    }
    let gh_installed = have("gh");
    let gh_ready = gh_installed && gh_signed_in();
    // Same rule as the installer: refuse before downloading anything.
    if !check_only && !skip_attestation && !gh_ready {
        let install_gh = if cfg!(windows) {
            "  winget install --id GitHub.cli -e\n  gh auth login"
        } else if cfg!(target_os = "macos") {
            "  brew install gh   (or see https://cli.github.com)\n  gh auth login"
        } else {
            "  sudo apt install gh   (or see https://cli.github.com)\n  gh auth login"
        };
        let (why, fix) = if gh_installed {
            ("the GitHub CLI is not signed in", "  gh auth login")
        } else {
            ("the GitHub CLI (gh) is not installed", install_gh)
        };
        return Err(format!(
            "REFUSED: the build-provenance attestation cannot be verified because {why}. Nothing was installed.\nThe attestation proves the programs were built by this repository's release workflow. To check it, run:\n{fix}\nthen open a new terminal and run quayflow update again.\nOr update with the SHA-256 check alone (it proves the files match the release's SHA256SUMS, not who built them): quayflow update --skip-attestation"
        ));
    }
    let tag = match (source, version) {
        (Some(_), _) => None,
        (None, Some(v)) => Some(v.to_string()),
        (None, None) => Some(latest_tag(gh_ready)?),
    };
    let tmp = dd.root.join("update");
    let _ = std::fs::remove_dir_all(&tmp);
    if PROGRAMS.is_empty() {
        return Err("no Quayflow release is built for this platform".into());
    }
    let mut wanted = vec!["SHA256SUMS"];
    if !check_only {
        wanted.push(INSTALLER);
    }
    let fetched = match (source, tag.as_deref()) {
        (Some(s), _) => std::fs::create_dir_all(&tmp).map_err(|e| e.to_string()).and_then(|_| {
            wanted.iter().try_for_each(|a| {
                std::fs::copy(s.join(a), tmp.join(a))
                    .map(|_| ())
                    .map_err(|e| format!("{}: {e}", s.join(a).display()))
            })
        }),
        (None, Some(t)) => download(&wanted, &tmp, t, gh_ready),
        (None, None) => Err("no release to update from".into()),
    };
    let result = fetched.and_then(|_| hand_off(dd, &tmp, check_only, tag.as_deref(), source, skip_attestation));
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn hand_off(
    dd: &DataDir,
    tmp: &Path,
    check_only: bool,
    tag: Option<&str>,
    source: Option<&Path>,
    skip_attestation: bool,
) -> Result<String, String> {
    let sums = parse_sums(&std::fs::read_to_string(tmp.join("SHA256SUMS")).map_err(|e| e.to_string())?);
    let bin = dd.bin();
    let same = PROGRAMS.iter().all(|(asset, installed)| {
        let listed = sums.iter().find(|(_, n)| n == asset).map(|(h, _)| h.clone());
        matches!((listed, sha256_file(&bin.join(installed))), (Some(x), Ok(y)) if x == y)
    });
    let shown = tag.unwrap_or("the local build");
    // A local source is always installed: that is how a build is tested.
    if same && source.is_none() {
        return Ok(format!("Quayflow is up to date ({shown})."));
    }
    if check_only {
        return Ok(if same {
            format!("The programs in {shown} match the installed ones.")
        } else {
            format!(
                "Quayflow {shown} is available. Run `quayflow update` to install it (the files and their attestation are verified first)."
            )
        });
    }
    let script = tmp.join(INSTALLER);
    verify_against(&sums, &script)?;
    // SHA256SUMS comes from the same release, so the hash alone does not say who built the
    // installer: check its attestation before running it, as the installer checks the programs.
    if let (Some(tag), false, None) = (tag, skip_attestation, source) {
        verify_attestation(&script, tag)?;
    }
    let text = std::fs::read_to_string(&script).map_err(|e| e.to_string())?;
    let update_mode = if cfg!(windows) {
        "[switch]$Update"
    } else {
        "quayflow-installer: update-mode"
    };
    if !text.contains(update_mode) {
        return Err(format!(
            "the installer of {shown} has no update mode (releases before v0.1.4); run that release's installer directly (docs/user-guide.md, Install)"
        ));
    }
    // Never replace components mid-transition (the installer stops and restarts the engine).
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
    }
    let shell = if !cfg!(windows) {
        "sh"
    } else if have("pwsh") {
        "pwsh"
    } else {
        "powershell"
    };
    // The installer talks to the person directly (progress, warnings): its output is not captured.
    let status = std::process::Command::new(shell)
        .args(installer_args(&script, tag, source, skip_attestation, !cfg!(windows)))
        .status()
        .map_err(|e| format!("could not run the installer with {shell}: {e}"))?;
    if status.success() {
        Ok(format!("Quayflow updated to {shown}."))
    } else {
        Err(format!(
            "the installer stopped (exit {}); see its messages above",
            status.code().unwrap_or(-1)
        ))
    }
}

// ---------------- uninstall ----------------

/// `raw` (the user PATH as stored) without the entries naming `dir`, or None when no entry does.
/// Compares case-insensitively, ignoring trailing slashes, in both raw (`%VAR%`) and expanded form.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn path_without_entry(raw: &str, dir: &Path, expand: impl Fn(&str) -> String) -> Option<String> {
    let norm = |s: &str| s.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let d = dir.to_string_lossy();
    let targets = [norm(&d), norm(&expand(&d))];
    let parts: Vec<&str> = raw.split(';').collect();
    let kept: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|e| {
            e.trim().is_empty() || {
                let (r, x) = (norm(e), norm(&expand(e)));
                !targets.iter().any(|t| *t == r || *t == x)
            }
        })
        .collect();
    (kept.len() != parts.len()).then(|| kept.join(";"))
}

/// Tell running programs (Explorer, new terminals) that the user environment changed.
#[cfg(windows)]
fn broadcast_environment_change() {
    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }
    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    let name: Vec<u16> = "Environment".encode_utf16().chain([0]).collect();
    let mut result = 0usize;
    // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            name.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}

/// Remove the Quayflow folder from the user PATH, keeping the value's registry type
/// (REG_EXPAND_SZ stays REG_EXPAND_SZ). Returns whether PATH changed.
#[cfg(windows)]
fn remove_from_user_path(dir: &Path) -> Result<bool, String> {
    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    let k = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ | KEY_SET_VALUE)
        .map_err(|e| e.to_string())?;
    let v = match k.get_raw_value("Path") {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    if !matches!(v.vtype, RegType::REG_SZ | RegType::REG_EXPAND_SZ) || !v.bytes.len().is_multiple_of(2) {
        return Err("the user PATH has an unexpected registry type; left untouched".into());
    }
    let units: Vec<u16> = v
        .bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    let raw = String::from_utf16(&units[..end]).map_err(|_| "the user PATH is not valid text; left untouched")?;
    let Some(new) = path_without_entry(&raw, dir, expand_env) else {
        return Ok(false);
    };
    let bytes = new.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    k.set_raw_value("Path", &winreg::RegValue { bytes, vtype: v.vtype })
        .map_err(|e| e.to_string())?;
    broadcast_environment_change();
    Ok(true)
}

/// Linux: the installer links `~/.local/bin/quayflow` to the program; remove that link
/// when it points into `dir`.
#[cfg(not(windows))]
fn remove_from_user_path(dir: &Path) -> Result<bool, String> {
    let link = qf_adapter::env::home_dir().join(".local").join("bin").join("quayflow");
    match std::fs::read_link(&link) {
        Ok(target) if target.starts_with(dir) => std::fs::remove_file(&link).map(|_| true).map_err(|e| e.to_string()),
        _ => Ok(false),
    }
}

/// Lane names (folders under `lanes\`).
pub fn lane_names(dd: &DataDir) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dd.lanes())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// Lanes with a running Claude session.
pub fn busy_lanes(dd: &DataDir) -> Vec<String> {
    lane_names(dd)
        .into_iter()
        .filter(|n| !qf_adapter::sessions::read_sessions(&qf_adapter::lanes::lane_env(&dd.lanes(), n)).is_empty())
        .collect()
}

/// What `quayflow uninstall` deletes besides the program, startup entry and prompt hook.
/// Each choice is separate and off by default; `all` (`--delete-data`) is both plus
/// settings, lanes and the other Quayflow data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteChoice {
    /// The protected vault copies of registered logins (and leftover sign-in copies).
    pub vault: bool,
    /// Routine logs and saved support reports (and the journal, unless a record is unresolved).
    pub logs: bool,
    pub all: bool,
}

impl DeleteChoice {
    pub fn from_flags(delete_vault: bool, delete_logs: bool, delete_data: bool) -> Self {
        DeleteChoice {
            vault: delete_vault || delete_data,
            logs: delete_logs || delete_data,
            all: delete_data,
        }
    }
}

/// Whether the journal holds a transition that was requested and never resolved.
fn unresolved_journal(dd: &DataDir) -> bool {
    let journal = qf_engine::journal::Journal::new(dd.journal());
    let all = journal.read_all();
    all.iter().any(|r| r["event"] == "transition.requested")
        && !all
            .iter()
            .any(|r| r["event"] == "transition.reconciled" || r["event"] == "transition.written")
}

pub fn uninstall(dd: &DataDir, delete: DeleteChoice) -> Result<Vec<String>, String> {
    let delete_data = delete.all;
    if delete_data {
        let busy = busy_lanes(dd);
        if !busy.is_empty() {
            return Err(format!(
                "Claude sessions are running in lane(s) {}; end them first. Nothing was changed",
                busy.join(", ")
            ));
        }
    }
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
    let r = shell_uninstall(dd, &profile_paths());
    if !r.done.is_empty() {
        notes.push(format!("prompt hook removed from {}", r.done.join(", ")));
    }
    for p in r.problems {
        notes.push(format!("prompt hook NOT removed: {p}"));
    }
    match remove_from_user_path(&dd.bin()) {
        Ok(true) if cfg!(windows) => notes.push(format!(
            "{} removed from your user PATH (new terminals)",
            dd.bin().display()
        )),
        Ok(true) => notes.push("~/.local/bin/quayflow link removed".into()),
        Ok(false) => {}
        Err(e) => notes.push(format!("user PATH: {e}")),
    }
    notes.push("Claude's current login is left as it is: your active account keeps working".into());
    let unresolved = unresolved_journal(dd);
    if delete_data {
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
            "vault, logs, settings and lanes (with their logins and conversation history) deleted; protected backups of unknown logins are kept in {} (delete them yourself when no longer needed)",
            dd.backups().display()
        ));
    } else {
        if delete.vault {
            let _ = std::fs::remove_dir_all(dd.vault());
            let _ = std::fs::remove_dir_all(dd.signin_dir());
            notes.push(format!(
                "vault deleted (the protected copies of your registered logins); protected backups of unknown logins are kept in {}",
                dd.backups().display()
            ));
        } else {
            notes.push(format!("vault kept in {}", dd.vault().display()));
        }
        if delete.logs {
            let _ = std::fs::remove_dir_all(dd.logs());
            let _ = std::fs::remove_dir_all(dd.reports());
            if unresolved {
                notes.push(format!(
                    "logs and support reports deleted; kept {} (unresolved recovery record)",
                    dd.journal().display()
                ));
            } else {
                let _ = std::fs::remove_file(dd.journal());
                notes.push("logs, journal and support reports deleted".into());
            }
        } else {
            notes.push(format!("logs kept in {}", dd.logs().display()));
        }
        notes.push(format!(
            "settings and lanes kept in {} (quayflow uninstall --delete-data removes everything)",
            dd.root.display()
        ));
    }
    // The running executable cannot delete itself: remove the program folder after exit.
    let bin = dd.bin();
    if bin.exists() {
        match spawn_cleanup(&bin) {
            Ok(()) => notes.push(format!(
                "program files in {} are removed in a few seconds",
                bin.display()
            )),
            Err(e) => notes.push(format!("could not remove {} ({e}); delete it yourself", bin.display())),
        }
    }
    Ok(notes)
}

/// The `cmd.exe` command line that deletes `bin` after this process has exited. Passed verbatim
/// (Rust's argument quoting would turn the inner quotes into \" which cmd.exe does not understand).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn cleanup_command_line(bin: &Path) -> String {
    format!("/d /c ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{}\"", bin.display())
}

#[cfg(windows)]
fn spawn_cleanup(bin: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    qf_adapter::process::hidden("cmd.exe")
        .raw_arg(cleanup_command_line(bin))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
fn spawn_cleanup(bin: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstall_deletions_are_separate_choices() {
        assert_eq!(DeleteChoice::from_flags(false, false, false), DeleteChoice::default());
        let v = DeleteChoice::from_flags(true, false, false);
        assert!(v.vault && !v.logs && !v.all);
        let l = DeleteChoice::from_flags(false, true, false);
        assert!(!l.vault && l.logs && !l.all);
        // --delete-data stays the full deletion: both choices and the rest.
        let d = DeleteChoice::from_flags(false, false, true);
        assert!(d.vault && d.logs && d.all);
    }

    #[test]
    fn profile_block_round_trip_preserves_user_content() {
        let user = "Set-Alias ll Get-ChildItem\r\nfunction prompt { 'x> ' }\r\n";
        let with = add_block(user).unwrap();
        assert!(with.starts_with(user));
        assert!(with.contains("Enable-QuayflowPromptHook"));
        assert_eq!(add_block(&with).unwrap(), with, "idempotent");
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
        let r = shell_install(&dd, &profiles).unwrap();
        assert_eq!(r.done.len(), 2);
        assert!(r.problems.is_empty());
        assert!(dd.shell().join("Quayflow.psm1").is_file());
        let r = shell_uninstall(&dd, &profiles);
        assert_eq!(r.done.len(), 2);
        assert!(r.problems.is_empty());
        assert_eq!(std::fs::read_to_string(&profiles[0]).unwrap(), "# mine\r\n");
        assert_eq!(std::fs::read_to_string(&profiles[1]).unwrap(), "");
    }

    #[test]
    fn strip_block_keeps_text_when_end_marker_is_missing() {
        let user = "a\r\n";
        let tail = format!("{BLOCK_START}\r\n$x = 1\r\nfunction mine {{ 'keep me' }}\r\n");
        let broken = format!("{user}{tail}");
        assert_eq!(strip_block_checked(&broken), (broken.clone(), BlockState::Incomplete));
        assert_eq!(strip_block(&broken), broken);
        assert!(add_block(&broken).unwrap_err().contains("without its end line"));
        // A complete block followed by a broken one: the complete one goes, the rest stays.
        let both = format!("{user}{}\r\nb\r\n{tail}", profile_block());
        assert_eq!(
            strip_block_checked(&both),
            (format!("{user}b\r\n{tail}"), BlockState::Incomplete)
        );
        assert_eq!(strip_block_checked("x\n").1, BlockState::Absent);
        assert_eq!(strip_block_checked(&add_block("x\n").unwrap()).1, BlockState::Complete);
    }

    #[test]
    fn shell_install_and_uninstall_refuse_an_incomplete_block() {
        let d = tempfile::tempdir().unwrap();
        let dd = DataDir::at(&d.path().join("qf"));
        let p = d.path().join("p.ps1");
        let broken = format!("# mine\r\n{BLOCK_START}\r\nfunction mine {{}}\r\n");
        std::fs::write(&p, &broken).unwrap();
        let r = shell_install(&dd, std::slice::from_ref(&p)).unwrap();
        assert!(r.done.is_empty());
        assert_eq!(r.problems.len(), 1, "{:?}", r.problems);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), broken);
        let r = shell_uninstall(&dd, std::slice::from_ref(&p));
        assert_eq!(r.problems.len(), 1);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), broken);
    }

    #[test]
    fn decode_and_encode_by_bom() {
        let text = "# héllo\r\nWrite-Host 'ü'\r\n";
        for enc in [
            TextEncoding::Utf8,
            TextEncoding::Utf8Bom,
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be,
        ] {
            let bytes = encode_text(text, enc);
            assert_eq!(decode_text(&bytes).unwrap(), (text.to_string(), enc), "{enc:?}");
        }
        assert_eq!(&encode_text("a", TextEncoding::Utf16Le), &[0xFF, 0xFE, b'a', 0]);
        assert_eq!(&encode_text("a", TextEncoding::Utf8Bom), &[0xEF, 0xBB, 0xBF, b'a']);
        // ANSI (Windows-1252 "é"), UTF-16 without BOM, broken UTF-16: refused.
        assert!(decode_text(b"# caf\xe9\r\n").is_err());
        assert!(decode_text(&[b'a', 0, b'b', 0]).is_err());
        assert!(decode_text(&[0xFF, 0xFE, b'a']).is_err());
    }

    #[test]
    fn shell_install_keeps_profile_encoding_and_refuses_undecodable_files() {
        let d = tempfile::tempdir().unwrap();
        let dd = DataDir::at(&d.path().join("qf"));
        let user = "# mine ü\r\nSet-Alias ll Get-ChildItem\r\n";
        for enc in [TextEncoding::Utf8Bom, TextEncoding::Utf16Le, TextEncoding::Utf16Be] {
            let p = d.path().join(format!("{enc:?}.ps1"));
            std::fs::write(&p, encode_text(user, enc)).unwrap();
            let r = shell_install(&dd, std::slice::from_ref(&p)).unwrap();
            assert!(r.problems.is_empty(), "{:?}", r.problems);
            let (text, got) = decode_text(&std::fs::read(&p).unwrap()).unwrap();
            assert_eq!(got, enc);
            assert!(text.starts_with(user) && text.contains(BLOCK_START) && text.contains(BLOCK_END));
            let r = shell_uninstall(&dd, std::slice::from_ref(&p));
            assert_eq!(r.done.len(), 1);
            assert_eq!(std::fs::read(&p).unwrap(), encode_text(user, enc), "{enc:?} round trip");
        }
        // ANSI profile: install refuses and leaves it byte for byte; uninstall has nothing to do.
        let p = d.path().join("ansi.ps1");
        let ansi = b"# caf\xe9\r\nSet-Alias ll Get-ChildItem\r\n".to_vec();
        std::fs::write(&p, &ansi).unwrap();
        let r = shell_install(&dd, std::slice::from_ref(&p)).unwrap();
        assert!(r.done.is_empty());
        assert!(r.problems[0].contains("left untouched"), "{:?}", r.problems);
        assert_eq!(std::fs::read(&p).unwrap(), ansi);
        let r = shell_uninstall(&dd, std::slice::from_ref(&p));
        assert!(r.done.is_empty() && r.problems.is_empty());
        assert_eq!(std::fs::read(&p).unwrap(), ansi);
        // Unreadable (a folder where the profile should be): an error, not "empty".
        let p = d.path().join("dir.ps1");
        std::fs::create_dir_all(&p).unwrap();
        let r = shell_install(&dd, std::slice::from_ref(&p)).unwrap();
        assert_eq!(r.problems.len(), 1);
        assert!(p.is_dir());
    }

    #[cfg(windows)]
    #[test]
    fn shell_install_writes_through_a_symlinked_profile() {
        let d = tempfile::tempdir().unwrap();
        let dd = DataDir::at(&d.path().join("qf"));
        let target = d.path().join("dotfiles").join("profile.ps1");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "# mine\r\n").unwrap();
        let link = d.path().join("Microsoft.PowerShell_profile.ps1");
        if std::os::windows::fs::symlink_file(&target, &link).is_err() {
            eprintln!("symbolic links need Developer Mode or elevation here; skipped");
            return;
        }
        shell_install(&dd, std::slice::from_ref(&link)).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert!(std::fs::read_to_string(&target).unwrap().contains(BLOCK_START));
        shell_uninstall(&dd, std::slice::from_ref(&link));
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "# mine\r\n");
    }

    #[test]
    fn path_edit_removes_only_the_quayflow_entry() {
        let expand = |s: &str| s.replace("%LOCALAPPDATA%", r"C:\Users\u\AppData\Local");
        let bin = Path::new(r"C:\Users\u\AppData\Local\Quayflow\bin");
        let raw = r"%USERPROFILE%\bin;C:\Users\u\AppData\Local\Quayflow\bin;C:\tools";
        assert_eq!(
            path_without_entry(raw, bin, expand).as_deref(),
            Some(r"%USERPROFILE%\bin;C:\tools")
        );
        // Case, trailing slash and the unexpanded form all match.
        let raw = r"C:\a;c:\users\U\appdata\local\quayflow\BIN\;%LOCALAPPDATA%\Quayflow\bin;C:\b";
        assert_eq!(path_without_entry(raw, bin, expand).as_deref(), Some(r"C:\a;C:\b"));
        // No entry: no rewrite at all.
        assert_eq!(path_without_entry(r"%USERPROFILE%\bin;C:\tools", bin, expand), None);
        assert_eq!(path_without_entry("", bin, expand), None);
        // Similar-looking entries stay.
        assert_eq!(
            path_without_entry(r"C:\Users\u\AppData\Local\Quayflow\bin2", bin, expand),
            None
        );
    }

    #[test]
    fn cleanup_command_keeps_quotes_for_cmd() {
        let s = cleanup_command_line(Path::new(r"C:\Users\A B\AppData\Local\Quayflow\bin"));
        assert!(
            s.ends_with(r#"rmdir /s /q "C:\Users\A B\AppData\Local\Quayflow\bin""#),
            "{s}"
        );
        assert!(!s.contains("\\\""), "{s}");
    }

    #[test]
    fn update_hands_off_to_the_installer_in_update_mode() {
        let s = Path::new(r"C:\q\update\install.ps1");
        let a = installer_args(s, Some("v0.1.4"), None, false, false);
        assert_eq!(
            a,
            [
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                r"C:\q\update\install.ps1",
                "-Update",
                "-Version",
                "v0.1.4"
            ]
        );
        // A local build: engine only, the Mod stays where it is.
        let a = installer_args(s, None, Some(Path::new(r"C:\dist")), true, false);
        assert!(a.ends_with(&[
            "-Update".to_string(),
            "-Source".into(),
            r"C:\dist".into(),
            "-NoPlugin".into(),
            "-SkipAttestation".into()
        ]));
        assert!(!a.iter().any(|x| x == "-Version" || x == "-Yes" || x == "-Startup"));
        // install.sh takes the same choices as long options, run by `sh`.
        let a = installer_args(Path::new("/q/update/install.sh"), Some("v0.1.5"), None, true, true);
        assert_eq!(
            a,
            [
                "/q/update/install.sh",
                "--update",
                "--version",
                "v0.1.5",
                "--skip-attestation"
            ]
        );
        let a = installer_args(Path::new("install.sh"), None, Some(Path::new("/dist")), false, true);
        assert_eq!(a, ["install.sh", "--update", "--source", "/dist", "--no-plugin"]);
    }

    #[test]
    fn the_launch_agent_runs_quayflow_start() {
        let a = launch_agent(Path::new("/Users/u/.local/share/quayflow/bin/quayflow"));
        assert!(a.contains("<string>/Users/u/.local/share/quayflow/bin/quayflow</string>\n    <string>start</string>"));
        assert!(a.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(a.contains("<key>AbandonProcessGroup</key>\n  <true/>"));
        assert!(launch_agent(Path::new("/a&b/q")).contains("/a&amp;b/q"));
    }

    #[test]
    fn the_startup_unit_runs_quayflow_start() {
        let u = startup_unit(Path::new("/home/u/.local/share/quayflow/bin/quayflow"));
        assert!(
            u.contains("ExecStart=\"/home/u/.local/share/quayflow/bin/quayflow\" start\n"),
            "{u}"
        );
        assert!(u.contains("WantedBy=default.target"));
    }

    #[test]
    fn shell_refresh_rewrites_only_an_installed_module() {
        let d = tempfile::tempdir().unwrap();
        let dd = DataDir::at(&d.path().join("qf"));
        assert!(!shell_refresh(&dd).unwrap());
        assert!(!dd.shell().join("Quayflow.psm1").exists());
        std::fs::create_dir_all(dd.shell()).unwrap();
        std::fs::write(dd.shell().join("Quayflow.psm1"), "old").unwrap();
        assert!(shell_refresh(&dd).unwrap());
        assert_eq!(std::fs::read_to_string(dd.shell().join("Quayflow.psm1")).unwrap(), PSM1);
        assert!(!shell_refresh(&dd).unwrap());
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
