//! Window-less launcher for the sign-in startup entry: runs `quayflow.exe start` from the
//! same folder without ever showing a console window.
#![windows_subsystem = "windows"]

fn main() {
    let Ok(me) = std::env::current_exe() else { return };
    let exe = me.with_file_name("quayflow.exe");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = if args.is_empty() {
        vec!["start".to_string()]
    } else {
        args
    };
    let mut c = std::process::Command::new(exe);
    c.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = c.status();
}
