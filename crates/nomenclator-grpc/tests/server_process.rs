//! Runs the `nomenclator-server` binary as a child process and checks how it stops.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Starts the server with `arguments`, a piped standard input and a piped standard
/// output, and returns it with the first line it prints.
fn start(arguments: &[&str]) -> (Child, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nomenclator-server"))
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the server starts");
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().expect("standard output is piped"))
        .read_line(&mut line)
        .expect("the server prints its address");
    (child, line)
}

/// Waits up to `timeout` for `child` to exit. Returns `None` if it is still running.
fn wait(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("the child can be polled") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn server_stops_on_stdin_close_with_option() {
    let (mut child, line) = start(&["--tcp", "127.0.0.1:0", "--stop-on-stdin-close"]);
    assert!(line.starts_with("listening on tcp:127.0.0.1:"), "{line}");
    drop(child.stdin.take());
    let status = wait(&mut child, Duration::from_secs(10))
        .expect("the server stops after its standard input closes");
    assert!(status.success(), "{status}");
}

#[test]
fn server_keeps_running_on_stdin_close_without_option() {
    let (mut child, _) = start(&["--tcp", "127.0.0.1:0"]);
    drop(child.stdin.take());
    assert!(
        wait(&mut child, Duration::from_millis(500)).is_none(),
        "the server stopped without `--stop-on-stdin-close`"
    );
    child.kill().expect("the server can be killed");
    child.wait().expect("the server exits");
}

#[cfg(unix)]
#[test]
fn server_removes_unix_socket_on_stdin_close() {
    let socket = std::env::temp_dir().join(format!("nomenclator-test-{}.sock", std::process::id()));
    let path = socket.to_str().expect("the socket path is UTF-8");
    let (mut child, line) = start(&["--unix", path, "--stop-on-stdin-close"]);
    assert_eq!(line.trim_end(), format!("listening on unix:{path}"));
    assert!(socket.exists(), "the socket file is missing while serving");
    drop(child.stdin.take());
    let status = wait(&mut child, Duration::from_secs(10))
        .expect("the server stops after its standard input closes");
    assert!(status.success(), "{status}");
    assert!(!socket.exists(), "the socket file remains after the stop");
}
