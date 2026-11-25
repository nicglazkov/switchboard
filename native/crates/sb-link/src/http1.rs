//! Just enough HTTP/1.1 to ask one question on a stream and read the answer, and to read the head of a request.
//!
//! The own surface (a computer's answers about its own chats) is HTTP on a tunnel stream, one request per
//! stream. A device that asks is this file's `ask`. The stand-in that answers on the loopback ports while the
//! home computer is away reads each request with `read_head` and `read_body`. Nothing here keeps a connection
//! open for a second request: every answer says `Connection: close`.

use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The most a request's or an answer's head may be. (The phones' gate reads its first request's head with this
/// file too, and that head carries whatever the app's HTTP client adds.)
pub const HEAD_MAX: usize = 32 * 1024;

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

/// The first line and the headers of a request or an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The three parts of the first line: `GET`, `/path?query`, `HTTP/1.1` for a request; `HTTP/1.1`, `200`,
    /// `OK` for an answer.
    pub first: [String; 3],
    /// Names in lowercase.
    pub headers: Vec<(String, String)>,
    /// The head exactly as it came, with the empty line that ends it.
    pub raw: Vec<u8>,
    /// What was read past the head: the start of the body.
    pub rest: Vec<u8>,
}

impl Head {
    pub fn method(&self) -> &str {
        &self.first[0]
    }

    /// A request's target, `/path?query`.
    pub fn target(&self) -> &str {
        &self.first[1]
    }

    pub fn path(&self) -> &str {
        self.target().split('?').next().unwrap_or("")
    }

    /// The first value of `key` in the query, as it was sent (nothing is decoded).
    pub fn query(&self, key: &str) -> Option<&str> {
        let q = self.target().split_once('?')?.1;
        q.split('&').filter_map(|p| p.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// An answer's status.
    pub fn status(&self) -> u16 {
        self.first[1].parse().unwrap_or(0)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Content-Length. None when there is none, it is not a number, or the body is sent in chunks.
    pub fn length(&self) -> Option<usize> {
        if self.header("transfer-encoding").is_some() {
            return None;
        }
        self.header("content-length")?.trim().parse().ok()
    }
}

type Headers = Vec<(String, String)>;

fn parse(raw: &[u8]) -> io::Result<([String; 3], Headers)> {
    let text = std::str::from_utf8(raw).map_err(|_| bad("the head is not text"))?;
    let mut lines = text.split("\r\n");
    let mut parts = lines.next().unwrap_or("").splitn(3, ' ');
    let first = [
        parts.next().unwrap_or("").to_string(),
        parts.next().unwrap_or("").to_string(),
        parts.next().unwrap_or("").to_string(),
    ];
    if first[0].is_empty() || first[1].is_empty() {
        return Err(bad("the first line is not one"));
    }
    let mut headers = vec![];
    for line in lines.filter(|l| !l.is_empty()) {
        let (k, v) = line.split_once(':').ok_or_else(|| bad("a header has no colon"))?;
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    Ok((first, headers))
}

/// Reads up to the empty line that ends a head, and parses nothing: the head exactly as it came (with its empty
/// line), and what was read past it. Fails when the stream ends first, or the head is longer than `HEAD_MAX`.
/// For a reader that only looks for one header and passes the bytes on (the phones' gate).
pub async fn read_head_bytes<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut got = Vec::with_capacity(1024);
    let mut buf = [0u8; 4096];
    loop {
        // (the search starts a little before the new bytes: the empty line may straddle two reads)
        let from = got.len().saturating_sub(3);
        let n = r.read(&mut buf).await?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the connection closed before the head ended"));
        }
        got.extend_from_slice(&buf[..n]);
        if let Some(i) = got[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = got.split_off(from + i + 4);
            return Ok((got, rest));
        }
        if got.len() > HEAD_MAX {
            return Err(bad("the head is too long"));
        }
    }
}

/// Reads up to the empty line that ends a head. Fails when the stream ends first, the head is longer than
/// `HEAD_MAX`, or it is not a first line and headers.
pub async fn read_head<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Head> {
    let (raw, rest) = read_head_bytes(r).await?;
    let (first, headers) = parse(&raw[..raw.len() - 4])?;
    Ok(Head { first, headers, raw, rest })
}

/// The body that follows `head`: `length` bytes when it says how many (refused past `max`), else everything up
/// to the end of the stream (cut off, as an error, past `max`).
pub async fn read_body<R: AsyncRead + Unpin>(r: &mut R, head: &Head, max: usize) -> io::Result<Vec<u8>> {
    let mut body = head.rest.clone();
    match head.length() {
        Some(n) if n > max => Err(bad("the body is too large")),
        Some(n) => {
            body.truncate(n);
            let had = body.len();
            body.resize(n, 0);
            r.read_exact(&mut body[had..]).await?;
            Ok(body)
        }
        None => {
            let mut buf = [0u8; 4096];
            loop {
                if body.len() > max {
                    return Err(bad("the body is too large"));
                }
                let n = r.read(&mut buf).await?;
                if n == 0 {
                    return Ok(body);
                }
                body.extend_from_slice(&buf[..n]);
            }
        }
    }
}

/// A request's body when it says how long it is; nothing when it names no length (a GET).
pub async fn read_request_body<R: AsyncRead + Unpin>(r: &mut R, head: &Head, max: usize) -> io::Result<Vec<u8>> {
    match head.length() {
        Some(_) => read_body(r, head, max).await,
        None if head.header("transfer-encoding").is_some() => Err(bad("a body sent in chunks is not read")),
        None => Ok(vec![]),
    }
}

/// An answer to `ask`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// The bytes of a request with a JSON body (or none).
pub fn request(method: &str, target: &str, body: Option<&[u8]>) -> Vec<u8> {
    let mut out = format!("{method} {target} HTTP/1.1\r\nHost: own\r\nConnection: close\r\n").into_bytes();
    if let Some(b) = body {
        out.extend_from_slice(format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()).as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body.unwrap_or(b""));
    out
}

/// One request on `stream`, and its answer (at most `max` bytes of body). The stream is used up.
pub async fn ask<S>(stream: &mut S, method: &str, target: &str, body: Option<&[u8]>, max: usize) -> io::Result<Answer>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    stream.write_all(&request(method, target, body)).await?;
    stream.flush().await?;
    let head = read_head(stream).await?;
    if !head.first[0].starts_with("HTTP/1.") {
        return Err(bad("the answer is not HTTP"));
    }
    let body = read_body(stream, &head, max).await?;
    Ok(Answer { status: head.status(), body })
}

/// The bytes of an answer with a JSON body, which ends the connection.
pub fn answer(status: u16, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "",
    };
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: \
         no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_request_head_is_read_whole_however_it_arrives() {
        let text =
            b"GET /term?t=abc&own=laptop&cols=80 HTTP/1.1\r\nHost: 127.0.0.1:8132\r\nUpgrade: websocket\r\n\r\nXY";
        for chunk in [1usize, 2, 3, 7, 4096] {
            let (mut a, mut b) = tokio::io::duplex(8);
            let send = tokio::spawn(async move {
                for part in text.chunks(chunk) {
                    a.write_all(part).await.unwrap();
                }
                a
            });
            let head = read_head(&mut b).await.unwrap();
            assert_eq!((head.method(), head.path()), ("GET", "/term"));
            assert_eq!((head.query("own"), head.query("t"), head.query("rows")), (Some("laptop"), Some("abc"), None));
            assert_eq!(head.header("upgrade"), Some("websocket"));
            assert_eq!(head.raw, &text[..text.len() - 2], "the head as it came");
            let _a = send.await.unwrap();
            let mut all = head.rest.clone();
            let mut buf = [0u8; 8];
            while all.len() < 2 {
                let n = b.read(&mut buf).await.unwrap();
                all.extend_from_slice(&buf[..n]);
            }
            assert_eq!(all, b"XY", "what follows the head is not lost");
        }
    }

