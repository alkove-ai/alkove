use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};

use crate::engine::{self, Config, Event};
use crate::{focus, models, sound, speak};

// how long the microphone stays open once the last session left
const LINGER: Duration = Duration::from_secs(10);

pub fn dir() -> PathBuf {
    models::cache_dir().join("voice")
}

// when the running binary was built, as its file's time: a newer session replaces an older daemon
pub fn build() -> Json {
    let exe = std::env::current_exe().ok();
    let mtime = exe.as_ref().and_then(|e| std::fs::metadata(e).ok()).and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    json!({ "version": env!("CARGO_PKG_VERSION"), "exe": exe, "mtime": mtime })
}

// a higher version, or the same binary rebuilt; two different binaries of one version never oust each other
fn is_newer(theirs: &Json, mine: &Json) -> bool {
    let v = |b: &Json| b["version"].as_str().unwrap_or_default().split('.').map(|n| n.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    match v(theirs).cmp(&v(mine)) {
        std::cmp::Ordering::Equal => theirs["exe"] == mine["exe"] && theirs["mtime"].as_u64() > mine["mtime"].as_u64(),
        o => o.is_gt(),
    }
}

pub fn socket() -> PathBuf {
    dir().join("daemon.sock")
}

pub fn private(path: &PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(path).and_then(|_| std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))).map_err(|e| format!("{}: {e}", path.display()))
}

struct Session {
    number: Option<u32>,
    tty: Option<String>,
    active: Instant,
    out: UnixStream,
}

#[derive(Default)]
struct State {
    sessions: HashMap<u64, Session>,
    next: u64,
    config: Config,
    ready: Option<Json>,
    error: Option<String>,
    empty_since: Option<Instant>,
    build: Json,
}

fn send(out: &mut UnixStream, msg: &Json) {
    let _ = writeln!(out, "{msg}");
}

fn broadcast(s: &mut State, msg: &Json) {
    for session in s.sessions.values_mut() {
        send(&mut session.out, msg);
    }
}

// the session a phrase without a number goes to: the terminal tab in front when it is one of ours, else the last one used
fn default_target(state: &Arc<Mutex<State>>) -> Option<u64> {
    let front = focus::front_tty();
    let s = state.lock().expect("state");
    if let Some(t) = front {
        if let Some((&id, _)) = s.sessions.iter().find(|(_, x)| x.tty.as_deref() == Some(t.as_str())) {
            return Some(id);
        }
    }
    s.sessions.iter().max_by_key(|(_, x)| x.active).map(|(&id, _)| id)
}

fn route(state: &Arc<Mutex<State>>, e: Event) {
    match e {
        Event::Ready { trigger } => {
            let mut s = state.lock().expect("state");
            s.error = None;
            s.ready = Some(trigger.clone());
            broadcast(&mut s, &json!({ "type": "ready", "trigger": trigger }));
        }
        Event::Error(message) => {
            let mut s = state.lock().expect("state");
            s.error = Some(message.clone());
            broadcast(&mut s, &json!({ "type": "error", "message": message }));
        }
        Event::Wake => {
            if let Some(id) = default_target(state) {
                if let Some(x) = state.lock().expect("state").sessions.get_mut(&id) {
                    send(&mut x.out, &json!({ "type": "wake" }));
                }
            }
        }
        Event::Heard { number, text, transcript } => {
            let target = match number {
                Some(n) => state.lock().expect("state").sessions.iter().find(|(_, x)| x.number == Some(n)).map(|(&id, _)| id),
                None => default_target(state),
            };
            let mut s = state.lock().expect("state");
            eprintln!("          route: number {number:?} → {}", target.and_then(|id| s.sessions.get(&id)).map_or("no session".to_string(), |x| format!("session {:?}", x.number)));
            match target.and_then(|id| s.sessions.get_mut(&id)) {
                Some(x) => send(&mut x.out, &json!({ "type": "heard", "text": text, "transcript": transcript, "number": number })),
                None => sound::play("error"),
            }
        }
    }
}

