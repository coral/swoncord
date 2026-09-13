//! Discord IPC with acknowledged commands and a deadline for every exchange.

use discord_rich_presence::activity::Activity;
use serde_json::{Value, json};
use socket2::{Domain, SockAddr, Socket, Type};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FRAME: usize = 64 * 1024;

pub(super) struct IpcClient {
    stream: UnixStream,
    nonce: u64,
}

impl IpcClient {
    fn from_stream(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self { stream, nonce: 0 })
    }

    pub fn connect(app_id: &str) -> io::Result<Self> {
        let deadline = Instant::now() + IO_TIMEOUT;
        let bases = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .chain([PathBuf::from("/tmp")]);
        let mut last_error =
            io::Error::new(io::ErrorKind::NotFound, "Discord IPC socket not found");
        for base in bases {
            for index in 0..10 {
                let path = base.join(format!("discord-ipc-{index}"));
                if !path.exists() {
                    continue;
                }
                let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
                if let Err(error) =
                    socket.connect_timeout(&SockAddr::unix(path)?, remaining(deadline)?)
                {
                    last_error = error;
                    continue;
                }
                let mut client = Self::from_stream(socket.into())?;
                client.send(0, &json!({"v": 1, "client_id": app_id}), deadline)?;
                client.receive_reply(None, deadline)?;
                return Ok(client);
            }
        }
        Err(last_error)
    }

    pub fn set_activity(&mut self, activity: Option<Activity<'_>>) -> io::Result<()> {
        self.set_activity_with_timeout(activity, IO_TIMEOUT)
    }

    fn set_activity_with_timeout(
        &mut self,
        activity: Option<Activity<'_>>,
        timeout: Duration,
    ) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        self.nonce = self.nonce.wrapping_add(1);
        let nonce = self.nonce.to_string();
        self.send(
            1,
            &json!({
                "cmd": "SET_ACTIVITY", "nonce": nonce,
                "args": {"pid": std::process::id(), "activity": activity}
            }),
            deadline,
        )?;
        self.receive_reply(Some(&nonce), deadline)
    }

    fn receive_reply(&mut self, nonce: Option<&str>, deadline: Instant) -> io::Result<()> {
        loop {
            let (opcode, reply) = self.receive(deadline)?;
            match opcode {
                3 => self.send(4, &reply, deadline)?, // PING -> PONG
                2 => return Err(io::Error::other(format!("Discord closed IPC: {reply}"))),
                1 if nonce.is_none() && reply["evt"] == "READY" => return Ok(()),
                1 if nonce.is_some() && reply["nonce"].as_str() == nonce => {
                    if reply["evt"] == "ERROR" {
                        return Err(io::Error::other(format!(
                            "Discord rejected activity: {}",
                            reply["data"]
                        )));
                    }
                    if reply["cmd"] != "SET_ACTIVITY" {
                        return Err(io::Error::other("unexpected Discord command response"));
                    }
                    log::debug!("Discord acknowledged activity: {}", reply["data"]["assets"]);
                    return Ok(());
                }
                1 if nonce.is_none() && reply["evt"] == "ERROR" => {
                    return Err(io::Error::other(format!(
                        "Discord rejected handshake: {}",
                        reply["data"]
                    )));
                }
                1 | 4 => {} // Unrelated event or PONG; keep waiting for our nonce.
                _ => {
                    return Err(io::Error::other(format!(
                        "unexpected Discord opcode {opcode}"
                    )));
                }
            }
        }
    }

    fn send(&mut self, opcode: u32, data: &Value, deadline: Instant) -> io::Result<()> {
        let body = serde_json::to_vec(data)?;
        if body.len() > MAX_FRAME {
            return Err(io::Error::other("Discord frame too large"));
        }
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend(opcode.to_le_bytes());
        frame.extend((body.len() as u32).to_le_bytes());
        frame.extend(body);
        let mut bytes = frame.as_slice();
        while !bytes.is_empty() {
            remaining(deadline)?;
            match self.stream.write(bytes) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => bytes = &bytes[n..],
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.wait_ready(libc::POLLOUT, deadline)?
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn receive(&mut self, deadline: Instant) -> io::Result<(u32, Value)> {
        let mut header = [0; 8];
        self.read_exact(&mut header, deadline)?;
        let opcode = u32::from_le_bytes(header[..4].try_into().unwrap());
        let length = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        if length > MAX_FRAME {
            return Err(io::Error::other("Discord frame too large"));
        }
        let mut body = vec![0; length];
        self.read_exact(&mut body, deadline)?;
        Ok((opcode, serde_json::from_slice(&body)?))
    }

    fn read_exact(&mut self, mut bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
        while !bytes.is_empty() {
            remaining(deadline)?;
            match self.stream.read(bytes) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => bytes = &mut bytes[n..],
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.wait_ready(libc::POLLIN, deadline)?
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn wait_ready(&self, events: libc::c_short, deadline: Instant) -> io::Result<()> {
        loop {
            let timeout = remaining(deadline)?
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32;
            let mut fd = libc::pollfd {
                fd: self.stream.as_raw_fd(),
                events,
                revents: 0,
            };
            // SAFETY: fd points to one initialized pollfd for a live socket;
            // poll borrows the array only for this call.
            let result = unsafe { libc::poll(&mut fd, 1, timeout) };
            if result > 0 {
                return Ok(());
            }
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "Discord IPC deadline exceeded"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn pair() -> (IpcClient, IpcClient) {
        let (a, b) = UnixStream::pair().unwrap();
        (
            IpcClient::from_stream(a).unwrap(),
            IpcClient::from_stream(b).unwrap(),
        )
    }

    #[test]
    fn thousands_of_updates_drain_replies_and_handle_ping() {
        let (mut client, mut server) = pair();
        let worker = thread::spawn(move || {
            for _ in 0..2000 {
                let deadline = Instant::now() + IO_TIMEOUT;
                let (_, request) = server.receive(deadline).unwrap();
                server.send(3, &json!({"ping": true}), deadline).unwrap();
                let (opcode, pong) = server.receive(deadline).unwrap();
                assert_eq!(opcode, 4);
                assert_eq!(pong, json!({"ping": true}));
                server
                    .send(
                        1,
                        &json!({"cmd": "SET_ACTIVITY", "nonce": request["nonce"], "data": null}),
                        deadline,
                    )
                    .unwrap();
            }
        });
        for _ in 0..2000 {
            client
                .set_activity(Some(Activity::new().details("Song")))
                .unwrap();
        }
        worker.join().unwrap();
    }

    #[test]
    fn discord_errors_are_failures_including_clear() {
        let (mut client, mut server) = pair();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + IO_TIMEOUT;
            let (_, request) = server.receive(deadline).unwrap();
            assert!(request["args"]["activity"].is_null());
            server
                .send(
                    1,
                    &json!({"cmd": "SET_ACTIVITY", "nonce": request["nonce"],
                "evt": "ERROR", "data": {"message": "invalid asset"}}),
                    deadline,
                )
                .unwrap();
        });
        let error = client.set_activity(None).unwrap_err();
        assert!(error.to_string().contains("invalid asset"));
        worker.join().unwrap();
    }

    #[test]
    fn silent_discord_cannot_block_forever() {
        let (mut client, _server) = pair();
        let started = Instant::now();
        assert!(
            client
                .set_activity_with_timeout(None, Duration::from_millis(50))
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn partial_reply_and_unrelated_nonce_cannot_extend_the_deadline() {
        let (mut client, mut server) = pair();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + IO_TIMEOUT;
            server.receive(deadline).unwrap();
            server
                .send(
                    1,
                    &json!({"cmd": "SET_ACTIVITY", "nonce": "unrelated"}),
                    deadline,
                )
                .unwrap();
            // Begin another frame but never finish its header.
            server.stream.write_all(&[1, 0]).unwrap();
            std::thread::sleep(Duration::from_millis(200));
        });
        let started = Instant::now();
        let error = client
            .set_activity_with_timeout(None, Duration::from_millis(50))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        worker.join().unwrap();
    }

    #[test]
    fn oversized_reply_is_rejected_before_allocating_body() {
        let (mut client, mut server) = pair();
        server.stream.write_all(&1u32.to_le_bytes()).unwrap();
        server.stream.write_all(&u32::MAX.to_le_bytes()).unwrap();
        assert!(
            client
                .receive(Instant::now() + IO_TIMEOUT)
                .unwrap_err()
                .to_string()
                .contains("too large")
        );
    }
}