    #[tokio::test]
    async fn a_head_that_never_ends_or_is_not_one_is_refused() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        a.write_all(&vec![b'a'; HEAD_MAX + 4096]).await.unwrap();
        assert!(read_head(&mut b).await.is_err());
        let (mut a, mut b) = tokio::io::duplex(1024);
        a.write_all(b"GET /x HTTP/1.1\r\nHost").await.unwrap();
        drop(a);
        assert_eq!(read_head(&mut b).await.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        let (mut a, mut b) = tokio::io::duplex(1024);
        a.write_all(b"\r\n\r\n").await.unwrap();
        assert!(read_head(&mut b).await.is_err());
        let (mut a, mut b) = tokio::io::duplex(1024);
        a.write_all(b"GET / HTTP/1.1\r\nno colon here\r\n\r\n").await.unwrap();
        assert!(read_head(&mut b).await.is_err());
    }

    #[tokio::test]
    async fn the_bytes_of_a_head_are_read_without_parsing_them() {
        // a head no parser here would take (a header with no colon, bytes that are not text) is still a head
        let text = b"GET / HTTP/1.1\r\nno colon here\r\nX: \xff\r\n\r\nrest";
        for chunk in [1usize, 3, 4096] {
            let (mut a, mut b) = tokio::io::duplex(8);
            let send = tokio::spawn(async move {
                for part in text.chunks(chunk) {
                    a.write_all(part).await.unwrap();
                }
                a
            });
            let (raw, mut rest) = read_head_bytes(&mut b).await.unwrap();
            assert_eq!(raw, &text[..text.len() - 4]);
            let _a = send.await.unwrap();
            let mut buf = [0u8; 8];
            while rest.len() < 4 {
                let n = b.read(&mut buf).await.unwrap();
                rest.extend_from_slice(&buf[..n]);
            }
            assert_eq!(rest, b"rest");
        }
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        a.write_all(&vec![b'a'; HEAD_MAX + 4096]).await.unwrap();
        assert_eq!(read_head_bytes(&mut b).await.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn one_question_and_its_answer() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let server = tokio::spawn(async move {
            let head = read_head(&mut b).await.unwrap();
            let body = read_request_body(&mut b, &head, 1024).await.unwrap();
            assert_eq!((head.method(), head.target(), body.as_slice()), ("POST", "/own/v1/action", &b"{\"a\":1}"[..]));
            assert_eq!(head.header("content-type"), Some("application/json"));
            b.write_all(&answer(404, b"{\"ok\":false}")).await.unwrap();
        });
        let got = ask(&mut a, "POST", "/own/v1/action", Some(b"{\"a\":1}"), 1024).await.unwrap();
        assert_eq!((got.status, got.json().unwrap()["ok"].as_bool()), (404, Some(false)));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_body_past_the_limit_is_refused_and_one_with_no_length_ends_with_the_stream() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        b.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\n\r\n").await.unwrap();
        assert!(ask(&mut a, "GET", "/x", None, 100).await.is_err());
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        b.write_all(b"HTTP/1.1 200 OK\r\n\r\nhello").await.unwrap();
        b.shutdown().await.unwrap();
        let got = ask(&mut a, "GET", "/x", None, 100).await.unwrap();
        assert_eq!(got.body, b"hello");
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        b.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        b.write_all(&[b'x'; 500]).await.unwrap();
        b.shutdown().await.unwrap();
        assert!(ask(&mut a, "GET", "/x", None, 100).await.is_err());
        drop(b);
    }
}
