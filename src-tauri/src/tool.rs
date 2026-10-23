//! Runs macOS command-line tools with fixed arguments and bounded cost.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;

/// Runs a fixed system tool (absolute path, constant arguments, no shell) with a
/// time limit on both its output and its exit, so a hung tool cannot hold a
/// lock or a worker thread. A tool that overruns is killed.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    run_timed(program, args, timeout).map(|(output, _)| output)
}

/// `run`, also returning when the tool closed its output, which is when it had
/// finished its work.
pub fn run_timed(program: &str, args: &[&str], timeout: Duration) -> Result<(String, Instant), String> {
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {program}: {e}"))?;

    // Read on another thread so a full pipe cannot stall the child. The channel
    // bounds the wait even if something keeps the pipe open.
    let stdout = child.stdout.take().ok_or("stdout was not captured")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let read = stdout.take(MAX_OUTPUT_BYTES).read_to_end(&mut buf).map(|_| buf);
        let _ = tx.send((read, Instant::now()));
    });
    let overran = || format!("{program} did not finish within {} s", timeout.as_secs());

    let (out, ended) = match rx.recv_timeout(timeout) {
        Ok((read, ended)) => (read.map_err(|e| format!("{program}: {e}")), ended),
        Err(_) => (Err(overran()), Instant::now()),
    };
    // Tools exit as soon as they close stdout, so this normally returns at once.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) if out.is_ok() => break status,
            Ok(None) if out.is_ok() && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) if out.is_ok() => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(overran());
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(out.err().unwrap_or_else(overran));
            }
        }
    };
    let out = out?;
    if !status.success() {
        return Err(format!("{program} exited with {status}"));
    }
    Ok((String::from_utf8_lossy(&out).into_owned(), ended))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn run_returns_output_and_reports_failures() {
        assert_eq!(run("/bin/echo", &["ok"], TIMEOUT).unwrap(), "ok\n");
        assert!(run("/usr/bin/false", &[], TIMEOUT).unwrap_err().contains("exited"));
        assert!(run("/nonexistent/tool", &[], TIMEOUT).unwrap_err().contains("could not run"));
    }

    #[test]
    fn run_is_bounded_when_a_descendant_keeps_the_pipe_open() {
        // The shell exits at once but leaves a background sleep holding stdout.
        let started = Instant::now();
        let err = run("/bin/sh", &["-c", "/bin/sleep 30 & echo started"], Duration::from_millis(400)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn run_kills_a_hung_command() {
        let started = Instant::now();
        let err = run("/bin/sleep", &["30"], Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
