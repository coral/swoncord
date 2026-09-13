//! Bounded persistent logs for launches from Finder and at login.

use pretty_env_logger::env_logger::{Target, WriteStyle};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

pub fn init() {
    let file = std::env::var_os("HOME")
        .ok_or_else(|| io::Error::other("HOME is unset"))
        .and_then(|home| {
            RotatingFile::open(
                PathBuf::from(home).join("Library/Logs/Swoncord/swoncord.log"),
                MAX_LOG_BYTES,
            )
        });
    let file = match file {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!("Cannot open Swoncord log file: {error}");
            None
        }
    };
    pretty_env_logger::formatted_builder()
        .parse_filters(&std::env::var("RUST_LOG").unwrap_or_else(|_| "swoncord=info".into()))
        .write_style(WriteStyle::Never)
        .format(|buf, record| {
            writeln!(
                buf,
                "{} {} {}: {}",
                buf.timestamp_millis(),
                record.level(),
                record.target(),
                record.args()
            )
        })
        .target(Target::Pipe(Box::new(LogOutput(file))))
        .init();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        log::error!("{panic}");
        previous(panic);
    }));
}

struct LogOutput(Option<RotatingFile>);

impl Write for LogOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(bytes);
        if let Some(file) = &mut self.0
            && let Err(error) = file.write_all(bytes)
        {
            eprintln!("Cannot write Swoncord log file: {error}");
            self.0 = None;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.0 {
            file.flush()?;
        }
        Ok(())
    }
}

struct RotatingFile {
    path: PathBuf,
    file: File,
    size: u64,
    limit: u64,
}

impl RotatingFile {
    fn open(path: PathBuf, limit: u64) -> io::Result<Self> {
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("invalid log path"))?,
        )?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            path,
            file,
            size,
            limit,
        })
    }
}

impl Write for RotatingFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.size > 0 && self.size + bytes.len() as u64 > self.limit {
            fs::rename(&self.path, self.path.with_extension("previous.log"))?;
            self.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            self.size = 0;
        }
        let written = self.file.write(bytes)?;
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_survive_restart_and_rotate_during_a_long_session() {
        let dir = std::env::temp_dir().join(format!("swoncord-log-test-{}", std::process::id()));
        let path = dir.join("swoncord.log");
        {
            let mut log = RotatingFile::open(path.clone(), 10).unwrap();
            log.write_all(b"before\n").unwrap();
        }
        let mut log = RotatingFile::open(path.clone(), 10).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"before\n");
        log.write_all(b"after\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"after\n");
        assert_eq!(
            fs::read(path.with_extension("previous.log")).unwrap(),
            b"before\n"
        );
        log.write_all(b"newest\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"newest\n");
        assert_eq!(
            fs::read(path.with_extension("previous.log")).unwrap(),
            b"after\n"
        );
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }
}
