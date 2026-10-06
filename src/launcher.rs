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

/// The Wine prefix of an MT5 command (`WINEPREFIX=` in it, else the environment, else `~/.mt5`).
fn wine_prefix(command: &[String]) -> std::path::PathBuf {
    command
        .iter()
        .find_map(|a| a.strip_prefix("WINEPREFIX="))
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("WINEPREFIX").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".mt5"))
}

/// Startup config for MT5's `/config:`: log into `login`@`server` with the password saved in the
/// terminal, and keep automated trading on across the account change (only for this start).
fn login_ini(login: i64, server: &str) -> Vec<u8> {
    let text = format!("[Common]\r\nLogin={login}\r\nServer={server}\r\n[Experts]\r\nEnabled=1\r\nAccount=0\r\n");
    // MT5's own .ini files are UTF-16LE with a BOM
    let mut out = vec![0xFF, 0xFE];
    out.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
    out
}

/// MT5's main window on Hyprland: (address, workspace). Its title starts with the login
/// ("1234567 - Server: ..."); dialogs of the terminal don't.
fn mt5_main_window() -> Option<(String, i64)> {
    let out = Command::new("hyprctl").args(["clients", "-j"]).output().ok()?;
    let clients = serde_json::from_slice::<serde_json::Value>(&out.stdout).ok()?;
    let main = clients.as_array()?.iter().find(|c| {
        c["class"] == "terminal64.exe"
            && c["title"].as_str().and_then(|t| t.split(' ').next()).is_some_and(|w| !w.is_empty() && w.chars().all(|ch| ch.is_ascii_digit()))
    })?;
    Some((main["address"].as_str()?.to_string(), main["workspace"]["id"].as_i64()?))
}

fn hypr(dispatch: String) -> bool {
    Command::new("hyprctl").args(["dispatch", &dispatch]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Restart MT5 logged into another account, off the UI thread. Progress and the outcome go to
/// `report` (an error starts with "erro").
pub fn switch_account(command: Vec<String>, login: i64, server: String, report: crossbeam_channel::Sender<String>) {
    std::thread::spawn(move || {
        let say = |m: String| {
            let _ = report.send(m);
        };
        // the terminal comes back on the workspace it was on, not on top of the app
        let home = mt5_main_window().map(|(_, ws)| ws);
        if mt5_running() {
            say("fechando o MetaTrader 5…".into());
            // as clicking its ×: the terminal saves and exits
            let closed = mt5_main_window().is_some_and(|(addr, _)| hypr(format!("hl.dsp.window.close({{ window = \"address:{addr}\" }})")));
            if !closed {
                say("erro: não encontrei a janela do MetaTrader 5 para fechar".into());
                return;
            }
            let started = std::time::Instant::now();
            while mt5_running() {
                if started.elapsed() > std::time::Duration::from_secs(40) {
                    say("erro: o MetaTrader 5 não fechou (há algum diálogo aberto nele?)".into());
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
        let ini = wine_prefix(&command).join("drive_c/mt5-terminal-login.ini");
        if let Err(e) = std::fs::write(&ini, login_ini(login, &server)) {
            say(format!("erro: não gravou {}: {e}", ini.display()));
            return;
        }
        let mut cmd = command;
        cmd.push(r"/config:C:\mt5-terminal-login.ini".into());
        say(format!("abrindo o MetaTrader 5 na conta {login}…"));
        if let Err(e) = start_mt5(&cmd) {
            say(format!("erro: não abriu o MetaTrader 5: {e}"));
            return;
        }
        let Some(home) = home else { return };
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(60) {
            if let Some((addr, ws)) = mt5_main_window() {
                if ws != home {
                    hypr(format!("hl.dsp.window.move({{ workspace = \"{home}\", window = \"address:{addr}\", follow = false }})"));
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    });
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

    #[test]
    fn login_config_for_mt5() {
        let ini = login_ini(42, "Broker-Demo");
        assert_eq!(&ini[..2], &[0xFF, 0xFE]);
        let text = String::from_utf16(&ini[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).unwrap();
        assert!(text.contains("Login=42\r\nServer=Broker-Demo") && text.contains("Account=0"));
        assert!(!text.to_lowercase().contains("password"), "never a password");
        let cmd: Vec<String> = ["env", "WINEPREFIX=/x/.mt5", "wine", "t.exe"].iter().map(|s| s.to_string()).collect();
        assert_eq!(wine_prefix(&cmd), std::path::PathBuf::from("/x/.mt5"));
    }
}
