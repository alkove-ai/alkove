use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// a command's output, or None when it fails or takes more than a second (an Automation prompt left open)
fn run(argv: &[&str]) -> Option<String> {
    let mut child = Command::new(argv[0]).args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let t = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(s) if s.success() => break,
            Some(_) => return None,
            None if t.elapsed() > Duration::from_secs(1) => {
                let _ = child.kill();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string())
}

pub fn short(tty: &str) -> String {
    tty.trim().trim_start_matches("/dev/").to_string()
}

// the terminal a process runs in, "ttys002"; None for an app's process (the desktop app) or a gone one
pub fn tty_of(pid: u32) -> Option<String> {
    run(&["ps", "-o", "tty=", "-p", &pid.to_string()]).map(|t| short(&t)).filter(|t| !t.is_empty() && !t.starts_with('?'))
}

#[cfg(target_os = "macos")]
fn front_app() -> Option<String> {
    let asn = run(&["lsappinfo", "front"])?;
    let info = run(&["lsappinfo", "info", "-only", "name", &asn])?;
    info.split('=').nth(1).map(|n| n.trim().trim_matches('"').to_string())
}

#[cfg(target_os = "macos")]
fn front_tab() -> Option<String> {
    let script = match front_app()?.as_str() {
        "iTerm2" => "tell application \"iTerm2\" to tty of current session of current window",
        "Terminal" => "tell application \"Terminal\" to tty of selected tab of front window",
        "WezTerm" => return wezterm(),
        _ => return None,
    };
    run(&["osascript", "-e", script]).map(|t| short(&t))
}

#[cfg(not(target_os = "macos"))]
fn front_tab() -> Option<String> {
    wezterm()
}

fn wezterm() -> Option<String> {
    let clients: serde_json::Value = serde_json::from_str(&run(&["wezterm", "cli", "list-clients", "--format", "json"])?).ok()?;
    let idle = |c: &&serde_json::Value| (c["idle_time"]["secs"].as_u64().unwrap_or(u64::MAX), c["idle_time"]["nanos"].as_u64().unwrap_or(0));
    let pane = clients.as_array()?.iter().min_by_key(idle)?["focused_pane_id"].as_u64()?;
    let panes: serde_json::Value = serde_json::from_str(&run(&["wezterm", "cli", "list", "--format", "json"])?).ok()?;
    panes.as_array()?.iter().find(|p| p["pane_id"].as_u64() == Some(pane))?["tty_name"].as_str().map(short)
}

// the tty of the pane a tmux client shows, when `tty` is a tmux client's; else `tty`
fn through_tmux(tty: String) -> String {
    let Some(list) = run(&["tmux", "list-clients", "-F", "#{client_tty} #{pane_tty}"]) else { return tty };
    list.lines().find_map(|l| l.split_once(' ').filter(|(c, _)| short(c) == tty).map(|(_, p)| short(p))).unwrap_or(tty)
}

// the tty of the terminal tab in front, when the terminal can say
pub fn front_tty() -> Option<String> {
    front_tab().map(through_tmux)
}
