//! The Windows setup file: the installer exe with the app's files attached to its end. One download, no zip to
//! unpack first, and the attached files are read straight from the exe the user opened.
//!
//!   Switchboard-Setup-<version>.exe = installer exe | pack | pack length (u64 LE) | "SBSETUP!"
//!   pack = "SBPACK01" | count (u32 LE) | per file: name length (u16 LE), name, size (u64 LE),
//!          compressed length (u64 LE), zlib data
//!
//! Signing a file like this adds the certificate after the end marker (with up to 7 bytes of padding first), so the
//! reader looks for the marker where the PE header says the certificate starts. zlib's checksum and the stated size
//! catch a damaged download.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

const HEAD: &[u8; 8] = b"SBPACK01";
const TAIL: &[u8; 8] = b"SBSETUP!";

/// Writes `installer` with `files` (name in the pack, file on disk) attached as `out`. Returns the size of `out`.
pub fn make(installer: &Path, files: &[(String, &Path)], out: &Path) -> io::Result<u64> {
    if attached(installer)?.is_some() {
        return Err(io::Error::other(format!("{} already has files attached", installer.display())));
    }
    let mut w = BufWriter::new(File::create(out)?);
    let base = io::copy(&mut File::open(installer)?, &mut w)?;
    w.write_all(HEAD)?;
    w.write_all(&u32::try_from(files.len()).map_err(io::Error::other)?.to_le_bytes())?;
    let mut len = HEAD.len() as u64 + 4;
    for (name, path) in files {
        if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
            return Err(io::Error::other(format!("{name:?} is not a plain file name")));
        }
        let raw = std::fs::read(path)?;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&raw)?;
        let packed = z.finish()?;
        let n = name.as_bytes();
        w.write_all(&u16::try_from(n.len()).map_err(io::Error::other)?.to_le_bytes())?;
        w.write_all(n)?;
        w.write_all(&(raw.len() as u64).to_le_bytes())?;
        w.write_all(&(packed.len() as u64).to_le_bytes())?;
        w.write_all(&packed)?;
        len += 2 + n.len() as u64 + 16 + packed.len() as u64;
    }
    w.write_all(&len.to_le_bytes())?;
    w.write_all(TAIL)?;
    w.flush()?;
    Ok(base + len + 16)
}

/// Where the pack sits in `exe` (offset, length), or None when nothing is attached.
pub fn attached(exe: &Path) -> io::Result<Option<(u64, u64)>> {
    let mut f = File::open(exe)?;
    let size = f.metadata()?.len();
    let end = cert_start(&mut f)?.filter(|&c| c <= size).unwrap_or(size);
    // The marker ends right at `end`, or a few zero bytes before it when a signature was added.
    let back = end.min(16 + 8);
    if back < 16 {
        return Ok(None);
    }
    let mut buf = vec![0u8; back as usize];
    f.seek(SeekFrom::Start(end - back))?;
    f.read_exact(&mut buf)?;
    for pad in 0..=7usize {
        let Some(stop) = buf.len().checked_sub(pad) else { break };
        if stop < 16 || buf[stop..].iter().any(|&b| b != 0) {
            continue;
        }
        if &buf[stop - 8..stop] != TAIL {
            continue;
        }
        let len = u64::from_le_bytes(buf[stop - 16..stop - 8].try_into().unwrap_or_default());
        let tail_at = end - pad as u64 - 16;
        let Some(start) = tail_at.checked_sub(len) else { return Ok(None) };
        let mut head = [0u8; 8];
        f.seek(SeekFrom::Start(start))?;
        f.read_exact(&mut head)?;
        return Ok((&head == HEAD).then_some((start, len)));
    }
    Ok(None)
}

/// Unpacks the files attached to `exe` into `to` (created). Returns their names. Fails on anything damaged.
pub fn extract(exe: &Path, to: &Path) -> io::Result<Vec<String>> {
    let (start, len) = attached(exe)?.ok_or_else(|| io::Error::other("no files are attached to this installer"))?;
    let mut f = BufReader::new(File::open(exe)?);
    f.seek(SeekFrom::Start(start))?;
    let mut r = f.take(len);
    let mut head = [0u8; 12];
    r.read_exact(&mut head)?;
    let count = u32::from_le_bytes(head[8..12].try_into().unwrap_or_default());
    std::fs::create_dir_all(to)?;
    let mut names = vec![];
    for _ in 0..count {
        let mut n = [0u8; 2];
        r.read_exact(&mut n)?;
        let mut name = vec![0u8; u16::from_le_bytes(n) as usize];
        r.read_exact(&mut name)?;
        let name = String::from_utf8(name).map_err(io::Error::other)?;
        if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
            return Err(io::Error::other(format!("the installer holds a file with a bad name ({name:?})")));
        }
        let mut sizes = [0u8; 16];
        r.read_exact(&mut sizes)?;
        let raw = u64::from_le_bytes(sizes[..8].try_into().unwrap_or_default());
        let packed = u64::from_le_bytes(sizes[8..].try_into().unwrap_or_default());
        let mut out = BufWriter::new(File::create(to.join(&name))?);
        let mut z = flate2::read::ZlibDecoder::new((&mut r).take(packed));
        let wrote = io::copy(&mut z, &mut out)?;
        out.flush()?;
        // The decoder may stop before reading the last bytes of its input; skip to the next file either way.
        io::copy(&mut z.into_inner(), &mut io::sink())?;
        if wrote != raw {
            return Err(io::Error::other(format!("{name} in the installer is damaged ({wrote} of {raw} bytes)")));
        }
        names.push(name);
    }
    Ok(names)
}

