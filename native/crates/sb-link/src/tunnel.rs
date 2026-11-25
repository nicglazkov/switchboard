//! The tunnel: one stream for each connection, with a first line that says where it goes.
//!
//! A program on a paired device connects to a loopback port (`Forwarder`). For each connection the forwarder
//! opens one stream to home, writes one line of JSON, `{"to":"api"}` or `{"to":"hub"}`, and from then on copies
//! bytes both ways. Home's side (`serve`) reads the line, connects to the local address its own settings give
//! for that name, and copies. A name it has no address for is refused: the other side never picks an address.
//!
//! Who may open a stream at all is decided before this (the paired list). What is here is the same for any
//! stream that is `AsyncRead + AsyncWrite`.
//!
//! Copying (`pipe`) keeps the two directions apart. When one side has sent everything, the other side's write
//! half is closed and the answer still flows back, as HTTP needs. Three clocks stop one side from holding the
//! other for good: a write that makes no progress, a side that stays silent after the other has finished, and
//! (when asked for) no bytes at all in either direction.

use serde::Deserialize;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinSet;

/// The longest first line read, with its newline.
pub const HEADER_MAX: usize = 64;
/// How long the first line may take to arrive, and to be written.
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(5);
/// How long home waits for its own local service to take the connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a forwarder waits for a stream to home before it gives the local connection up.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections one forwarder carries at once; more wait to be accepted.
pub const MAX_STREAMS: usize = 256;
const BUF: usize = 16 * 1024;

/// Where a stream goes at home.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// The server's HTTP API.
    Api,
    /// The hub's listener for apps and nodes.
    Hub,
    /// This computer's own surface: its own chats, answered by its helper and never by an address (phase 4).
    Own,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Api => "api",
            Target::Hub => "hub",
            Target::Own => "own",
        }
    }

    fn named(name: &str) -> Option<Target> {
        match name {
            "api" => Some(Target::Api),
            "hub" => Some(Target::Hub),
            "own" => Some(Target::Own),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct Header<'a> {
    #[serde(borrow)]
    to: std::borrow::Cow<'a, str>,
}

#[derive(Debug)]
pub enum TunnelError {
    /// The first line is too long, not JSON, or has no `to`.
    Header(&'static str),
    /// A target this side does not serve: one it does not know, or has no address for.
    Refused(String),
    /// Which step ran out of time.
    Timeout(&'static str),
    Io(io::Error),
}

impl fmt::Display for TunnelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TunnelError::Header(what) => write!(f, "a stream's first line was not usable: {what}"),
            TunnelError::Refused(to) => write!(f, "a stream asked for {to:?}, which is not served here"),
            TunnelError::Timeout(what) => write!(f, "timed out: {what}"),
            TunnelError::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for TunnelError {}

impl From<io::Error> for TunnelError {
    fn from(e: io::Error) -> TunnelError {
        TunnelError::Io(e)
    }
}

/// The first line of a stream to `target`, with its newline.
pub fn encode_header(target: Target) -> Vec<u8> {
    format!("{{\"to\":\"{}\"}}\n", target.name()).into_bytes()
}

/// Reads a first line (without its newline). A target this version does not know is `Refused`, by name; fields
/// it does not know are skipped, so a later version can add some.
pub fn parse_header(line: &[u8]) -> Result<Target, TunnelError> {
    if line.len() >= HEADER_MAX {
        return Err(TunnelError::Header("too long"));
    }
    let header = serde_json::from_slice::<Header>(line).map_err(|_| TunnelError::Header("not the expected JSON"))?;
    Target::named(&header.to).ok_or_else(|| TunnelError::Refused(crate::clean(&header.to, 32)))
}

pub async fn write_header<W: AsyncWrite + Unpin>(w: &mut W, target: Target) -> io::Result<()> {
    w.write_all(&encode_header(target)).await?;
    w.flush().await
}

/// Reads the first line off a stream, and not one byte past its newline: what follows belongs to the target.
/// Gives up after `HEADER_TIMEOUT` or `HEADER_MAX` bytes.
pub async fn read_header<R: AsyncRead + Unpin>(r: &mut R) -> Result<Target, TunnelError> {
    let line = async {
        let mut line = Vec::with_capacity(HEADER_MAX);
        loop {
            let mut byte = [0u8; 1];
            if r.read(&mut byte).await? == 0 {
                return Err(TunnelError::Header("the stream ended before the line did"));
            }
            if byte[0] == b'\n' {
                return Ok(line);
            }
            line.push(byte[0]);
            if line.len() >= HEADER_MAX {
                return Err(TunnelError::Header("too long"));
            }
        }
    };
    let line =
        tokio::time::timeout(HEADER_TIMEOUT, line).await.map_err(|_| TunnelError::Timeout("the first line"))??;
    parse_header(&line)
}

/// The local addresses home's side connects to. A target with no address is refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Targets {
    pub api: Option<SocketAddr>,
    pub hub: Option<SocketAddr>,
}

impl Targets {
    pub fn addr(&self, target: Target) -> Option<SocketAddr> {
        match target {
            Target::Api => self.api,
            Target::Hub => self.hub,
            // never an address: whoever serves `own` takes the stream itself (endpoint.rs `OwnIncoming`)
            Target::Own => None,
        }
    }
}

/// The clocks `pipe` runs by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeOptions {
    /// A side that takes nothing for this long while there is something to give it has stalled: both sides are
    /// closed.
    pub write_stall: Duration,
    /// Once one side has finished sending, the other may stay silent this long before both are closed. Bytes
    /// arriving start the clock again, so a long answer is not cut.
    pub half_closed_idle: Duration,
    /// Close both sides when no byte has moved either way for this long. None: never, which is right for a
    /// terminal or an event stream that may sit idle for hours.
    pub idle: Option<Duration>,
}

impl Default for PipeOptions {
    fn default() -> PipeOptions {
        PipeOptions { write_stall: Duration::from_secs(60), half_closed_idle: Duration::from_secs(300), idle: None }
    }
}

/// One direction's "I have sent everything", which the other direction waits on.
#[derive(Default)]
struct Done {
    flag: AtomicBool,
    wake: Notify,
}

fn stalled(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, what)
}

