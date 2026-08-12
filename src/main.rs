// Claude Code pane statusline, and the hooks that feed it.
//
// One binary, two jobs:
//   statusline              read the session JSON on stdin, print one ANSI line
//   statusline state MODE   working|waiting|clear — the hook that drives the
//                           attention dot and the working-stint timer
//
// This replaces statusline.js + session-state.js. Those were node, and node cost
// ~326 ms of startup and ~47 MB per invocation — paid once per second per pane
// (settings.json `refreshInterval` is in seconds and stacks on top of
// event-driven renders), plus once per tool call for the state hook, plus a
// `git` child process per render. Measured end to end, a render was ~592 ms and
// roughly six processes deep once Git Bash's double-exec and the conhosts were
// counted. Nothing here needs a JS runtime, so it no longer has one.
//
// What this does NOT prevent: Claude Code sometimes creates a render process
// suspended and abandons it before it ever runs an instruction (0 CPU time, one
// suspended thread). Those strays leak whatever the binary is written in — no
// code inside the process runs, so it cannot defend itself. Since nothing can
// stop it from inside, the next render collects it instead (see reap.rs), which
// needs no scheduled task and no extra process.

mod json;
mod paths;
mod reap;
mod render;
mod state;

use std::io::{Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Never block forever on stdin. The JS this replaces originally used
/// `fs.readFileSync(0)`, which hangs unkillably if the parent never sends EOF —
/// a reproduced hang that no timer can rescue. Read on a thread, render with
/// whatever arrived by the deadline, and exit explicitly.
const STDIN_DEADLINE_MS: u64 = 500;

fn read_stdin() -> String {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (tx, rx) = mpsc::channel::<()>();
    {
        let buf = Arc::clone(&buf);
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut chunk = [0u8; 8192];
            loop {
                match stdin.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let complete = {
                            let Ok(mut g) = buf.lock() else { break };
                            g.extend_from_slice(&chunk[..n]);
                            // Claude Code writes the JSON in one shot — parse
                            // eagerly and skip waiting for EOF.
                            json::parse(&String::from_utf8_lossy(&g)).is_some()
                        };
                        if complete {
                            break;
                        }
                    }
                }
            }
            let _ = tx.send(());
        });
    }
    let _ = rx.recv_timeout(Duration::from_millis(STDIN_DEADLINE_MS));
    let bytes = buf.lock().map(|g| g.clone()).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let input = json::parse(&read_stdin()).unwrap_or(json::Json::Obj(Default::default()));

    match args.first().map(String::as_str) {
        Some("state") => {
            match args.get(1).map(String::as_str) {
                Some(mode @ ("working" | "waiting" | "clear")) => state::run(mode, &input),
                _ => eprintln!("usage: statusline state working|waiting|clear"),
            }
            reap::maybe_sweep(&paths::session_tasks_dir());
            std::process::exit(0);
        }
        // Bare invocation (or an explicit `render`) is the statusline itself.
        None | Some("render") => {
            let line = render::render(&input);
            let mut out = std::io::stdout();
            let _ = out.write_all(line.as_bytes());
            let _ = out.flush();
            // Collect abandoned siblings *after* the line is out, so the sweep
            // never delays what the user sees. Rate-limited to once a minute.
            reap::maybe_sweep(&paths::session_tasks_dir());
            // Exit explicitly: the stdin reader thread may still be parked on a
            // pipe that will never close, and a stray render must never outlive
            // its own output.
            std::process::exit(0);
        }
        Some(other) => {
            eprintln!("unknown command: {other}");
            std::process::exit(2);
        }
    }
}