/// The file offset of the Authenticode certificate table, from the PE header, if the file is signed.
fn cert_start(f: &mut File) -> io::Result<Option<u64>> {
    let mut dos = [0u8; 64];
    f.seek(SeekFrom::Start(0))?;
    if f.read_exact(&mut dos).is_err() || &dos[..2] != b"MZ" {
        return Ok(None);
    }
    let pe = u32::from_le_bytes(dos[0x3C..0x40].try_into().unwrap_or_default()) as u64;
    let mut hdr = [0u8; 24 + 2];
    f.seek(SeekFrom::Start(pe))?;
    if f.read_exact(&mut hdr).is_err() || &hdr[..4] != b"PE\0\0" {
        return Ok(None);
    }
    // The data directories follow the optional header's fixed part: 96 bytes in PE32, 112 in PE32+. The
    // certificate table is entry 4, and its address is a file offset (not an RVA like the others).
    let dirs = match u16::from_le_bytes([hdr[24], hdr[25]]) {
        0x10b => 96,
        0x20b => 112,
        _ => return Ok(None),
    };
    let mut cert = [0u8; 8];
    f.seek(SeekFrom::Start(pe + 24 + dirs + 4 * 8))?;
    f.read_exact(&mut cert)?;
    let at = u32::from_le_bytes(cert[..4].try_into().unwrap_or_default()) as u64;
    let size = u32::from_le_bytes(cert[4..].try_into().unwrap_or_default());
    Ok((at > 0 && size > 0).then_some(at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-install-pack-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A minimal PE32+ header, with the certificate table at `cert` when given.
    fn fake_exe(cert: Option<(u32, u32)>) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[..2].copy_from_slice(b"MZ");
        b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x80 + 24..0x80 + 26].copy_from_slice(&0x20bu16.to_le_bytes());
        if let Some((at, size)) = cert {
            let d = 0x80 + 24 + 112 + 32;
            b[d..d + 4].copy_from_slice(&at.to_le_bytes());
            b[d + 4..d + 8].copy_from_slice(&size.to_le_bytes());
        }
        b
    }

    #[test]
    fn round_trip() {
        let d = dir("trip");
        std::fs::write(d.join("installer.exe"), fake_exe(None)).unwrap();
        let big: Vec<u8> = (0..300_000u32).flat_map(|i| (i % 251).to_le_bytes()).collect();
        std::fs::write(d.join("a.exe"), &big).unwrap();
        std::fs::write(d.join("b.txt"), "notices").unwrap();
        std::fs::write(d.join("empty"), "").unwrap();
        assert_eq!(attached(&d.join("installer.exe")).unwrap(), None);
        let files = [
            ("Switchboard.exe".to_string(), d.join("a.exe")),
            ("THIRD-PARTY-NOTICES.txt".into(), d.join("b.txt")),
            ("empty.txt".into(), d.join("empty")),
        ];
        let refs: Vec<(String, &Path)> = files.iter().map(|(n, p)| (n.clone(), p.as_path())).collect();
        let size = make(&d.join("installer.exe"), &refs, &d.join("setup.exe")).unwrap();
        assert_eq!(size, std::fs::metadata(d.join("setup.exe")).unwrap().len());
        assert!(size < 512 + big.len() as u64 / 4, "compressed: {size}");
        let names = extract(&d.join("setup.exe"), &d.join("out")).unwrap();
        assert_eq!(names, ["Switchboard.exe", "THIRD-PARTY-NOTICES.txt", "empty.txt"]);
        assert_eq!(std::fs::read(d.join("out/Switchboard.exe")).unwrap(), big);
        assert_eq!(std::fs::read_to_string(d.join("out/THIRD-PARTY-NOTICES.txt")).unwrap(), "notices");
        assert!(make(&d.join("setup.exe"), &refs, &d.join("twice.exe")).is_err(), "attached twice");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn found_after_a_signature_is_added() {
        let d = dir("signed");
        std::fs::write(d.join("installer.exe"), fake_exe(None)).unwrap();
        std::fs::write(d.join("a"), "app").unwrap();
        make(&d.join("installer.exe"), &[("A.exe".into(), &d.join("a"))], &d.join("setup.exe")).unwrap();
        // What signtool does: pad to 8 bytes, append the certificate, point the header at it.
        let mut b = std::fs::read(d.join("setup.exe")).unwrap();
        while !b.len().is_multiple_of(8) {
            b.push(0);
        }
        let at = b.len() as u32;
        b.extend_from_slice(&[0xAB; 40]);
        b[..512].copy_from_slice(&fake_exe(Some((at, 40))));
        std::fs::write(d.join("setup.exe"), &b).unwrap();
        assert!(attached(&d.join("setup.exe")).unwrap().is_some());
        extract(&d.join("setup.exe"), &d.join("out")).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("out/A.exe")).unwrap(), "app");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn damage_is_caught() {
        let d = dir("damaged");
        std::fs::write(d.join("installer.exe"), fake_exe(None)).unwrap();
        std::fs::write(d.join("a"), "x".repeat(10_000)).unwrap();
        make(&d.join("installer.exe"), &[("A.exe".into(), &d.join("a"))], &d.join("setup.exe")).unwrap();
        let mut b = std::fs::read(d.join("setup.exe")).unwrap();
        let mid = 512 + 40;
        b[mid] ^= 0xFF;
        std::fs::write(d.join("setup.exe"), &b).unwrap();
        assert!(extract(&d.join("setup.exe"), &d.join("out")).is_err());
        assert!(make(&d.join("installer.exe"), &[("../x".into(), &d.join("a"))], &d.join("bad.exe")).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
