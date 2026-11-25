use super::*;
use crate::testing::Dice;
use futures_util::{SinkExt, StreamExt};
use tokio::io::duplex;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::Message;

const LONG: Duration = Duration::from_secs(20);

/// Clocks short enough to test and long enough for a busy build machine.
fn quick() -> PipeOptions {
    PipeOptions { write_stall: Duration::from_millis(400), half_closed_idle: Duration::from_millis(400), idle: None }
}

async fn listener() -> (TcpListener, SocketAddr) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    (l, addr)
}

/// Both ends of a tunnel in this process: a forwarder on a free loopback port whose every stream is a pipe to
/// `serve` with `targets`. Returns the port to connect to and the forwarder's task.
async fn tunnel_with(f: Forwarder, targets: Targets, opts: PipeOptions) -> (SocketAddr, JoinHandle<()>) {
    let addr = f.local_addr().unwrap();
    let serve_opts = opts.clone();
    let task = tokio::spawn(f.options(opts).run(move || {
        let (near, far) = duplex(64 * 1024);
        let (targets, opts) = (targets.clone(), serve_opts.clone());
        tokio::spawn(async move {
            let _ = serve(far, &targets, &opts).await;
        });
        async move { Ok(near) }
    }));
    (addr, task)
}

async fn tunnel(target: Target, targets: Targets, opts: PipeOptions) -> (SocketAddr, JoinHandle<()>) {
    let f = Forwarder::bind("127.0.0.1:0".parse().unwrap(), target).await.unwrap();
    tunnel_with(f, targets, opts).await
}

/// The connection is over, cleanly or not, before the test gives up on it.
async fn closed(c: &mut TcpStream) -> bool {
    let mut buf = [0u8; 1024];
    loop {
        match timeout(LONG, c.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) => return true,
            Ok(Ok(_)) => {}
            Err(_) => return false,
        }
    }
}

