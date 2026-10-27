use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value as Json};

use crate::daemon::{dir, private, socket};
use crate::focus;

fn connect(is_debug: bool) -> Result<UnixStream, String> {
    if let Ok(c) = UnixStream::connect(socket()) {
        return Ok(c);
    }
    private(&dir())?;
    let log = if is_debug { Stdio::from(std::fs::File::create(dir().join("daemon.log")).map_err(|e| e.to_string())?) } else { Stdio::null() };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // its own process group: the daemon outlives the session that started it
    Command::new(exe).arg("daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(log).process_group(0).spawn().map_err(|e| format!("daemon: {e}"))?;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(c) = UnixStream::connect(socket()) {
            return Ok(c);
        }
    }
    Err("daemon: did not start".into())
}

// one HTTP/1.1 request, `POST /<type>` with a JSON object, as the message `{ type, ... }`
fn request(conn: &mut UnixStream) -> Option<Json> {
    let mut reader = BufReader::new(conn.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let kind = line.split_whitespace().nth(1)?.trim_start_matches('/').to_string();
    let mut len = 0;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).ok()?;
        if h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body).ok()?;
    let mut msg: Json = serde_json::from_slice(&body).unwrap_or(json!({}));
    msg.as_object_mut()?.insert("type".into(), kind.into());
    let _ = conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    Some(msg)
}

// the bridge between one session's mod and the daemon: the daemon's messages on stdout, the mod's through a private HTTP socket; a daemon that goes is replaced, the last config sent again
pub fn run(is_debug: bool) -> Result<(), String> {
    let claude = std::os::unix::process::parent_id();
    let hello = json!({ "type": "hello", "pid": claude, "tty": focus::tty_of(claude), "build": crate::daemon::build() }).to_string();
    let daemon: Arc<Mutex<Option<UnixStream>>> = Arc::new(Mutex::new(None));
    let config: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let folder = dir().join("s");
    private(&folder)?;
    let path = folder.join(format!("{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let http = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("{}", json!({ "type": "socket", "path": path }));
    let (d, cfg) = (daemon.clone(), config.clone());
    std::thread::spawn(move || {
        for mut c in http.incoming().map_while(Result::ok) {
            let Some(msg) = request(&mut c) else { continue };
            let line = msg.to_string();
            if msg["type"] == "config" {
                *cfg.lock().expect("config") = Some(line.clone());
            }
            if let Some(out) = d.lock().expect("daemon").as_mut() {
                let _ = writeln!(out, "{line}");
            }
        }
    });

    let mut stdout = std::io::stdout().lock();
    // reconnects in a row that got nothing from the daemon
    let mut misses = 0;
    while misses < 20 {
        misses += 1;
        let conn = connect(is_debug)?;
        {
            // held while the config is replayed, so one the mod sends meanwhile reaches the new daemon after it
            let mut d = daemon.lock().expect("daemon");
            let mut out = conn.try_clone().map_err(|e| e.to_string())?;
            writeln!(out, "{hello}").map_err(|e| e.to_string())?;
            if let Some(c) = config.lock().expect("config").as_ref() {
                writeln!(out, "{c}").map_err(|e| e.to_string())?;
            }
            *d = Some(out);
        }
        for line in BufReader::new(conn).lines().map_while(Result::ok) {
            misses = 0;
            if writeln!(stdout, "{line}").and_then(|_| stdout.flush()).is_err() {
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }
        }
        *daemon.lock().expect("daemon") = None;
        std::thread::sleep(Duration::from_millis(300));
    }
    let _ = std::fs::remove_file(&path);
    Err("daemon: gone".into())
}
