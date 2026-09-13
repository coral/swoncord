//! A hard deadline for artwork, including HTTP client creation and destruction.
//! A timed-out thread cannot be safely killed. A helper process can be killed
//! and reaped, so recovery never leaks an increasing number of stuck threads.

use super::albumart::{AlbumArtRequester, ArtworkLookup};
use crate::{error::Error, track::TrackInfo};
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const HELPER_FLAG: &str = "--artwork-helper";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_REPLY: u64 = 8192;

/// Runs before AppKit or logging initialization. Only the parent writes logs.
pub(crate) fn run_helper() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(HELPER_FLAG) {
        return Ok(false);
    }
    // The helper also expires if the menu app quits while a lookup is stuck.
    // This thread belongs to the disposable process, never to the menu app.
    std::thread::Builder::new()
        .name("artwork-deadline".into())
        .spawn(|| {
            std::thread::sleep(LOOKUP_TIMEOUT);
            std::process::exit(124);
        })?;
    let track = TrackInfo {
        artist: args.next().ok_or("missing artwork artist")?,
        album: args.next().ok_or("missing artwork album")?,
        ..Default::default()
    };
    let reply = match AlbumArtRequester::new().get_album_art(&track) {
        Ok(cover) => json!({"cover": cover}),
        Err(Error::NoData) => json!({"cover": null}),
        Err(error) => {
            // Keep the entire response below the pipe capacity: the parent
            // waits for exit before reading it. Preserve nested HTTP errors.
            let message: String = format!("{error:?}").chars().take(500).collect();
            json!({"error": message})
        }
    };
    serde_json::to_writer(io::stdout().lock(), &reply)?;
    io::stdout().flush()?;
    Ok(true)
}

pub(super) struct ArtworkProcess;

impl ArtworkLookup for ArtworkProcess {
    fn lookup(&mut self, track: &TrackInfo, cancelled: &dyn Fn() -> bool) -> Option<String> {
        let result = (|| -> io::Result<Value> {
            let mut command = Command::new(std::env::current_exe()?);
            command.args([HELPER_FLAG, &track.artist, &track.album]);
            let output = run_bounded(&mut command, LOOKUP_TIMEOUT, cancelled)?;
            serde_json::from_slice(&output).map_err(io::Error::other)
        })();
        match result {
            Ok(reply) => {
                if let Some(error) = reply.get("error") {
                    log::warn!("Artwork lookup failed: {error}");
                }
                reply
                    .get("cover")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                log::info!("Artwork lookup cancelled: superseded by a different album");
                None
            }
            Err(error) => {
                log::warn!("Artwork helper failed (next attempt uses fresh clients): {error}");
                None
            }
        }
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        // Also clean up on timeout and on errors from try_wait/read.
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn run_bounded(
    command: &mut Command,
    timeout: Duration,
    cancelled: &dyn Fn() -> bool,
) -> io::Result<Vec<u8>> {
    if cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let deadline = Instant::now() + timeout;
    let mut child = ChildGuard(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?,
    );
    loop {
        if cancelled() {
            // ChildGuard kills and reaps the helper before the worker can
            // start the newest album. No old HTTP request remains in flight.
            return Err(io::ErrorKind::Interrupted.into());
        }
        if let Some(status) = child.0.try_wait()? {
            if status.code() == Some(124) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "artwork helper deadline exceeded",
                ));
            }
            if !status.success() {
                return Err(io::Error::other(format!("exited with {status}")));
            }
            let mut output = Vec::new();
            child
                .0
                .stdout
                .take()
                .expect("piped stdout")
                .take(MAX_REPLY + 1)
                .read_to_end(&mut output)?;
            if output.len() as u64 > MAX_REPLY {
                return Err(io::Error::other("artwork helper reply too large"));
            }
            return Ok(output);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("artwork lookup exceeded {timeout:?}; terminating helper"),
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn cancellation_terminates_a_running_helper_without_waiting_for_its_deadline() {
        let polls = Cell::new(0);
        let started = Instant::now();
        let error = run_bounded(
            Command::new("/bin/sleep").arg("60"),
            Duration::from_secs(45),
            &|| {
                polls.set(polls.get() + 1);
                // First call precedes spawn; second is after spawn. Cancel
                // on the third call while the child is already running.
                polls.get() >= 3
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(started.elapsed() < Duration::from_secs(2));
        let next = run_bounded(
            Command::new("/usr/bin/printf").arg("new album"),
            Duration::from_secs(2),
            &|| false,
        )
        .unwrap();
        assert_eq!(next, b"new album");
    }

    #[test]
    fn stuck_helper_is_killed_and_the_next_lookup_can_complete() {
        let start = Instant::now();
        let error = run_bounded(
            Command::new("/bin/sleep").arg("60"),
            Duration::from_millis(50),
            &|| false,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
        let output = run_bounded(
            Command::new("/usr/bin/printf").arg("{\"cover\":\"art\"}"),
            Duration::from_secs(2),
            &|| false,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["cover"],
            "art"
        );
    }

    #[test]
    fn helper_exit_failure_is_reported() {
        assert!(
            run_bounded(
                &mut Command::new("/usr/bin/false"),
                Duration::from_secs(2),
                &|| false
            )
            .is_err()
        );
    }
}
