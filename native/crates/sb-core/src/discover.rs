//! Finding the server on the local network when no address is saved. The server announces itself as
//! `_switchboard._tcp` with `url=<address>` in its TXT record (server/bonjour.py); this asks for it with one
//! multicast DNS question from an ordinary port, so the answers come straight back to us (RFC 6762 section 6.7)
//! and nothing else on the machine has to share port 5353.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const SERVICE: &str = "_switchboard._tcp.local";
const GROUP: (Ipv4Addr, u16) = (Ipv4Addr::new(224, 0, 0, 251), 5353);
const PTR: u16 = 12;
const TXT: u16 = 16;
const SRV: u16 = 33;
const A: u16 = 1;

/// Server addresses announced on this network ("http://studio.local:8130"), in the order they answered.
/// Empty when nothing answered within `wait`, or the network does not allow multicast.
pub fn browse(wait: Duration) -> Vec<String> {
    match ask(wait) {
        Ok(v) => v,
        Err(e) => {
            log::info!("local network search failed: {e}");
            Vec::new()
        }
    }
}

fn ask(wait: Duration) -> std::io::Result<Vec<String>> {
    let sock = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))?;
    sock.set_multicast_ttl_v4(255)?;
    let to = SocketAddr::from(GROUP);
    sock.send_to(&query(1, &[(SERVICE, PTR)]), to)?;
    let mut found = Found::default();
    let mut asked_again = false;
    let end = Instant::now() + wait;
    let mut buf = [0u8; 9000];
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        sock.set_read_timeout(Some(left))?;
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => found.take(&buf[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
            Err(e) => return Err(e),
        }
        // A responder that only listed the name: ask for its record once.
        let bare: Vec<String> = found.instances.iter().filter(|i| found.url(i).is_none()).cloned().collect();
        if !bare.is_empty() && !asked_again {
            asked_again = true;
            let qs: Vec<(&str, u16)> = bare.iter().flat_map(|i| [(i.as_str(), TXT), (i.as_str(), SRV)]).collect();
            let _ = sock.send_to(&query(2, &qs), to);
        }
    }
    Ok(found.urls())
}

/// What the answers said so far.
#[derive(Default, Debug)]
struct Found {
    instances: Vec<String>,
    txt: HashMap<String, String>,
    srv: HashMap<String, (String, u16)>,
    a: HashMap<String, Ipv4Addr>,
}

impl Found {
    fn take(&mut self, packet: &[u8]) {
        let Some(records) = parse(packet) else { return };
        for r in records {
            match r.kind {
                PTR if r.name.eq_ignore_ascii_case(SERVICE) => {
                    if let Some((target, _)) = name_at(packet, r.data_at)
                        && !self.instances.contains(&target)
                    {
                        self.instances.push(target);
                    }
                }
                TXT => {
                    if let Some(u) = txt_url(r.data) {
                        self.txt.insert(r.name.to_ascii_lowercase(), u);
                    }
                }
                SRV if r.data.len() > 6 => {
                    let port = u16::from_be_bytes([r.data[4], r.data[5]]);
                    if let Some((host, _)) = name_at(packet, r.data_at + 6) {
                        self.srv.insert(r.name.to_ascii_lowercase(), (host, port));
                    }
                }
                A if r.data.len() == 4 => {
                    self.a.insert(r.name.to_ascii_lowercase(), Ipv4Addr::new(r.data[0], r.data[1], r.data[2], r.data[3]));
                }
                _ => {}
            }
        }
    }

    /// The instance's address: its TXT url, else its SRV host and port.
    fn url(&self, instance: &str) -> Option<String> {
        let key = instance.to_ascii_lowercase();
        if let Some(u) = self.txt.get(&key) {
            return Some(u.clone());
        }
        let (host, port) = self.srv.get(&key)?;
        let host = self.a.get(&host.to_ascii_lowercase()).map(|ip| ip.to_string()).unwrap_or_else(|| host.clone());
        Some(format!("http://{host}:{port}"))
    }

    fn urls(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for u in self.instances.iter().filter_map(|i| self.url(i)) {
            let u = u.trim_end_matches('/').to_string();
            if !out.contains(&u) {
                out.push(u);
            }
        }
        out
    }
}

