//! The phone's two loopback ports, and what keeps other apps off them.
//!
//! On a computer the forwarder's port is as open as the server's own port always was. A phone is different:
//! every app on it can connect to 127.0.0.1, and this port leads straight to the home computer. So each
//! connection has to show a secret before one byte of it is carried: the first request on it must have the
//! header `X-Switchboard-Gate: <secret>`. The secret is made new for every launch, lives only in this app's
//! memory, and the app's own HTTP and WebSocket clients add it to what they send to 127.0.0.1.
//!
//! The check reads only the first request's head (to the blank line) and passes it on untouched, header and
//! all; the server does not know the header and skips it. After that the connection is copied byte for byte, as
//! `sb_link::tunnel` does on a computer, so keep-alive requests and a WebSocket's frames need nothing more. A
//! connection that does not show the secret gets `403` and is closed, and nothing of it reaches home.
//!
//! What is behind the gate is the stand-in (`sb_link::standin`, as on a computer): while the home computer
//! answers, a connection is one stream to it, as before. When home has not been reached for a few seconds the
//! same ports answer from what the other computers say about their own chats (away.rs). The secret is the
//! phone's own business, so while away the header is taken out of the head before anything is passed on: a
//! terminal's request goes to another computer then, and that computer has no use for it.

use sb_link::endpoint::Tunnel;
use sb_link::http1;
use sb_link::reach::Others;
use sb_link::standin::StandIn;
use sb_link::tunnel::{self, Target};
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// The header a connection to a loopback port must carry on its first request.
pub const GATE_HEADER: &str = "X-Switchboard-Gate";
/// How long a connection has to send its first request's head.
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

const REFUSAL: &[u8] = b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// A new secret: 32 bytes from the system's random source, as 64 hex digits.
pub fn new_secret() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("the system random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Whether a request head (everything up to and including its blank line) carries the secret. Header names are
/// compared without case, as HTTP has them; the value is compared in constant time.
pub fn admits(head: &[u8], secret: &str) -> bool {
    let mut lines = head.split(|b| *b == b'\n');
    lines.next(); // the request line
    for line in lines {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            break; // the head is over: a header in the body does not count
        }
        let Some(colon) = line.iter().position(|b| *b == b':') else { continue };
        if !line[..colon].eq_ignore_ascii_case(GATE_HEADER.as_bytes()) {
            continue;
        }
        let value = line[colon + 1..].trim_ascii();
        // (a second header with the name could still be the right one, so keep looking)
        if value.len() == secret.len() && bool::from(value.ct_eq(secret.as_bytes())) {
            return true;
        }
    }
    false
}

/// The head without its gate header lines, for when it goes somewhere other than home. Everything else is as it
/// came, byte for byte.
pub fn without_gate(head: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(head.len());
    let mut over = false;
    for (n, line) in head.split_inclusive(|b| *b == b'\n').enumerate() {
        let bare = line.strip_suffix(b"\n").unwrap_or(line);
        let bare = bare.strip_suffix(b"\r").unwrap_or(bare);
        over |= bare.is_empty();
        let named = bare.iter().position(|b| *b == b':').map(|colon| &bare[..colon]);
        let gate = n > 0 && !over && named.is_some_and(|name| name.eq_ignore_ascii_case(GATE_HEADER.as_bytes()));
        if !gate {
            out.extend_from_slice(line);
        }
    }
    out
}

/// A connection whose first bytes were read already (its first request's head, and whatever came with it):
/// they are read again first, and then the connection goes on as it is.
pub struct Replay<S> {
    first: Vec<u8>,
    at: usize,
    rest: S,
}

impl<S> Replay<S> {
    pub fn new(first: Vec<u8>, rest: S) -> Replay<S> {
        Replay { first, at: 0, rest }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Replay<S> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let me = self.get_mut();
        if me.at < me.first.len() {
            let n = (me.first.len() - me.at).min(buf.remaining());
            buf.put_slice(&me.first[me.at..me.at + n]);
            me.at += n;
            if me.at == me.first.len() {
                me.first = Vec::new();
                me.at = 0;
            }
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut me.rest).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Replay<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().rest).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().rest).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().rest).poll_shutdown(cx)
    }
}