fn client(conn: UnixStream, state: Arc<Mutex<State>>, configs: Sender<Config>, voice: Sender<speak::Job>) {
    let Ok(out) = conn.try_clone() else { return };
    let mut lines = BufReader::new(conn).lines();
    let Some(Ok(hello)) = lines.next() else { return };
    let hello: Json = serde_json::from_str(&hello).unwrap_or_default();
    if is_newer(&hello["build"], &state.lock().expect("state").build) {
        eprintln!("          a session runs a newer alkove-voice: this daemon steps aside");
        let _ = std::fs::remove_file(socket());
        std::process::exit(0);
    }
    let id = {
        let mut s = state.lock().expect("state");
        let max = s.config.max_sessions;
        let number = (1..=max).find(|n| s.sessions.values().all(|x| x.number != Some(*n)));
        let id = s.next;
        s.next += 1;
        let mut out = out;
        send(&mut out, &json!({ "type": "session", "number": number }));
        send(&mut out, &json!({ "type": "listening", "on": s.config.listening }));
        send(&mut out, &crate::devices::list());
        if let Some(t) = &s.ready {
            send(&mut out, &json!({ "type": "ready", "trigger": t }));
        }
        if let Some(e) = &s.error {
            send(&mut out, &json!({ "type": "error", "message": e }));
        }
        s.sessions.insert(id, Session { number, tty: hello["tty"].as_str().map(focus::short), active: Instant::now(), out });
        s.empty_since = None;
        id
    };
    for line in lines.map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Json>(&line) else { continue };
        match msg["type"].as_str() {
            Some("config") => {
                let c = Config::from_json(&msg);
                eprintln!("          config: {c:?}");
                let mut s = state.lock().expect("state");
                crate::devices::set_speaker(c.speaker.clone());
                if s.config.listening != c.listening {
                    broadcast(&mut s, &json!({ "type": "listening", "on": c.listening }));
                }
                s.config = c.clone();
                let _ = configs.send(c);
                if msg["readAloud"].as_bool() == Some(true) {
                    let _ = voice.send(speak::Job::Prepare);
                }
            }
            Some("activity") => {
                if let Some(x) = state.lock().expect("state").sessions.get_mut(&id) {
                    x.active = Instant::now();
                }
            }
            Some("log") => eprintln!("          mod {id}: {}", msg["text"].as_str().unwrap_or_default()),
            Some("devices") => {
                if let Some(x) = state.lock().expect("state").sessions.get_mut(&id) {
                    send(&mut x.out, &crate::devices::list());
                }
            }
            Some("sound") => sound::play(msg["name"].as_str().unwrap_or_default()),
            Some("say") => {
                let text = msg["text"].as_str().unwrap_or_default().trim().to_string();
                if !text.is_empty() {
                    let lang = msg["lang"].as_str().unwrap_or("en").to_string();
                    let int = |k: &str, d: i64, max: i64| msg[k].as_i64().unwrap_or(d).clamp(0, max) as i32;
                    let _ = voice.send(speak::Job::Say { text, lang, voice: int("voice", 0, 9), steps: int("steps", 4, 32).max(1) });
                }
            }
            _ => {}
        }
    }
    let mut s = state.lock().expect("state");
    s.sessions.remove(&id);
    if s.sessions.is_empty() {
        s.empty_since = Some(Instant::now());
    }
}

fn bind() -> Result<UnixListener, String> {
    private(&dir())?;
    let path = socket();
    match UnixListener::bind(&path) {
        Ok(l) => Ok(l),
        Err(_) if UnixStream::connect(&path).is_ok() => Err("another daemon runs".into()),
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))
        }
    }
}

// one per user: the microphone and the models, shared by every session; ends a while after the last one left
pub fn run(replay: Option<String>) -> Result<(), String> {
    let listener = bind()?;
    let state = Arc::new(Mutex::new(State { empty_since: Some(Instant::now()), build: build(), ..State::default() }));
    let speaking = Arc::new(AtomicBool::new(false));
    let (configs, config_rx) = channel();
    let (events, event_rx) = channel();
    let sp = speaking.clone();
    let (voice, voice_rx) = channel();
    let sv = speaking.clone();
    std::thread::spawn(move || speak::run(voice_rx, sv));
    std::thread::spawn(move || engine::run(config_rx, events, sp, replay));
    let st = state.clone();
    std::thread::spawn(move || {
        for e in event_rx {
            route(&st, e);
        }
    });
    let st = state.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        if st.lock().expect("state").empty_since.is_some_and(|t| t.elapsed() > LINGER) {
            let _ = std::fs::remove_file(socket());
            std::process::exit(0);
        }
    });
    for conn in listener.incoming().map_while(Result::ok) {
        let (st, c, v) = (state.clone(), configs.clone(), voice.clone());
        std::thread::spawn(move || client(conn, st, c, v));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_daemon_steps_aside_for_a_higher_version_or_its_own_binary_rebuilt() {
        let b = |version: &str, exe: &str, mtime: u64| json!({ "version": version, "exe": exe, "mtime": mtime });
        assert!(is_newer(&b("0.3.10", "/a", 1), &b("0.3.9", "/b", 9)));
        assert!(!is_newer(&b("0.3.9", "/b", 9), &b("0.3.10", "/a", 1)));
        assert!(is_newer(&b("0.3.2", "/a", 2), &b("0.3.2", "/a", 1)));
        assert!(!is_newer(&b("0.3.2", "/a", 2), &b("0.3.2", "/b", 1)));
    }
}
