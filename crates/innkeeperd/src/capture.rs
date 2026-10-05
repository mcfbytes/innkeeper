use std::fmt::Display;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const BYTES_PER_LINE: usize = 16;
/// Bytes of one direction closer together than this share a capture line (2400 baud sends one every 4 ms).
const RUN_GAP: Duration = Duration::from_millis(50);
const MAX_RUN_BYTES: usize = 4096;

/// Byte direction, labelled from the client's point of view as everywhere in this project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    FromClient,
    ToClient,
}

impl Direction {
    fn label(self) -> &'static str {
        match self {
            Direction::FromClient => "tx",
            Direction::ToClient => "rx",
        }
    }
}

/// Consecutive bytes of one direction, written as one line group once the run ends.
#[derive(Debug)]
struct Run {
    direction: Direction,
    first: Instant,
    last: Instant,
    bytes: Vec<u8>,
}

/// A per-session text log of every byte and decoded event; the format is in innkeeperd.md.
#[derive(Debug)]
pub(crate) struct Capture<W: Write> {
    out: W,
    started: Instant,
    run: Option<Run>,
}

impl Capture<BufWriter<File>> {
    pub(crate) fn create(
        dir: &Path,
        session: u64,
        peer: SocketAddr,
        started: Instant,
    ) -> io::Result<(Self, PathBuf)> {
        fs::create_dir_all(dir)?;
        let stamp = humantime::format_rfc3339_seconds(SystemTime::now()).to_string();
        let path = dir.join(format!(
            "{}-session{session}.hexlog",
            stamp.replace(':', "")
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mut capture = Capture::new(BufWriter::new(file), started);
        writeln!(
            capture.out,
            "# innkeeperd session {session} from {peer}, started {stamp}"
        )?;
        Ok((capture, path))
    }
}

impl<W: Write> Capture<W> {
    fn new(out: W, started: Instant) -> Self {
        Capture {
            out,
            started,
            run: None,
        }
    }

    pub(crate) fn record_bytes(
        &mut self,
        direction: Direction,
        now: Instant,
        bytes: &[u8],
    ) -> io::Result<()> {
        let continues_run = self.run.as_ref().is_some_and(|run| {
            run.direction == direction
                && now.saturating_duration_since(run.last) < RUN_GAP
                && run.bytes.len() < MAX_RUN_BYTES
        });
        if !continues_run {
            self.finish_run()?;
        }
        let run = self.run.get_or_insert_with(|| Run {
            direction,
            first: now,
            last: now,
            bytes: Vec::new(),
        });
        run.last = now;
        run.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub(crate) fn record_event(&mut self, now: Instant, event: &dyn Display) -> io::Result<()> {
        self.finish_run()?;
        writeln!(self.out, "{} ev {event}", self.stamp(now))?;
        self.out.flush()
    }

    /// When the pending run stops growing and should be written.
    pub(crate) fn run_deadline(&self) -> Option<Instant> {
        self.run.as_ref().map(|run| run.last + RUN_GAP)
    }

    /// Writes any pending run; also call when the session ends.
    pub(crate) fn finish_run(&mut self) -> io::Result<()> {
        let Some(run) = self.run.take() else {
            return Ok(());
        };
        let stamp = self.stamp(run.first);
        let label = run.direction.label();
        for chunk in run.bytes.chunks(BYTES_PER_LINE) {
            let hex: Vec<String> = chunk.iter().map(|byte| format!("{byte:02x}")).collect();
            let ascii: String = chunk.iter().map(|&byte| printable(byte)).collect();
            writeln!(self.out, "{stamp} {label} {:<47} |{ascii}|", hex.join(" "))?;
        }
        self.out.flush()
    }

    fn stamp(&self, now: Instant) -> String {
        format!(
            "{:>10.3}",
            now.saturating_duration_since(self.started).as_secs_f64()
        )
    }
}

fn printable(byte: u8) -> char {
    if byte.is_ascii_graphic() || byte == b' ' {
        char::from(byte)
    } else {
        '.'
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn lines_carry_time_direction_hex_and_ascii() {
        let started = Instant::now();
        let mut capture = Capture::new(Vec::new(), started);
        let later = started + Duration::from_millis(1500);
        capture
            .record_bytes(Direction::FromClient, later, b"\rAT\r")
            .unwrap();
        capture
            .record_bytes(Direction::ToClient, later, &[0x81, 0x49, 0x62, 0x90, 0x82])
            .unwrap();
        capture.record_event(later, &"message len=1 22").unwrap();
        let text = String::from_utf8(capture.out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            format!("     1.500 tx {:<47} |.AT.|", "0d 41 54 0d")
        );
        assert_eq!(
            lines[1],
            format!("     1.500 rx {:<47} |.Ib..|", "81 49 62 90 82")
        );
        assert_eq!(lines[2], "     1.500 ev message len=1 22");
    }

    #[test]
    fn long_writes_wrap_at_sixteen_bytes() {
        let started = Instant::now();
        let mut capture = Capture::new(Vec::new(), started);
        capture
            .record_bytes(Direction::ToClient, started, &[0x41; 20])
            .unwrap();
        capture.finish_run().unwrap();
        assert_eq!(String::from_utf8(capture.out).unwrap().lines().count(), 2);
    }

    #[test]
    fn bytes_arriving_close_together_share_a_line_stamped_at_the_first() {
        let started = Instant::now();
        let mut capture = Capture::new(Vec::new(), started);
        for index in 0..3u64 {
            let at = started + Duration::from_millis(1000 + index * 4);
            capture
                .record_bytes(Direction::FromClient, at, &[0x80 + index as u8])
                .unwrap();
        }
        let after_gap = started + Duration::from_millis(2000);
        capture
            .record_bytes(Direction::FromClient, after_gap, &[0x0d])
            .unwrap();
        capture
            .record_bytes(Direction::ToClient, after_gap, &[0x40])
            .unwrap();
        capture.finish_run().unwrap();
        let text = String::from_utf8(capture.out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("     1.000 tx 80 81 82"));
        assert!(lines[1].starts_with("     2.000 tx 0d"));
        assert!(lines[2].starts_with("     2.000 rx 40"));
    }

    #[test]
    fn a_pending_run_reports_when_it_should_be_written() {
        let started = Instant::now();
        let mut capture = Capture::new(Vec::new(), started);
        assert_eq!(capture.run_deadline(), None);
        capture
            .record_bytes(Direction::FromClient, started, b"@")
            .unwrap();
        assert_eq!(capture.run_deadline(), Some(started + RUN_GAP));
    }
}