/// What the ports carry into right now: the tunnel to home, and the way to the other computers that goes with
/// its endpoint. It is swapped when the endpoint is made again (the network changed, the app came back to the
/// front), and the ports stay as they are.
#[derive(Default)]
pub struct Carrier {
    link: RwLock<Option<(Arc<Tunnel>, Arc<Others>)>>,
}

impl Carrier {
    pub fn set(&self, link: Option<(Arc<Tunnel>, Arc<Others>)>) {
        *self.link.write().unwrap_or_else(|e| e.into_inner()) = link;
    }

    pub fn get(&self) -> Option<(Arc<Tunnel>, Arc<Others>)> {
        self.link.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// One loopback port whose connections, once they have shown the secret, each become a stream to `target` at
/// home, or are answered by the stand-in while home is away.
pub struct Port {
    listener: TcpListener,
    target: Target,
}

impl Port {
    /// Listens on 127.0.0.1: on `wanted` when that port is free (the one this launch used before, so the app's
    /// address stays the same), else on any free one.
    pub async fn bind(wanted: Option<u16>, target: Target) -> io::Result<Port> {
        let at = |port: u16| SocketAddr::from(([127, 0, 0, 1], port));
        let listener = match wanted.filter(|p| *p != 0) {
            Some(p) => match TcpListener::bind(at(p)).await {
                Ok(l) => l,
                Err(_) => TcpListener::bind(at(0)).await?,
            },
            None => TcpListener::bind(at(0)).await?,
        };
        Ok(Port { listener, target })
    }

    pub fn port(&self) -> io::Result<u16> {
        self.listener.local_addr().map(|a| a.port())
    }

    /// Accepts connections until dropped; it never returns. Dropping the future closes the port and every
    /// connection it was carrying.
    pub async fn run(self, secret: Arc<str>, standin: Arc<StandIn>) {
        let Port { listener, target } = self;
        let room = Arc::new(Semaphore::new(tunnel::MAX_STREAMS));
        let mut carried = JoinSet::new();
        loop {
            let Ok(place) = room.clone().acquire_owned().await else { return };
            let tcp = match listener.accept().await {
                // (the listener is bound to loopback, so nothing else can arrive; checked all the same)
                Ok((tcp, from)) if from.ip().is_loopback() => tcp,
                Ok(_) => continue,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            while carried.try_join_next().is_some() {}
            let _ = tcp.set_nodelay(true);
            let (secret, standin) = (secret.clone(), standin.clone());
            carried.spawn(async move {
                let _place = place;
                carry(tcp, &secret, standin, target).await;
            });
        }
    }
}

/// One local connection: its first request's head, the secret in it, and then the stand-in has it: a stream to
/// home and bytes both ways, or an answer of its own while home is away.
async fn carry(mut tcp: TcpStream, secret: &str, standin: Arc<StandIn>, target: Target) {
    // (sb-link's reader, the one the stand-in and the own surface use: the head as it came, and what followed)
    let (mut head, rest) = match tokio::time::timeout(HEAD_TIMEOUT, http1::read_head_bytes(&mut tcp)).await {
        Ok(Ok(read)) => read,
        _ => return,
    };
    if !admits(&head, secret) {
        let _ = tcp.write_all(REFUSAL).await;
        let _ = tcp.shutdown().await;
        return;
    }
    if standin.away_since().is_some() {
        head = without_gate(&head);
    }
    head.extend_from_slice(&rest);
    standin.carry(target, Replay::new(head, tcp)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn head(lines: &[&str]) -> Vec<u8> {
        let mut h = String::from("GET /api/sessions HTTP/1.1\r\n");
        for l in lines {
            h.push_str(l);
            h.push_str("\r\n");
        }
        h.push_str("\r\n");
        h.into_bytes()
    }

    #[test]
    fn a_secret_is_64_hex_digits_and_new_each_time() {
        let (a, b) = (new_secret(), new_secret());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn the_header_lets_a_request_in() {
        assert!(admits(&head(&["Host: 127.0.0.1:5", &format!("X-Switchboard-Gate: {S}")]), S));
        assert!(admits(&head(&[&format!("x-switchboard-gate:{S}"), "Host: x"]), S), "any case, no space");
        assert!(admits(&head(&[&format!("X-SWITCHBOARD-GATE:   {S}  ")]), S), "spaces around the value");
        assert!(admits(&head(&["X-Switchboard-Gate: wrong", &format!("X-Switchboard-Gate: {S}")]), S));
    }

    #[test]
    fn the_secret_is_taken_out_of_a_head_that_goes_elsewhere() {
        let h = head(&["Host: 127.0.0.1:5", &format!("X-Switchboard-Gate: {S}"), "Upgrade: websocket"]);
        let out = without_gate(&h);
        assert_eq!(out, head(&["Host: 127.0.0.1:5", "Upgrade: websocket"]));
        assert_eq!(
            without_gate(&head(&[&format!("x-switchboard-gate:{S}"), &format!("X-SWITCHBOARD-GATE: {S}")])),
            head(&[])
        );
        // nothing else is touched: a head without the header, another header's name, a body that looks like one
        let plain = head(&["Host: x", &format!("X-Switchboard-Gates: {S}")]);
        assert_eq!(without_gate(&plain), plain);
        let mut with_body = head(&["Content-Length: 90"]);
        with_body.extend_from_slice(format!("X-Switchboard-Gate: {S}\r\n\r\n").as_bytes());
        assert_eq!(without_gate(&with_body), with_body);
        assert_eq!(without_gate(b""), b"");
    }

    #[tokio::test]
    async fn what_was_read_of_a_connection_is_read_again_first() {
        use tokio::io::AsyncReadExt;
        let (mut far, near) = tokio::io::duplex(64);
        let mut r = Replay::new(b"GET / HTTP/1.1\r\n\r\nab".to_vec(), near);
        far.write_all(b"cd").await.unwrap();
        let mut small = [0u8; 5];
        r.read_exact(&mut small).await.unwrap();
        assert_eq!(&small, b"GET /");
        let mut got = vec![0u8; 17];
        r.read_exact(&mut got).await.unwrap();
        assert_eq!(got, b" HTTP/1.1\r\n\r\nabcd");
        r.write_all(b"back").await.unwrap();
        let mut back = [0u8; 4];
        far.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"back");
        drop(far);
        assert_eq!(r.read(&mut small).await.unwrap(), 0);
        let mut empty = Replay::new(vec![], tokio::io::empty());
        assert_eq!(empty.read(&mut small).await.unwrap(), 0);
    }

    #[test]
    fn anything_else_is_kept_out() {
        assert!(!admits(&head(&["Host: 127.0.0.1:5"]), S), "no header");
        assert!(!admits(&head(&["X-Switchboard-Gate: "]), S));
        assert!(!admits(&head(&[&format!("X-Switchboard-Gate: {}", &S[1..])]), S), "one digit short");
        assert!(!admits(&head(&[&format!("X-Switchboard-Gate: {S}0")]), S), "one digit long");
        assert!(!admits(&head(&[&format!("X-Switchboard-Gate: {}", S.to_uppercase().replace('0', "1"))]), S));
        assert!(!admits(&head(&[&format!("X-Switchboard-Gates: {S}")]), S), "another header's name");
        assert!(!admits(&head(&[&format!("Cookie: X-Switchboard-Gate: {S}")]), S), "inside another header");
        assert!(!admits(format!("GET /?X-Switchboard-Gate: {S} HTTP/1.1\r\n\r\n").as_bytes(), S), "the request line");
        // in the body, after the blank line
        let mut h = head(&["Content-Length: 90"]);
        h.extend_from_slice(format!("X-Switchboard-Gate: {S}\r\n\r\n").as_bytes());
        assert!(!admits(&h, S));
        assert!(!admits(b"", S));
        assert!(!admits(b"\r\n\r\n", S));
    }
}