/// Copies `a` to `b` and `b` to `a` until both directions have ended, and returns the bytes moved each way.
/// The end of one direction closes the write half of the other side and leaves the other direction running.
/// Err when a side fails or one of the clocks in `opts` runs out; either way both sides are dropped, and so
/// closed, when this returns.
pub async fn pipe<A, B>(a: A, b: B, opts: &PipeOptions) -> io::Result<(u64, u64)>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (mut ar, mut aw) = tokio::io::split(a);
    let (mut br, mut bw) = tokio::io::split(b);
    let (a_done, b_done) = (Done::default(), Done::default());
    let moved = AtomicU64::new(0); // reads in either direction, for the idle clock
    // (try_join: the first error drops the other direction where it stands)
    tokio::try_join!(
        one_way(&mut ar, &mut bw, opts, &a_done, &b_done, &moved),
        one_way(&mut br, &mut aw, opts, &b_done, &a_done, &moved),
    )
}

async fn one_way<R, W>(
    r: &mut R,
    w: &mut W,
    opts: &PipeOptions,
    mine: &Done,
    other: &Done,
    moved: &AtomicU64,
) -> io::Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; BUF];
    let mut total = 0u64;
    loop {
        let n = read_some(r, &mut buf, opts, other, moved).await?;
        if n == 0 {
            break;
        }
        let write = async {
            w.write_all(&buf[..n]).await?;
            w.flush().await
        };
        tokio::time::timeout(opts.write_stall, write)
            .await
            .map_err(|_| stalled("the other side stopped reading"))??;
        total += n as u64;
    }
    mine.flag.store(true, Ordering::Release);
    mine.wake.notify_one();
    // tell the far side there is no more; if it is not listening, the other direction will find out
    let _ = tokio::time::timeout(opts.write_stall, w.shutdown()).await;
    Ok(total)
}

