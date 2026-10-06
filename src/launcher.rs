//! The processes around the app: only one instance owns the EA's port (a second launch brings the
//! first one forward and exits), and MetaTrader 5 is started when it isn't running.

use std::process::{Command, Stdio};

/// Command lines (NUL-separated arguments) of every running process (Linux `/proc`), skipping this one.
fn processes() -> impl Iterator<Item = (u32, String)> {
    let me = std::process::id();
    std::fs::read_dir("/proc").into_iter().flatten().flatten().filter_map(move |e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        if pid == me {
            return None;
        }
        let raw = std::fs::read(e.path().join("cmdline")).ok()?;
        Some((pid, String::from_utf8_lossy(&raw).into_owned()))
    })
}

/// Another mt5-terminal is running (by process name, which survives a rebuild of the executable).
pub fn other_instance() -> bool {
    let me = std::fs::read_to_string("/proc/self/comm").unwrap_or_default();
    processes().any(|(pid, _)| std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c == me))
}

/// Bring the running instance's window forward (Hyprland); harmless elsewhere.
pub fn focus_other() {
    let _ = Command::new("hyprctl")
        .args(["dispatch", "hl.dsp.focus({ window = \"class:mt5-terminal\" })"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// The terminal itself: Wine sets its argv[0] to the Windows path of `terminal64.exe`. Only the
/// program is checked, never the arguments (a shell or grep mentioning the name is not the terminal).
pub fn mt5_running() -> bool {
    processes().any(|(_, cmd)| is_terminal(&cmd))
}

fn is_terminal(cmdline: &str) -> bool {
    let program = cmdline.split('\0').next().unwrap_or("").to_ascii_lowercase();
    program.ends_with("terminal64.exe")
}

/// How MetaTrader 5 is started: the same as its desktop entry (Wine of the system, prefix `~/.mt5`).
pub fn default_mt5_command() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        "env".into(),
        format!("WINEPREFIX={home}/.mt5"),
        "wine".into(),
        r"C:\Program Files\MetaTrader 5\terminal64.exe".into(),
    ]
}

/// Start MetaTrader 5 detached from the app (its own systemd scope under uwsm, so closing the app
/// never closes it).
pub fn start_mt5(command: &[String]) -> std::io::Result<()> {
    let Some((program, args)) = command.split_first() else {
        return Err(std::io::Error::other("comando vazio"));
    };
    let uwsm = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("uwsm-app").is_file()));
    let mut cmd = if uwsm {
        let mut c = Command::new("uwsm-app");
        c.arg("--").arg(program).args(args);
        c
    } else {
        let mut c = Command::new(program);
        c.args(args);
        c
    };
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    // reap the launcher when it returns (uwsm-app does quickly; Wine forks the terminal itself)
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_by_program_only() {
        assert!(is_terminal("C:\\Program Files\\MetaTrader 5\\terminal64.exe\0"));
        assert!(!is_terminal("/usr/bin/bash\0-c\0pgrep terminal64.exe\0"));
        assert!(!is_terminal("grep\0terminal64.exe\0"));
    }
}
