//! Process helpers: liveness, parent process, window-less and new-console launches.
//! Windows uses Win32; Linux reads `/proc`.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Command, Stdio};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const DETACHED_PROCESS: u32 = 0x0000_0008;
pub const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
pub const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

#[cfg(windows)]
pub fn is_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    if pid == 0 {
        return false;
    }
    // SAFETY: plain Win32 calls with a handle we close.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            // Access denied still means it exists.
            return std::io::Error::last_os_error().raw_os_error() == Some(5);
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

#[cfg(not(windows))]
pub fn is_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Creation time of a running process, in Unix milliseconds (`None` when unknown).
#[cfg(windows)]
pub fn start_time_ms(pid: u32) -> Option<i64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    if pid == 0 {
        return None;
    }
    // SAFETY: plain Win32 calls with a handle we close; FILETIME out-params are plain data.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        let ok = GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        // 100 ns ticks since 1601-01-01 to Unix milliseconds.
        Some((ticks as i64 - 116_444_736_000_000_000) / 10_000)
    }
}

/// Fields of `/proc/<pid>/stat` after the command name (which may hold spaces): index 0 is
/// the state (field 3), 1 the parent pid (field 4), 19 the start time in clock ticks (field 22).
#[cfg(not(windows))]
fn proc_stat(pid: u32) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &text[text.rfind(')')? + 1..];
    Some(rest.split_whitespace().map(String::from).collect())
}

#[cfg(not(windows))]
pub fn start_time_ms(pid: u32) -> Option<i64> {
    if pid == 0 {
        return None;
    }
    let ticks: i64 = proc_stat(pid)?.get(19)?.parse().ok()?;
    // Boot time (seconds since the epoch) plus the start offset; Linux reports clock ticks
    // at USER_HZ, which is 100 on every supported architecture.
    let btime: i64 = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())?;
    Some(btime * 1000 + ticks * 10)
}

/// Parent process id (Toolhelp snapshot).
#[cfg(windows)]
pub fn parent_pid(pid: u32) -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    // SAFETY: snapshot handle closed; entry struct sized as required.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                if e.th32ProcessID == pid {
                    found = Some(e.th32ParentProcessID);
                    break;
                }
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        found
    }
}

#[cfg(not(windows))]
pub fn parent_pid(pid: u32) -> Option<u32> {
    proc_stat(pid)?.get(1)?.parse().ok()
}

/// Image name of a process (lower-case, e.g. "pwsh.exe").
#[cfg(windows)]
pub fn process_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    // SAFETY: as in parent_pid.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                if e.th32ProcessID == pid {
                    let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                    found = Some(String::from_utf16_lossy(&e.szExeFile[..len]).to_ascii_lowercase());
                    break;
                }
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        found
    }
}

#[cfg(not(windows))]
pub fn process_name(pid: u32) -> Option<String> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(name.trim().to_ascii_lowercase())
}

/// The nearest PowerShell ancestor of a process (the terminal a Claude session runs in).
pub fn powershell_ancestor(pid: u32) -> Option<(u32, String)> {
    let mut cur = pid;
    for _ in 0..6 {
        let parent = parent_pid(cur)?;
        if parent == 0 || parent == cur {
            return None;
        }
        if let Some(name) = process_name(parent) {
            if matches!(name.as_str(), "pwsh.exe" | "powershell.exe" | "pwsh") {
                return Some((parent, name));
            }
        }
        cur = parent;
    }
    None
}

/// Build a command that never shows a console window.
pub fn hidden(program: impl AsRef<OsStr>) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Make this process's standard handles non-inheritable, so a long-lived child never holds
/// the caller's pipes open (a piped `quayflow start` must return as soon as it prints).
#[cfg(windows)]
fn uninherit_std_handles() {
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: plain Win32 calls on this process's own standard handles.
        unsafe {
            let h = GetStdHandle(which);
            if !h.is_null() && h as isize != -1 {
                SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// Start a detached background process with no window and no inherited handles.
pub fn spawn_detached(exe: &Path, args: &[&str]) -> std::io::Result<Child> {
    #[cfg(windows)]
    uninherit_std_handles();
    let mut c = Command::new(exe);
    c.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
    }
    // Its own process group: a signal to the caller's group (a closed terminal, Ctrl+C in
    // the session that started it) does not reach the engine.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    match c.spawn() {
        Ok(child) => Ok(child),
        // Breakaway is refused inside some job objects; retry without it.
        Err(_) => {
            let mut c = Command::new(exe);
            c.args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                c.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            }
            c.spawn()
        }
    }
}