/// A question packet for each (name, type), asking for answers by unicast.
fn query(id: u16, questions: &[(&str, u16)]) -> Vec<u8> {
    let mut p = Vec::with_capacity(64);
    p.extend_from_slice(&id.to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(&(questions.len() as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    for (name, kind) in questions {
        for label in name.trim_end_matches('.').split('.') {
            let b = label.as_bytes();
            p.push(b.len().min(63) as u8);
            p.extend_from_slice(&b[..b.len().min(63)]);
        }
        p.push(0);
        p.extend_from_slice(&kind.to_be_bytes());
        // class IN with the unicast-response bit
        p.extend_from_slice(&0x8001u16.to_be_bytes());
    }
    p
}

struct Record<'a> {
    name: String,
    kind: u16,
    data: &'a [u8],
    data_at: usize,
}

/// Every answer, authority and additional record in a reply. None when it is not a well-formed reply.
fn parse(p: &[u8]) -> Option<Vec<Record<'_>>> {
    if p.len() < 12 || p[2] & 0x80 == 0 {
        return None;
    }
    let count = |i: usize| u16::from_be_bytes([p[i], p[i + 1]]) as usize;
    let (qd, rrs) = (count(4), count(6) + count(8) + count(10));
    let mut at = 12;
    for _ in 0..qd {
        at = name_at(p, at)?.1 + 4;
    }
    let mut out = Vec::new();
    for _ in 0..rrs {
        let (name, next) = name_at(p, at)?;
        let head = p.get(next..next + 10)?;
        let kind = u16::from_be_bytes([head[0], head[1]]);
        let len = u16::from_be_bytes([head[8], head[9]]) as usize;
        let data_at = next + 10;
        let data = p.get(data_at..data_at + len)?;
        out.push(Record { name, kind, data, data_at });
        at = data_at + len;
    }
    Some(out)
}

/// The dotted name at `at`, following compression pointers, and where the bytes after it start.
fn name_at(p: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut end = None;
    for _ in 0..64 {
        let len = *p.get(at)? as usize;
        if len == 0 {
            return Some((labels.join("."), end.unwrap_or(at + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let to = ((len & 0x3F) << 8) | *p.get(at + 1)? as usize;
            end.get_or_insert(at + 2);
            at = to;
            continue;
        }
        labels.push(String::from_utf8_lossy(p.get(at + 1..at + 1 + len)?).into_owned());
        at += 1 + len;
    }
    None
}

/// `url=...` from a TXT record's strings.
fn txt_url(data: &[u8]) -> Option<String> {
    let mut at = 0;
    while at < data.len() {
        let len = data[at] as usize;
        let s = data.get(at + 1..at + 1 + len)?;
        if let Some(v) = std::str::from_utf8(s).ok().and_then(|s| s.strip_prefix("url="))
            && (v.starts_with("http://") || v.starts_with("https://"))
        {
            return Some(v.to_string());
        }
        at += 1 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(out: &mut Vec<u8>, n: &str) {
        for l in n.split('.') {
            out.push(l.len() as u8);
            out.extend_from_slice(l.as_bytes());
        }
        out.push(0);
    }

    fn rr(out: &mut Vec<u8>, owner: &[u8], kind: u16, data: &[u8]) {
        out.extend_from_slice(owner);
        out.extend_from_slice(&kind.to_be_bytes());
        out.extend_from_slice(&[0x80, 1, 0, 0, 0x11, 0x94]);
        out.extend_from_slice(&(data.len() as u16).to_be_bytes());
        out.extend_from_slice(data);
    }

    /// A reply like dns-sd's: the PTR answer, then TXT, SRV and A as additional records, names compressed.
    fn reply(with_txt: bool) -> Vec<u8> {
        let mut p = vec![0, 1, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, if with_txt { 3 } else { 2 }];
        let service_at = p.len() as u16;
        let mut owner = Vec::new();
        name(&mut owner, SERVICE);
        let mut ptr = Vec::new();
        ptr.push(22);
        ptr.extend_from_slice(b"Switchboard on studio1");
        ptr.extend_from_slice(&(0xC000 | service_at).to_be_bytes());
        rr(&mut p, &owner, PTR, &ptr);
        let instance_at = (p.len() - ptr.len()) as u16;
        let inst = (0xC000 | instance_at).to_be_bytes();
        if with_txt {
            let mut txt = Vec::new();
            for s in ["v=1", "url=http://192.0.2.10:8130"] {
                txt.push(s.len() as u8);
                txt.extend_from_slice(s.as_bytes());
            }
            rr(&mut p, &inst, TXT, &txt);
        }
        let mut srv = vec![0, 0, 0, 0, 0x1F, 0xC2];
        name(&mut srv, "studio.local");
        rr(&mut p, &inst, SRV, &srv);
        let mut host = Vec::new();
        name(&mut host, "studio.local");
        rr(&mut p, &host, A, &[192, 0, 2, 11]);
        p
    }

    #[test]
    fn reads_the_announced_url() {
        let mut f = Found::default();
        f.take(&reply(true));
        assert_eq!(f.instances, vec!["Switchboard on studio1._switchboard._tcp.local"]);
        assert_eq!(f.urls(), vec!["http://192.0.2.10:8130"]);
    }

    #[test]
    fn falls_back_to_the_service_record() {
        let mut f = Found::default();
        f.take(&reply(false));
        assert_eq!(f.urls(), vec!["http://192.0.2.11:8130"]);
    }

    #[test]
    fn ignores_questions_and_junk() {
        let mut f = Found::default();
        f.take(&query(1, &[(SERVICE, PTR)]));
        f.take(&[0xFF; 5]);
        f.take(&reply(true)[..40]);
        assert!(f.urls().is_empty());
    }

    #[test]
    fn a_question_names_the_service() {
        let q = query(7, &[(SERVICE, PTR)]);
        assert_eq!(&q[..6], &[0, 7, 0, 0, 0, 1]);
        assert_eq!(name_at(&q, 12).unwrap().0, SERVICE);
    }
}
