//! Process helpers (Windows): liveness, parent process, window-less and new-console launches.

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

#[cfg(not(windows))]
pub fn start_time_ms(_pid: u32) -> Option<i64> {
    None
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
pub fn parent_pid(_pid: u32) -> Option<u32> {
    None
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
pub fn process_name(_pid: u32) -> Option<String> {
    None
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
            if name == "pwsh.exe" || name == "powershell.exe" {
                return Some((parent, name));
            }
        }
        cur = parent;
    }
    None
}

/// Build a command that never shows a console window.
pub fn hidden(program: impl AsRef<OsStr>) -> Command {
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
pub fn spawn_new_console(
    exe: &Path,
    args: &[String],
    env: &[(&str, Option<&OsStr>)],
    cwd: Option<&Path>,
) -> std::io::Result<Child> {
    let mut c = Command::new(exe);
    c.args(args);
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

/// Find `claude.exe` the way a shell would (PATH), then the native install location.
pub fn find_claude() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let p = dir.join("claude.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let p = crate::env::home_dir().join(".local").join("bin").join("claude.exe");
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn self_is_alive() {
        assert!(is_alive(std::process::id()));
        assert!(!is_alive(0));
        #[cfg(windows)]
        assert!(parent_pid(std::process::id()).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn own_start_time_is_in_the_past() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let t = start_time_ms(std::process::id()).unwrap();
        assert!(t <= now && t > now - 24 * 3_600_000, "{t} vs {now}");
    }
}