/// Open a new console window running a command (user-visible on purpose: sign-in, Open New Session).
/// On Linux this needs a desktop session and a terminal emulator; without one it fails, and
/// the caller shows the command to run instead.
pub fn spawn_new_console(
    exe: &Path,
    args: &[String],
    env: &[(&str, Option<&OsStr>)],
    cwd: Option<&Path>,
) -> std::io::Result<Child> {
    #[cfg(windows)]
    let mut c = {
        let mut c = Command::new(exe);
        c.args(args);
        c
    };
    #[cfg(not(windows))]
    let mut c = {
        let desktop = desktop_env();
        let graphical = desktop.iter().any(|(k, _)| k == "DISPLAY" || k == "WAYLAND_DISPLAY");
        let term = graphical
            .then(|| TERMINALS.iter().find_map(|t| which(t)))
            .flatten()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no terminal window available"))?;
        let mut c = Command::new(&term);
        // Every listed terminal takes `-e <program> <args>`; gnome-terminal wants `--`.
        if term.file_name().is_some_and(|n| n == "gnome-terminal") {
            c.arg("--");
        } else {
            c.arg("-e");
        }
        c.arg(exe).args(args);
        c.envs(desktop);
        std::os::unix::process::CommandExt::process_group(&mut c, 0);
        c
    };
    for (k, v) in env {
        match v {
            Some(v) => {
                c.env(k, v);
            }
            None => {
                c.env_remove(k);
            }
        }
    }
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NEW_CONSOLE | CREATE_BREAKAWAY_FROM_JOB);
    }
    c.spawn()
}

/// The desktop variables a new window needs on Linux: the display, the session bus, and what
/// `xdg-open` uses to find the browser (a snap browser is listed only through the desktop's
/// `XDG_DATA_DIRS`; without them a link can open in another program).
pub const DESKTOP_VARS: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "XAUTHORITY",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
    "XDG_SESSION_DESKTOP",
    "DESKTOP_SESSION",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_DIRS",
    "BROWSER",
];

static DESKTOP: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Remember the desktop of the latest interactive session (from its handshake). An engine
/// started without a desktop (over SSH, by systemd) opens its windows there. Only the
/// variables in `DESKTOP_VARS` are kept, and only when one names a display.
pub fn set_desktop(vars: &std::collections::BTreeMap<String, String>) {
    let kept: Vec<(String, String)> = vars
        .iter()
        .filter(|(k, v)| DESKTOP_VARS.contains(&k.as_str()) && !v.is_empty())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if kept.iter().any(|(k, _)| k == "DISPLAY" || k == "WAYLAND_DISPLAY") {
        if let Ok(mut d) = DESKTOP.lock() {
            *d = kept;
        }
    }
}

/// The desktop to open a window on: this process's own, else the latest session's.
#[cfg(not(windows))]
fn desktop_env() -> Vec<(String, String)> {
    let own: Vec<(String, String)> = DESKTOP_VARS
        .iter()
        .filter_map(|k| {
            std::env::var(k)
                .ok()
                .filter(|v| !v.is_empty())
                .map(|v| (k.to_string(), v))
        })
        .collect();
    if own.iter().any(|(k, _)| k == "DISPLAY" || k == "WAYLAND_DISPLAY") {
        return own;
    }
    DESKTOP.lock().map(|d| d.clone()).unwrap_or_default()
}

/// Terminal emulators tried, in order, for a new console window on Linux.
#[cfg(not(windows))]
const TERMINALS: &[&str] = &[
    "x-terminal-emulator",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "xterm",
];

/// This platform's file name for a program (`claude` -> `claude.exe` on Windows).
pub fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Find a program on PATH the way a shell would.
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let file = if name.contains('.') {
        name.to_string()
    } else {
        exe_name(name)
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(&file))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Find Claude Code the way a shell would (PATH), then the native install location.
pub fn find_claude() -> Option<std::path::PathBuf> {
    which("claude").or_else(|| {
        let p = crate::env::home_dir()
            .join(".local")
            .join("bin")
            .join(exe_name("claude"));
        is_executable(&p).then_some(p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn self_is_alive() {
        assert!(is_alive(std::process::id()));
        assert!(!is_alive(0));
        assert!(parent_pid(std::process::id()).is_some());
        assert!(process_name(std::process::id()).is_some_and(|n| !n.is_empty()));
    }

    #[test]
    fn own_start_time_is_in_the_past() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let t = start_time_ms(std::process::id()).unwrap();
        assert!(t <= now && t > now - 24 * 3_600_000, "{t} vs {now}");
    }

    #[test]
    fn only_a_desktop_with_a_display_is_kept() {
        let mut v = std::collections::BTreeMap::new();
        v.insert("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string());
        v.insert("PATH".to_string(), "/bin".to_string());
        set_desktop(&v);
        assert!(DESKTOP.lock().unwrap().is_empty(), "no display: nothing kept");
        v.insert("DISPLAY".to_string(), ":0".to_string());
        set_desktop(&v);
        let kept = DESKTOP.lock().unwrap().clone();
        assert!(kept.contains(&("DISPLAY".to_string(), ":0".to_string())));
        assert!(!kept.iter().any(|(k, _)| k == "PATH"), "only desktop variables");
    }
}
