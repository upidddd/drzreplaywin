//! The running app and everything that talks to it (the window, `DrzReplayWIN.exe save`…):
//! one JSON line each way over a loopback socket. The port and a secret token are in a file only this
//! user can read.
use crate::config as C;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

fn token() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut h: u64 = 0xcbf29ce484222325 ^ (t as u64) ^ ((std::process::id() as u64) << 32);
    for b in format!("{t}{:?}", std::thread::current().id()).bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}{:016x}", h.rotate_left(29) ^ (t as u64))
}

/// Serve requests until the process ends. `handle(cmd, request)` returns the reply fields.
pub fn serve(handle: Arc<dyn Fn(&str, &Value) -> Value + Send + Sync>) -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let tok = token();
    let _ = std::fs::create_dir_all(C::data_dir());
    std::fs::write(C::ipc_port_file(), format!("{port} {tok}"))?;
    std::thread::Builder::new().name("ipc".into()).spawn(move || {
        for conn in listener.incoming().flatten() {
            let (h, tok) = (handle.clone(), tok.clone());
            std::thread::spawn(move || {
                let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
                let mut line = String::new();
                if BufReader::new(&conn).read_line(&mut line).is_err() {
                    return;
                }
                let reply = match serde_json::from_str::<Value>(&line) {
                    Ok(req) if req.get("token").and_then(|t| t.as_str()) == Some(tok.as_str()) => {
                        let cmd = req.get("cmd").and_then(|c| c.as_str()).unwrap_or("").to_string();
                        let mut r = h(&cmd, &req);
                        if let Value::Object(m) = &mut r {
                            m.entry("ok").or_insert(json!(true));
                        }
                        r
                    }
                    _ => json!({"ok": false, "error": "bad request"}),
                };
                let mut c = &conn;
                let _ = writeln!(c, "{reply}");
            });
        }
    })?;
    Ok(())
}

pub fn remove_port_file() {
    let _ = std::fs::remove_file(C::ipc_port_file());
}

/// Ask the running app; None when there isn't one.
pub fn request(cmd: &str, extra: Value) -> Option<Value> {
    let text = std::fs::read_to_string(C::ipc_port_file()).ok()?;
    let (port, tok) = text.trim().split_once(' ')?;
    let addr = format!("127.0.0.1:{port}").parse().ok()?;
    let conn = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    let _ = conn.set_read_timeout(Some(Duration::from_secs(30)));
    let mut req = json!({"token": tok, "cmd": cmd});
    if let (Value::Object(r), Value::Object(e)) = (&mut req, extra) {
        r.extend(e);
    }
    let mut c = &conn;
    writeln!(c, "{req}").ok()?;
    let mut line = String::new();
    BufReader::new(&conn).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}
