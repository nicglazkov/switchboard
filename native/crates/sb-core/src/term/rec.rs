//! The terminal recorder (spec 2026-10-14, terminal pipeline). With `SB_TERM_REC=1` every terminal's connection is
//! written to a capture in `~/Library/Logs/Switchboard/term/` (on other systems the local data folder's
//! `Switchboard/term`; `SB_TERM_REC=<folder>` picks another): each frame both ways with its time, so a glitch can be
//! replayed through the same parser at the same sizes (`cargo run -p sb-core --features grid --example term_replay`).
//!
//! The format is plain enough to read in a hex viewer: a first line `SBREC1`, then per frame a header line
//! `<ms since start> <kind> <length>`, the frame's bytes, and a newline. Kinds: `o` output from the hub (binary),
//! `T` a text frame from the hub, `i` input to the hub (binary), `t` a text frame to the hub, `c` a note from the
//! client (where it connected, why it closed).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const MAGIC: &[u8] = b"SBREC1\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Out,
    TextIn,
    Input,
    TextOut,
    Note,
}

impl Kind {
    fn letter(self) -> char {
        match self {
            Kind::Out => 'o',
            Kind::TextIn => 'T',
            Kind::Input => 'i',
            Kind::TextOut => 't',
            Kind::Note => 'c',
        }
    }

    fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "o" => Kind::Out,
            "T" => Kind::TextIn,
            "i" => Kind::Input,
            "t" => Kind::TextOut,
            "c" => Kind::Note,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub ms: u64,
    pub kind: Kind,
    pub bytes: Vec<u8>,
}

pub struct Recorder {
    out: BufWriter<File>,
    start: Instant,
    pub path: PathBuf,
}

/// Where captures go, if recording is on.
pub fn dir() -> Option<PathBuf> {
    let v = std::env::var("SB_TERM_REC").ok().filter(|v| !v.is_empty() && v != "0")?;
    if v != "1" {
        return Some(PathBuf::from(v));
    }
    if cfg!(target_os = "macos") {
        return dirs::home_dir().map(|h| h.join("Library/Logs/Switchboard/term"));
    }
    dirs::data_local_dir().map(|d| d.join("Switchboard/term"))
}

impl Recorder {
    /// A new capture in `dir`, named for the time and `label`.
    pub fn create(dir: &Path, label: &str) -> std::io::Result<Recorder> {
        std::fs::create_dir_all(dir)?;
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let label: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).take(40).collect();
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = dir.join(format!("{stamp}-{}-{n}-{label}.rec", std::process::id()));
        let mut out = BufWriter::new(File::create(&path)?);
        out.write_all(MAGIC)?;
        Ok(Recorder { out, start: Instant::now(), path })
    }

    /// Recording is on (`SB_TERM_REC`): a capture for this connection, or None.
    pub fn from_env(label: &str) -> Option<Recorder> {
        let d = dir()?;
        match Recorder::create(&d, label) {
            Ok(r) => Some(r),
            Err(e) => {
                log::warn!("terminal recorder: {e}");
                None
            }
        }
    }

    pub fn frame(&mut self, kind: Kind, bytes: &[u8]) {
        let ms = self.start.elapsed().as_millis() as u64;
        let _ = writeln!(self.out, "{ms} {} {}", kind.letter(), bytes.len());
        let _ = self.out.write_all(bytes);
        let _ = self.out.write_all(b"\n");
        // a capture is for finding a glitch, often right before something went wrong: keep it on disk as it goes
        let _ = self.out.flush();
    }
}

/// Reads a capture back.
pub fn read(data: &[u8]) -> Result<Vec<Frame>, String> {
    let mut rest = data.strip_prefix(MAGIC).ok_or("not a terminal capture (no SBREC1 line)")?;
    let mut frames = Vec::new();
    while !rest.is_empty() {
        let nl = rest.iter().position(|&b| b == b'\n').ok_or("a header with no end")?;
        let head = std::str::from_utf8(&rest[..nl]).map_err(|_| "a header that is not text")?;
        let mut parts = head.split(' ');
        let (Some(ms), Some(kind), Some(len), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return Err(format!("a bad header: {head:?}"));
        };
        let ms: u64 = ms.parse().map_err(|_| format!("a bad time: {head:?}"))?;
        let kind = Kind::parse(kind).ok_or_else(|| format!("a bad kind: {head:?}"))?;
        let len: usize = len.parse().map_err(|_| format!("a bad length: {head:?}"))?;
        let body = rest.get(nl + 1..nl + 1 + len).ok_or("a frame cut short")?;
        frames.push(Frame { ms, kind, bytes: body.to_vec() });
        rest = rest.get(nl + 2 + len..).unwrap_or(&[]);
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("sbrec-{}", std::process::id()));
        let mut r = Recorder::create(&dir, "lab c03ef0d9").unwrap();
        r.frame(Kind::Note, b"connect cols=80&rows=24");
        r.frame(Kind::TextIn, br#"{"type":"size","cols":80,"rows":24}"#);
        r.frame(Kind::Out, b"\x1b[1mhi\nthere\n");
        r.frame(Kind::Input, b"");
        let path = r.path.clone();
        drop(r);
        let frames = read(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[2].kind, Kind::Out);
        assert_eq!(frames[2].bytes, b"\x1b[1mhi\nthere\n");
        assert_eq!(frames[3].bytes, b"");
        assert!(path.file_name().unwrap().to_string_lossy().ends_with("lab_c03ef0d9.rec"));
        let _ = std::fs::remove_dir_all(dir);
        assert!(read(b"nope").is_err());
        assert!(read(b"SBREC1\n5 o 10\nshort").is_err());
    }
}