#[test]
fn the_first_line() {
    assert_eq!(encode_header(Target::Api), b"{\"to\":\"api\"}\n");
    assert_eq!(encode_header(Target::Hub), b"{\"to\":\"hub\"}\n");
    for t in [Target::Api, Target::Hub] {
        let line = encode_header(t);
        assert!(matches!(parse_header(&line[..line.len() - 1]), Ok(got) if got == t));
    }
    let oks: &[&[u8]] = &[
        br#"{ "to" : "hub" }"#,
        br#"{"to":"hub","v":2,"x":[1,{"y":null}]}"#,
        b"{\"to\":\"\\u0068ub\"}",
        b"{\"to\":\"hub\"}\r",
    ];
    for ok in oks {
        assert!(matches!(parse_header(ok), Ok(Target::Hub)), "{}", String::from_utf8_lossy(ok));
    }
    let refused: &[(&[u8], &str)] = &[
        (br#"{"to":"ssh"}"#, "ssh"),
        (br#"{"to":""}"#, ""),
        (br#"{"to":"API"}"#, "API"),
        (br#"{"to":"api "}"#, "api"),
        (b"{\"to\":\"a\\u0007pi\"}", "api"),
    ];
    for (line, want) in refused {
        match parse_header(line) {
            Err(TunnelError::Refused(name)) => assert_eq!(name, *want),
            other => panic!("{}: {other:?}", String::from_utf8_lossy(line)),
        }
    }
    let long = format!(r#"{{"to":"api","pad":"{}"}}"#, "x".repeat(HEADER_MAX));
    let bads: &[&[u8]] = &[
        b"",
        b"api",
        b"{}",
        b"[]",
        b"null",
        br#"{"to":1}"#,
        br#"{"to":null}"#,
        br#"{"to":"api""#,
        br#"{"to":"api"} x"#,
        br#"{"TO":"api"}"#,
        b"\xff\xfe",
        long.as_bytes(),
    ];
    for bad in bads {
        assert!(matches!(parse_header(bad), Err(TunnelError::Header(_))), "{}", String::from_utf8_lossy(bad));
    }
    // exactly at the limit: 63 bytes and a newline is the longest line there is
    let pad = |n: usize| format!(r#"{{"to":"api","p":"{}"}}"#, "x".repeat(n - 19));
    assert_eq!(pad(63).len(), 63);
    assert!(matches!(parse_header(pad(63).as_bytes()), Ok(Target::Api)));
    assert!(matches!(parse_header(pad(64).as_bytes()), Err(TunnelError::Header(_))));
}

#[tokio::test]
async fn reading_the_first_line_takes_nothing_after_it() {
    let (mut a, mut b) = duplex(1024);
    a.write_all(b"{\"to\":\"api\"}\nGET / HTTP/1.1\r\n").await.unwrap();
    assert!(matches!(read_header(&mut b).await, Ok(Target::Api)));
    let mut rest = [0u8; 16];
    b.read_exact(&mut rest).await.unwrap();
    assert_eq!(&rest, b"GET / HTTP/1.1\r\n");

    // a line that never ends is given up on at the limit, not read for ever
    let (mut a, mut b) = duplex(1024);
    a.write_all(&[b'a'; 500]).await.unwrap();
    assert!(matches!(read_header(&mut b).await, Err(TunnelError::Header("too long"))));
    let mut left = vec![];
    drop(a);
    b.read_to_end(&mut left).await.unwrap();
    assert_eq!(left.len(), 500 - HEADER_MAX);

    let (mut a, mut b) = duplex(1024);
    a.write_all(b"{\"to\":\"api\"}").await.unwrap();
    drop(a);
    assert!(matches!(read_header(&mut b).await, Err(TunnelError::Header(_))));

    let (mut a, mut b) = duplex(1024);
    write_header(&mut a, Target::Hub).await.unwrap();
    assert!(matches!(read_header(&mut b).await, Ok(Target::Hub)));
}

#[tokio::test(start_paused = true)]
async fn a_first_line_that_does_not_come_is_given_up_on() {
    let (mut a, mut b) = duplex(1024);
    a.write_all(b"{\"to\":").await.unwrap();
    let started = Instant::now();
    assert!(matches!(read_header(&mut b).await, Err(TunnelError::Timeout(_))));
    assert!(started.elapsed() >= HEADER_TIMEOUT && started.elapsed() < HEADER_TIMEOUT * 2);
    // and `serve` with it, without having connected anywhere
    let (_held, far) = duplex(1024);
    let r = serve(far, &Targets::default(), &PipeOptions::default()).await;
    assert!(matches!(r, Err(TunnelError::Timeout(_))), "{r:?}");
}

#[tokio::test]
async fn random_first_lines_never_panic() {
    let mut dice = Dice(0x7a11_0001);
    for round in 0..5000 {
        let mut noise = dice.bytes(90);
        if round % 2 == 0 {
            // noise shaped like the real thing
            let at = dice.below(noise.len() + 1);
            drop(noise.splice(at..at, br#"{"to":""#.iter().copied()));
        }
        let _ = parse_header(&noise);
        let (mut a, mut b) = duplex(256);
        a.write_all(&noise).await.unwrap();
        drop(a);
        match read_header(&mut b).await {
            Ok(_) | Err(TunnelError::Header(_)) | Err(TunnelError::Refused(_)) => {}
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn a_target_that_is_not_served_is_refused() {
    let (api, api_addr) = listener().await;
    let targets = Targets { api: Some(api_addr), hub: None };
    let lines: &[(&[u8], &str)] = &[
        (b"{\"to\":\"ssh\"}\n", "ssh"),
        (b"{\"to\":\"hub\"}\n", "hub"),
        (b"{\"to\":\"127.0.0.1:22\"}\n", "127.0.0.1:22"),
    ];
    for (line, want) in lines {
        let (mut near, far) = duplex(1024);
        near.write_all(line).await.unwrap();
        near.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
        match serve(far, &targets, &quick()).await {
            Err(TunnelError::Refused(name)) => assert_eq!(name, *want),
            other => panic!("{other:?}"),
        }
        // the far side is gone, which the near side sees as the end
        let mut rest = vec![];
        near.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
    }
    assert!(timeout(Duration::from_millis(200), api.accept()).await.is_err(), "nothing was connected to");

    // through a forwarder: the program that connected just sees the connection close
    let (addr, _task) = tunnel(Target::Hub, targets, quick()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    assert!(closed(&mut c).await);
    assert!(timeout(Duration::from_millis(200), api.accept()).await.is_err());
}

#[tokio::test]
async fn an_http_request_goes_through_and_its_answer_comes_back() {
    let (server, server_addr) = listener().await;
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut s, _) = server.accept().await.unwrap();
        let mut got = vec![];
        let mut buf = [0u8; 4096];
        // the headers, then the 11 bytes of body they announce
        while !got.windows(4).any(|w| w == b"\r\n\r\n") || !got.ends_with(b"{\"a\":\"b c\"}") {
            let n = s.read(&mut buf).await.unwrap();
            assert!(n > 0, "the request was cut: {:?}", String::from_utf8_lossy(&got));
            got.extend_from_slice(&buf[..n]);
        }
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").await.unwrap();
        seen_tx.send(got).unwrap();
    });
    let (addr, _task) =
        tunnel(Target::Api, Targets { api: Some(server_addr), hub: None }, PipeOptions::default()).await;
    let request = b"POST /api/event HTTP/1.1\r\nHost: 127.0.0.1:8130\r\nX-Switchboard: 1\r\nContent-Length: 11\r\n\r\n{\"a\":\"b c\"}";
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(request).await.unwrap();
    let mut answer = vec![];
    timeout(LONG, c.read_to_end(&mut answer)).await.unwrap().unwrap();
    assert_eq!(answer, b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
    assert_eq!(seen_rx.await.unwrap(), request, "the server gets the request and none of the first line");
}

#[tokio::test]
async fn a_websocket_echoes_through_the_tunnel() {
    let (server, server_addr) = listener().await;
    tokio::spawn(async move {
        let (s, _) = server.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
        while let Some(Ok(msg)) = ws.next().await {
            if msg.is_text() || msg.is_binary() {
                ws.send(msg).await.unwrap();
            }
        }
    });
    let (addr, _task) =
        tunnel(Target::Hub, Targets { api: None, hub: Some(server_addr) }, PipeOptions::default()).await;
    let tcp = TcpStream::connect(addr).await.unwrap();
    let (mut ws, response) = tokio_tungstenite::client_async("ws://127.0.0.1:8132/term?t=ticket", tcp).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);

    ws.send(Message::Text("typed".into())).await.unwrap();
    let got = timeout(LONG, ws.next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(&got, Message::Text(t) if t.as_str() == "typed"), "{got:?}");

    // more than any one buffer on the way holds
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    ws.send(Message::Binary(big.clone().into())).await.unwrap();
    let got = timeout(LONG, ws.next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(&got, Message::Binary(b) if b[..] == big[..]));

    ws.close(None).await.unwrap();
    let end = timeout(LONG, ws.next()).await.unwrap();
    assert!(matches!(end, Some(Ok(Message::Close(_))) | None), "{end:?}");
}

#[tokio::test]
async fn a_side_that_finishes_sending_still_gets_its_answer() {
    // the program sends everything and closes its sending half; the server answers only after it has seen that
    let (server, server_addr) = listener().await;
    tokio::spawn(async move {
        loop {
            let (mut s, _) = server.accept().await.unwrap();
            tokio::spawn(async move {
                let mut all = vec![];
                s.read_to_end(&mut all).await.unwrap();
                let sum: u64 = all.iter().map(|b| u64::from(*b)).sum();
                s.write_all(format!("{} bytes, sum {sum}", all.len()).as_bytes()).await.unwrap();
            });
        }
    });
    let (addr, _task) =
        tunnel(Target::Api, Targets { api: Some(server_addr), hub: None }, PipeOptions::default()).await;
    let body: Vec<u8> = (0..1_000_000u32).map(|i| (i % 253) as u8).collect();
    let sum: u64 = body.iter().map(|b| u64::from(*b)).sum();
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(&body).await.unwrap();
    c.shutdown().await.unwrap();
    let mut answer = String::new();
    timeout(LONG, c.read_to_string(&mut answer)).await.unwrap().unwrap();
    assert_eq!(answer, format!("1000000 bytes, sum {sum}"));

    // nothing sent at all, only the half-close
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.shutdown().await.unwrap();
    let mut answer = String::new();
    timeout(LONG, c.read_to_string(&mut answer)).await.unwrap().unwrap();
    assert_eq!(answer, "0 bytes, sum 0");
}

#[tokio::test]
async fn a_server_that_finishes_sending_still_gets_the_rest() {
    // the other way round: the server says its piece and closes its sending half, then reads to the end
    let (server, server_addr) = listener().await;
    let (count_tx, count_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut s, _) = server.accept().await.unwrap();
        s.write_all(b"greeting").await.unwrap();
        s.shutdown().await.unwrap();
        let mut all = vec![];
        s.read_to_end(&mut all).await.unwrap();
        count_tx.send(all.len()).unwrap();
    });
    let (addr, _task) =
        tunnel(Target::Api, Targets { api: Some(server_addr), hub: None }, PipeOptions::default()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    let mut greeting = vec![];
    timeout(LONG, c.read_to_end(&mut greeting)).await.unwrap().unwrap();
    assert_eq!(greeting, b"greeting");
    c.write_all(&vec![7u8; 200_000]).await.unwrap();
    drop(c);
    assert_eq!(timeout(LONG, count_rx).await.unwrap().unwrap(), 200_000);
}

#[tokio::test]
async fn a_server_that_stops_reading_does_not_hold_the_other_side() {
    let (server, server_addr) = listener().await;
    tokio::spawn(async move {
        let mut held = vec![];
        loop {
            // takes the connection and never reads a byte of it
            held.push(server.accept().await.unwrap());
        }
    });
    let (addr, _task) = tunnel(Target::Api, Targets { api: Some(server_addr), hub: None }, quick()).await;
    let c = TcpStream::connect(addr).await.unwrap();
    let (mut r, mut w) = c.into_split();
    let writer = tokio::spawn(async move {
        let chunk = vec![0u8; 64 * 1024];
        let mut sent = 0u64;
        // until the tunnel gives the connection up (a limit far above any buffer on the way, in case it does not)
        while sent < 1 << 30 && w.write_all(&chunk).await.is_ok() {
            sent += chunk.len() as u64;
        }
        sent
    });
    let mut buf = [0u8; 16];
    let end = timeout(LONG, r.read(&mut buf)).await.expect("the stalled server held the program's connection");
    assert!(matches!(end, Ok(0) | Err(_)), "{end:?}");
    let sent = timeout(LONG, writer).await.expect("the writer was left hanging").unwrap();
    assert!(sent < 1 << 30, "the write was refused once the tunnel closed");
}

#[tokio::test]
async fn a_server_that_goes_silent_after_the_request_ended_does_not_hold_the_other_side() {
    let (server, server_addr) = listener().await;
    tokio::spawn(async move {
        let mut held = vec![];
        loop {
            let (mut s, _) = server.accept().await.unwrap();
            let mut all = vec![];
            s.read_to_end(&mut all).await.unwrap();
            held.push(s); // read everything, answers nothing, never closes
        }
    });
    let (addr, _task) = tunnel(Target::Api, Targets { api: Some(server_addr), hub: None }, quick()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"request").await.unwrap();
    c.shutdown().await.unwrap();
    let started = Instant::now();
    assert!(closed(&mut c).await, "the silent server held the program's connection");
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "closed by the clock, not at once: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn the_copy_itself() {
    // both directions, each ending on its own
    let (mut a, a_far) = duplex(64);
    let (b_far, mut b) = duplex(64);
    let opts = PipeOptions::default();
    let copy = pipe(a_far, b_far, &opts);
    let ends = async {
        a.write_all(b"ping ping ping").await.unwrap();
        a.shutdown().await.unwrap();
        let mut got = vec![];
        b.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"ping ping ping");
        b.write_all(b"pong").await.unwrap();
        b.shutdown().await.unwrap();
        let mut got = vec![];
        a.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"pong");
    };
    let (moved, ()) = tokio::join!(copy, ends);
    assert_eq!(moved.unwrap(), (14, 4));

    // a side that breaks ends both
    let (a, a_far) = duplex(64);
    let (b_far, mut b) = duplex(64);
    let copy = pipe(a_far, b_far, &opts);
    let ends = async {
        drop(a);
        let mut got = vec![];
        b.read_to_end(&mut got).await.unwrap();
        b.write_all(b"too late").await.unwrap();
        b.shutdown().await.unwrap();
    };
    let (moved, ()) = tokio::join!(copy, ends);
    assert!(moved.is_err(), "{moved:?}");
}

#[tokio::test(start_paused = true)]
async fn the_clocks() {
    let opts =
        PipeOptions { write_stall: Duration::from_secs(60), half_closed_idle: Duration::from_secs(300), idle: None };

    // no idle clock: two silent sides are left alone
    let (_a, a_far) = duplex(64);
    let (b_far, _b) = duplex(64);
    assert!(timeout(Duration::from_secs(86_400), pipe(a_far, b_far, &opts)).await.is_err());

    // with one, silence ends it, and traffic keeps it going
    let idle = PipeOptions { idle: Some(Duration::from_secs(30)), ..opts.clone() };
    let (mut a, a_far) = duplex(64);
    let (b_far, mut b) = duplex(64);
    let started = Instant::now();
    let copy = pipe(a_far, b_far, &idle);
    let ends = async {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_secs(20)).await;
            a.write_all(b"x").await.unwrap();
            b.read_exact(&mut [0u8; 1]).await.unwrap();
        }
    };
    let (moved, ()) = tokio::join!(copy, ends);
    assert_eq!(moved.unwrap_err().kind(), io::ErrorKind::TimedOut);
    // bytes one way kept both directions open for the 80 s they flowed; then the silence ended it
    assert!(
        started.elapsed() >= Duration::from_secs(110) && started.elapsed() < Duration::from_secs(141),
        "{:?}",
        started.elapsed()
    );

    // one side finished, the other silent: the half-closed clock
    let (mut a, a_far) = duplex(64);
    let (b_far, _b) = duplex(64);
    let started = Instant::now();
    a.shutdown().await.unwrap();
    let moved = pipe(a_far, b_far, &opts).await;
    assert_eq!(moved.unwrap_err().kind(), io::ErrorKind::TimedOut);
    assert!(started.elapsed() >= Duration::from_secs(300) && started.elapsed() < Duration::from_secs(301));

    // a side that stops reading: the write clock
    let (mut a, a_far) = duplex(64);
    let (b_far, _b) = duplex(64);
    let started = Instant::now();
    let copy = pipe(a_far, b_far, &opts);
    let ends = async {
        // (some of it fits in the pipes; the rest waits on a reader that never comes)
        let _ = timeout(Duration::from_secs(1), a.write_all(&[1u8; 1000])).await;
    };
    let (moved, ()) = tokio::join!(copy, ends);
    assert_eq!(moved.unwrap_err().kind(), io::ErrorKind::TimedOut);
    assert!(started.elapsed() >= Duration::from_secs(60) && started.elapsed() < Duration::from_secs(62));
}

#[tokio::test]
async fn a_forwarder_listens_on_loopback_only() {
    for addr in ["0.0.0.0:0", "192.0.2.1:0", "[::]:0"] {
        let e = Forwarder::bind(addr.parse().unwrap(), Target::Api).await.err().expect(addr);
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{addr}");
    }
    let f = Forwarder::bind("127.0.0.1:0".parse().unwrap(), Target::Hub).await.unwrap();
    assert!(f.local_addr().unwrap().ip().is_loopback());
    assert_eq!(f.target(), Target::Hub);
}

#[tokio::test]
async fn a_stream_that_cannot_be_opened_closes_the_connection_and_the_port_stays() {
    let f = Forwarder::bind("127.0.0.1:0".parse().unwrap(), Target::Api).await.unwrap();
    let addr = f.local_addr().unwrap();
    let tries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = tries.clone();
    let _task = tokio::spawn(f.run(move || {
        count.fetch_add(1, Ordering::Relaxed);
        async { Err::<tokio::io::DuplexStream, _>(io::Error::other("home is away")) }
    }));
    for n in 1..=3 {
        let mut c = TcpStream::connect(addr).await.unwrap();
        assert!(closed(&mut c).await);
        assert_eq!(tries.load(Ordering::Relaxed), n);
    }

    // and a home whose own service is down
    let (gone, gone_addr) = listener().await;
    drop(gone);
    let (addr, _task) = tunnel(Target::Api, Targets { api: Some(gone_addr), hub: None }, quick()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    assert!(closed(&mut c).await);
}

async fn echo_server() -> SocketAddr {
    let (server, addr) = listener().await;
    tokio::spawn(async move {
        loop {
            let (mut s, _) = server.accept().await.unwrap();
            tokio::spawn(async move {
                let (mut r, mut w) = s.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

#[tokio::test]
async fn only_so_many_connections_are_carried_at_once() {
    let echo = echo_server().await;
    let f = Forwarder::bind("127.0.0.1:0".parse().unwrap(), Target::Api).await.unwrap().max_streams(1);
    let (addr, _task) = tunnel_with(f, Targets { api: Some(echo), hub: None }, PipeOptions::default()).await;
    let mut first = TcpStream::connect(addr).await.unwrap();
    first.write_all(b"a").await.unwrap();
    let mut buf = [0u8; 1];
    timeout(LONG, first.read_exact(&mut buf)).await.unwrap().unwrap();

    let mut second = TcpStream::connect(addr).await.unwrap();
    second.write_all(b"b").await.unwrap();
    assert!(timeout(Duration::from_millis(300), second.read_exact(&mut buf)).await.is_err(), "it waits its turn");
    drop(first);
    timeout(LONG, second.read_exact(&mut buf)).await.unwrap().unwrap();
    assert_eq!(&buf, b"b");
}

#[tokio::test]
async fn stopping_the_forwarder_closes_the_port_and_what_it_carried() {
    let echo = echo_server().await;
    let (addr, task) = tunnel(Target::Api, Targets { api: Some(echo), hub: None }, PipeOptions::default()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"a").await.unwrap();
    let mut buf = [0u8; 1];
    timeout(LONG, c.read_exact(&mut buf)).await.unwrap().unwrap();
    task.abort();
    let _ = task.await;
    assert!(closed(&mut c).await);
    assert!(TcpStream::connect(addr).await.is_err(), "nothing listens there any more");
}

#[tokio::test]
async fn own_is_a_target_that_never_leads_to_an_address() {
    assert_eq!(encode_header(Target::Own), b"{\"to\":\"own\"}\n");
    assert_eq!(parse_header(b"{\"to\":\"own\"}").unwrap(), Target::Own);
    assert!(encode_header(Target::Own).len() <= HEADER_MAX);
    // whatever addresses a computer names, `own` is not one of them: only the endpoint hands such a stream over
    let (l, addr) = listener().await;
    let targets = Targets { api: Some(addr), hub: Some(addr) };
    assert_eq!(targets.addr(Target::Own), None);
    let (mut near, far) = duplex(1024);
    write_header(&mut near, Target::Own).await.unwrap();
    let r = serve(far, &targets, &quick()).await;
    assert!(matches!(&r, Err(TunnelError::Refused(name)) if name == "own"), "{r:?}");
    assert!(timeout(Duration::from_millis(200), l.accept()).await.is_err(), "nothing was connected");
}