/// The next bytes from `r`, under whichever clock applies: none, the idle one, or, once the other direction has
/// finished, the half-closed one.
async fn read_some<R: AsyncRead + Unpin>(
    r: &mut R,
    buf: &mut [u8],
    opts: &PipeOptions,
    other: &Done,
    moved: &AtomicU64,
) -> io::Result<usize> {
    loop {
        let other_done = other.flag.load(Ordering::Acquire);
        let before = moved.load(Ordering::Relaxed);
        let limit = if other_done { Some(opts.half_closed_idle) } else { opts.idle };
        let clock = async {
            match limit {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            n = r.read(buf) => {
                moved.fetch_add(1, Ordering::Relaxed);
                return n;
            }
            () = clock => {
                // the idle clock is for both directions together: bytes the other way start it again
                if !other_done && moved.load(Ordering::Relaxed) != before {
                    continue;
                }
                return Err(stalled(if other_done { "silent after the other side finished" } else { "idle" }));
            }
            // the other direction just finished: start over under the half-closed clock
            () = other.wake.notified(), if !other_done => {}
        }
    }
}

/// Home's side of one stream: reads where it goes, connects there, and copies until both directions end.
/// Returns the target and the bytes moved (from the stream, to the stream).
///
/// Whatever goes wrong, the stream is dropped, which the far side sees as its connection closing.
pub async fn serve<S>(mut stream: S, targets: &Targets, opts: &PipeOptions) -> Result<(Target, u64, u64), TunnelError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let target = read_header(&mut stream).await?;
    serve_to(stream, target, targets, opts).await
}

/// `serve` for a stream whose first line has been read already.
pub async fn serve_to<S>(
    stream: S,
    target: Target,
    targets: &Targets,
    opts: &PipeOptions,
) -> Result<(Target, u64, u64), TunnelError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let addr = targets.addr(target).ok_or_else(|| TunnelError::Refused(target.name().into()))?;
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| TunnelError::Timeout("connecting to the local service"))??;
    let _ = tcp.set_nodelay(true);
    let (up, down) = pipe(stream, tcp, opts).await?;
    Ok((target, up, down))
}

/// A loopback port whose connections each become one stream to `target` at home.
pub struct Forwarder {
    listener: TcpListener,
    target: Target,
    opts: PipeOptions,
    max: usize,
}

impl Forwarder {
    /// Listens on `addr`, which must be a loopback address: the port is this computer's door to home, and it
    /// is not for the network.
    pub async fn bind(addr: SocketAddr, target: Target) -> io::Result<Forwarder> {
        if !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("a forwarder listens on loopback only, not {}", addr.ip()),
            ));
        }
        let listener = TcpListener::bind(addr).await?;
        Ok(Forwarder { listener, target, opts: PipeOptions::default(), max: MAX_STREAMS })
    }

    pub fn options(mut self, opts: PipeOptions) -> Forwarder {
        self.opts = opts;
        self
    }

    /// How many connections are carried at once (default `MAX_STREAMS`). At least one.
    pub fn max_streams(mut self, max: usize) -> Forwarder {
        self.max = max.max(1);
        self
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn target(&self) -> Target {
        self.target
    }

    /// Accepts connections until dropped; it never returns. Each connection calls `open_stream` for a stream
    /// to home, writes the first line and copies. A connection whose stream cannot be opened is closed, and the
    /// program that made it tries again as it would with any server that is away.
    ///
    /// Dropping the future closes the port and every connection it was carrying.
    pub async fn run<F, Fut, S>(self, open_stream: F)
    where
        F: Fn() -> Fut,
        Fut: Future<Output = io::Result<S>> + Send + 'static,
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let Forwarder { listener, target, opts, max } = self;
        let opts = Arc::new(opts);
        let room = Arc::new(Semaphore::new(max));
        let mut carried = JoinSet::new();
        loop {
            let Ok(place) = room.clone().acquire_owned().await else { return };
            let tcp = match listener.accept().await {
                Ok((tcp, _)) => tcp,
                Err(_) => {
                    // out of file handles, most likely: let some connections end
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            while carried.try_join_next().is_some() {}
            let _ = tcp.set_nodelay(true);
            let (open, opts) = (open_stream(), opts.clone());
            carried.spawn(async move {
                let _place = place;
                let _ = forward(tcp, open, target, &opts).await;
            });
        }
    }
}

/// One local connection: a stream, its first line, then bytes both ways.
async fn forward<Fut, S>(
    tcp: TcpStream,
    open: Fut,
    target: Target,
    opts: &PipeOptions,
) -> Result<(u64, u64), TunnelError>
where
    Fut: Future<Output = io::Result<S>>,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut stream =
        tokio::time::timeout(OPEN_TIMEOUT, open).await.map_err(|_| TunnelError::Timeout("opening a stream"))??;
    tokio::time::timeout(HEADER_TIMEOUT, write_header(&mut stream, target))
        .await
        .map_err(|_| TunnelError::Timeout("writing the first line"))??;
    pipe(tcp, stream, opts).await.map_err(TunnelError::Io)
}

#[cfg(test)]
mod tests;
