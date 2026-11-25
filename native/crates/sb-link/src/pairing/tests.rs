use super::*;
use crate::identity::DeviceKey;
use crate::testing::Dice;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::{DuplexStream, duplex};

const NOW: u64 = 1_795_000_000;

type Check = fn(&PairError) -> bool;

/// A check that an error is the one meant, as a plain function so a table can hold several.
macro_rules! is {
    ($p:pat) => {
        (|e: &PairError| matches!(e, $p)) as Check
    };
}

struct World {
    offers: Offers,
    home: Home,
    joiner: DeviceId,
    minted: AtomicU32,
}

fn world() -> World {
    World {
        offers: Offers::new(),
        home: Home {
            id: DeviceKey::from_bytes([1; 32]).id(),
            name: "Home Mac".into(),
            relay: Some("https://relay.example.com/".into()),
        },
        joiner: DeviceKey::from_bytes([2; 32]).id(),
        minted: AtomicU32::new(0),
    }
}

fn about() -> About {
    About { name: "Sam's Netbook".into(), kind: Kind::Computer, os: "macos".into(), stays_with: None }
}

fn grant() -> Grant {
    Grant { machine: Some("netbook".into()), token: Some("tok-0123456789abcdef".into()) }
}

impl World {
    /// Both ends see the same ids and channel: the id that was dialed, and the joiner's.
    fn binding(&self, dialed: DeviceId) -> Binding<'static> {
        Binding { host: dialed.0, joiner: self.joiner.0, channel: b"exporter" }
    }

    /// One whole exchange over a pipe, each side with its own view of the binding. Each side's end of the pipe
    /// closes when it is done, as a connection would.
    async fn run_as(
        &self,
        secret: &Secret,
        select: Select<'_>,
        joiner_sees: Binding<'_>,
        host_sees: Binding<'_>,
        now: u64,
    ) -> (Result<Welcome, PairError>, Result<Joined, PairError>) {
        let (a, b) = duplex(4096);
        let j = async {
            let mut s = a;
            join(&mut s, secret, &joiner_sees, &about()).await
        };
        let h = async {
            let mut s = b;
            host(&mut s, &self.offers, select, &host_sees, &self.home, now, |_| async {
                self.minted.fetch_add(1, Ordering::Relaxed);
                Ok(grant())
            })
            .await
        };
        tokio::join!(j, h)
    }

    /// A joiner typing `typed` while home listens for `code`.
    async fn code(
        &self,
        typed: &ShortCode,
        code: &ShortCode,
        now: u64,
    ) -> (Result<Welcome, PairError>, Result<Joined, PairError>) {
        let b = self.binding(code.rendezvous_id());
        self.run_as(&Secret::Code(typed.clone()), Select::Code(code), b, b, now).await
    }

    fn minted(&self) -> u32 {
        self.minted.load(Ordering::Relaxed)
    }
}

/// A code that is certainly not `c`.
fn other_than(c: &ShortCode) -> ShortCode {
    loop {
        let o = ShortCode::generate();
        if o != *c {
            return o;
        }
    }
}

#[tokio::test]
async fn the_right_code_pairs() {
    let w = world();
    let (code, expires) = w.offers.mint_code(NOW);
    assert_eq!(expires, NOW + OFFER_TTL);
    assert_eq!(w.offers.codes(NOW), std::slice::from_ref(&code));
    let (welcome, joined) = w.code(&code, &code, NOW + 30).await;

    let welcome = welcome.unwrap();
    assert_eq!(welcome.home_id, w.home.id, "home's own id, not the rendezvous id");
    assert_eq!(welcome.home_name, "Home Mac");
    assert_eq!(welcome.relay.as_deref(), Some("https://relay.example.com/"));
    assert_eq!(welcome.machine.as_deref(), Some("netbook"));
    assert_eq!(welcome.token.as_deref(), Some("tok-0123456789abcdef"));
    assert!(!format!("{welcome:?}").contains("tok-"), "a token is never printed by Debug");

    let joined = joined.unwrap();
    assert_eq!(joined, Joined { id: w.joiner, about: about(), machine: Some("netbook".into()), via: Via::Code });
    assert_eq!(w.minted(), 1);
    assert_eq!(w.offers.live(NOW + 30), 0, "used up");
}

#[tokio::test]
async fn a_link_pairs_and_must_be_answered_by_the_home_it_names() {
    let w = world();
    let (secret, _) = w.offers.mint_link(NOW);
    let link = PairLink { home: w.home.id, relay: None, secret, name: "Home Mac".into() };
    let secret = Secret::from(&link);
    let b = w.binding(link.home);
    let (welcome, joined) = w.run_as(&secret, Select::Link, b, b, NOW).await;
    assert_eq!(welcome.unwrap().home_id, w.home.id);
    assert_eq!(joined.unwrap().via, Via::Link);
    assert_eq!(w.offers.live(NOW), 0);

    // a link's secret is not a code, and a code is not found by a hint
    let (secret, _) = w.offers.mint_link(NOW);
    let (code, _) = w.offers.mint_code(NOW);
    let b = w.binding(code.rendezvous_id());
    let (welcome, joined) = w.run_as(&Secret::Link(secret), Select::Code(&code), b, b, NOW).await;
    assert!(matches!((welcome, joined), (Err(PairError::Gone), Err(PairError::Gone))));
    let b = w.binding(w.home.id);
    let (welcome, joined) = w.run_as(&Secret::Code(code.clone()), Select::Link, b, b, NOW).await;
    assert!(matches!((welcome, joined), (Err(PairError::Gone), Err(PairError::Gone))));
    assert_eq!((w.offers.fails(&Secret::Link(secret)), w.offers.fails(&Secret::Code(code))), (Some(0), Some(0)));

    // the link names one home and another answers (its id is what the joiner dialed, its WELCOME says otherwise)
    let elsewhere = DeviceKey::from_bytes([9; 32]).id();
    let b = w.binding(elsewhere);
    let (welcome, _) = w.run_as(&Secret::Link(secret), Select::Link, b, b, NOW).await;
    assert!(matches!(welcome, Err(PairError::WrongHome)), "{welcome:?}");
}

#[tokio::test]
async fn a_wrong_code_fails_and_counts() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let (welcome, joined) = w.code(&other_than(&code), &code, NOW).await;
    assert!(matches!(welcome, Err(PairError::WrongCode)), "{welcome:?}");
    assert!(matches!(joined, Err(PairError::WrongCode)), "{joined:?}");
    assert_eq!(w.offers.fails(&Secret::Code(code.clone())), Some(1));
    assert_eq!(w.minted(), 0, "nothing is made for a device that did not prove itself");
    // the right code still works after a slip
    let (welcome, joined) = w.code(&code, &code, NOW).await;
    assert!(welcome.is_ok() && joined.is_ok());
}

#[tokio::test]
async fn a_code_expires_after_ten_minutes() {
    let w = world();
    let (code, expires) = w.offers.mint_code(NOW);
    let (welcome, joined) = w.code(&code, &code, expires).await;
    assert!(matches!(welcome, Err(PairError::Gone)), "{welcome:?}");
    assert!(matches!(joined, Err(PairError::Gone)), "{joined:?}");
    assert_eq!(w.minted(), 0);
    assert!(w.offers.codes(expires).is_empty());

    let (code, expires) = w.offers.mint_code(NOW);
    assert_eq!(expires - NOW, 600);
    let (welcome, joined) = w.code(&code, &code, expires - 1).await;
    assert!(welcome.is_ok() && joined.is_ok(), "the last second still works");
}

#[tokio::test]
async fn a_code_works_once() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let (welcome, joined) = w.code(&code, &code, NOW).await;
    assert!(welcome.is_ok() && joined.is_ok());
    let (welcome, joined) = w.code(&code, &code, NOW + 1).await;
    assert!(matches!(welcome, Err(PairError::Gone)), "{welcome:?}");
    assert!(matches!(joined, Err(PairError::Gone)), "{joined:?}");
    assert_eq!(w.minted(), 1);
}

#[tokio::test]
async fn five_wrong_codes_kill_the_code() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let (kept, _) = w.offers.mint_code(NOW);
    for n in 1..=MAX_FAILS {
        let (welcome, joined) = w.code(&other_than(&code), &code, NOW).await;
        assert!(matches!(welcome, Err(PairError::WrongCode)), "{n}: {welcome:?}");
        assert!(matches!(joined, Err(PairError::WrongCode)), "{n}: {joined:?}");
        let left = w.offers.fails(&Secret::Code(code.clone()));
        assert_eq!(left, (n < MAX_FAILS).then_some(n), "{n}");
    }
    let (welcome, joined) = w.code(&code, &code, NOW).await;
    assert!(matches!(welcome, Err(PairError::Gone)), "the right code is too late: {welcome:?}");
    assert!(matches!(joined, Err(PairError::Gone)), "{joined:?}");
    assert_eq!(w.minted(), 0);
    assert_eq!(w.offers.codes(NOW), [kept], "another code is not touched");
}

#[tokio::test]
async fn the_keys_are_bound_to_both_ids_and_the_channel() {
    let w = world();
    let stranger = DeviceKey::from_bytes([66; 32]).id();
    let (code, _) = w.offers.mint_code(NOW);
    let honest = w.binding(code.rendezvous_id());
    // Someone in the middle: each honest side has a channel to him, not to the other. Whatever differs between
    // what the two sides see (who answered, who dialed, the channel itself) breaks the proof, right code or not.
    let views = [
        Binding { host: stranger.0, ..honest },
        Binding { joiner: stranger.0, ..honest },
        Binding { channel: b"another channel", ..honest },
        Binding { channel: b"", ..honest },
    ];
    for (n, joiner_sees) in views.into_iter().enumerate() {
        let (welcome, joined) =
            w.run_as(&Secret::Code(code.clone()), Select::Code(&code), joiner_sees, honest, NOW).await;
        assert!(matches!(welcome, Err(PairError::WrongCode)), "{n}: {welcome:?}");
        assert!(matches!(joined, Err(PairError::WrongCode)), "{n}: {joined:?}");
    }
    assert_eq!(w.minted(), 0);
    assert_eq!(w.offers.fails(&Secret::Code(code.clone())), Some(4), "each one is a spent guess");
    let (welcome, joined) = w.code(&code, &code, NOW).await;
    assert!(welcome.is_ok() && joined.is_ok());
}

#[test]
fn every_part_of_the_transcript_changes_every_key() {
    let b = Binding { host: [1; 32], joiner: [2; 32], channel: b"ch" };
    let base = transcript(&b, b"hint", b"mj", b"mh");
    let others = [
        transcript(&Binding { host: [3; 32], ..b }, b"hint", b"mj", b"mh"),
        transcript(&Binding { joiner: [3; 32], ..b }, b"hint", b"mj", b"mh"),
        transcript(&Binding { host: b.joiner, joiner: b.host, ..b }, b"hint", b"mj", b"mh"),
        transcript(&Binding { channel: b"c", ..b }, b"hint", b"mj", b"mh"),
        transcript(&b, b"hin", b"mj", b"mh"),
        transcript(&b, b"hint", b"mh", b"mj"),
        // the same bytes cut at another place
        transcript(&b, b"hintm", b"j", b"mh"),
        transcript(&b, b"hint", b"mjm", b"h"),
    ];
    let keys = Keys::derive(b"shared", &base);
    let all = [keys.confirm_j, keys.confirm_h, keys.seal_j, keys.seal_h];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b, "the four keys are four");
        }
    }
    for (n, t) in others.iter().enumerate() {
        assert_ne!(*t, base, "{n}");
        let k = Keys::derive(b"shared", t);
        assert_ne!(k.confirm_j, keys.confirm_j, "{n}");
        assert_ne!(k.seal_h, keys.seal_h, "{n}");
    }
    assert_ne!(Keys::derive(b"share", &base).confirm_h, keys.confirm_h);
}

#[test]
fn a_sealed_message_opens_only_as_itself() {
    let key = [5u8; 32];
    let sealed = seal(&key, ABOUT, b"hello").unwrap();
    assert_eq!(open(&key, ABOUT, &sealed).unwrap(), b"hello");
    assert!(open(&key, WELCOME, &sealed).is_err(), "not as the other message");
    assert!(open(&[6u8; 32], ABOUT, &sealed).is_err());
    for i in 0..sealed.len() {
        let mut bent = sealed.clone();
        bent[i] ^= 1;
        assert!(open(&key, ABOUT, &bent).is_err(), "{i}");
    }
    assert!(open(&key, ABOUT, &sealed[..sealed.len() - 1]).is_err());
    assert!(open(&key, ABOUT, &[]).is_err());
}

/// The body of the first frame a real joiner sends for `code`.
fn hello_for(w: &World, code: &ShortCode) -> Vec<u8> {
    let (_, msg) =
        Spake::start(Role::Joiner, &Secret::Code(code.clone()).password(), &w.joiner.0, &code.rendezvous_id().0);
    [&[VERSION, 0][..], &msg].concat()
}

async fn host_on(w: &World, s: &mut DuplexStream, code: &ShortCode) -> Result<Joined, PairError> {
    let b = w.binding(code.rendezvous_id());
    host(s, &w.offers, Select::Code(code), &b, &w.home, NOW, |_| async {
        w.minted.fetch_add(1, Ordering::Relaxed);
        Ok(grant())
    })
    .await
}

#[tokio::test]
async fn one_exchange_at_a_time_and_a_broken_one_costs_nothing() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let (mut slow, mut b1) = duplex(4096);
    let first = host_on(&w, &mut b1, &code);
    let rest = async {
        // a joiner that says hello and then nothing
        write_frame(&mut slow, HELLO, &hello_for(&w, &code)).await.unwrap();
        assert_eq!(read_frame(&mut slow).await.unwrap().0, REPLY);
        // meanwhile a second one, with the right code, is told to wait
        let (welcome, joined) = w.code(&code, &code, NOW).await;
        assert!(matches!(welcome, Err(PairError::Busy)), "{welcome:?}");
        assert!(matches!(joined, Err(PairError::Busy)), "{joined:?}");
        drop(slow);
    };
    let (first, ()) = tokio::join!(first, rest);
    assert!(matches!(first, Err(PairError::Io(_))), "{first:?}");
    assert_eq!(w.offers.fails(&Secret::Code(code.clone())), Some(0), "a broken stream is not a wrong guess");
    let (welcome, joined) = w.code(&code, &code, NOW).await;
    assert!(welcome.is_ok() && joined.is_ok(), "and the code is free again");
}

#[tokio::test(start_paused = true)]
async fn a_joiner_that_goes_quiet_is_dropped_after_the_deadline() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let (mut quiet, mut b) = duplex(4096);
    write_frame(&mut quiet, HELLO, &hello_for(&w, &code)).await.unwrap();
    let started = tokio::time::Instant::now();
    let r = host_on(&w, &mut b, &code).await;
    assert!(matches!(r, Err(PairError::Timeout)), "{r:?}");
    let took = started.elapsed();
    assert!(took >= EXCHANGE_TIMEOUT && took < EXCHANGE_TIMEOUT + Duration::from_secs(1), "{took:?}");
    assert_eq!(w.offers.fails(&Secret::Code(code.clone())), Some(0));

    // and a home that goes quiet does not hold the joiner
    let (mut a, _held) = duplex(4096);
    let b = w.binding(code.rendezvous_id());
    let r = join(&mut a, &Secret::Code(code), &b, &about()).await;
    assert!(matches!(r, Err(PairError::Timeout)), "{r:?}");
}

#[tokio::test]
async fn a_home_that_cannot_add_the_device_says_so_and_the_code_survives() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let b = w.binding(code.rendezvous_id());
    let (mut a, mut s) = duplex(4096);
    let secret = Secret::Code(code.clone());
    let me = about();
    let j = join(&mut a, &secret, &b, &me);
    let h = host(&mut s, &w.offers, Select::Code(&code), &b, &w.home, NOW, |req| async move {
        assert_eq!(req, Request { id: DeviceKey::from_bytes([2; 32]).id(), about: about(), via: Via::Code });
        Err("the hub is not running\u{7}".to_string())
    });
    let (welcome, joined) = tokio::join!(j, h);
    assert!(matches!(welcome, Err(PairError::Refused(_))), "{welcome:?}");
    let Err(PairError::Refused(why)) = joined else { panic!("{joined:?}") };
    assert_eq!(why, "the hub is not running");
    assert_eq!(w.offers.fails(&secret), Some(0));
    let (welcome, joined) = w.code(&code, &code, NOW).await;
    assert!(welcome.is_ok() && joined.is_ok());
}

#[tokio::test]
async fn an_answer_that_cannot_be_sent_is_reported_with_what_was_made() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let b = w.binding(code.rendezvous_id());
    let (a, mut s) = duplex(4096);
    let gone = tokio::sync::Notify::new();
    let secret = Secret::Code(code.clone());
    let me = about();
    let j = async {
        let mut a = a;
        tokio::select! {
            r = join(&mut a, &secret, &b, &me) => Some(r),
            () = gone.notified() => None, // the device vanishes while home is making its place
        }
    };
    let h = host(&mut s, &w.offers, Select::Code(&code), &b, &w.home, NOW, |_| async {
        gone.notify_one();
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(grant())
    });
    let (welcome, joined) = tokio::join!(j, h);
    assert!(welcome.is_none());
    let Err(PairError::Undelivered { machine, .. }) = joined else { panic!("{joined:?}") };
    assert_eq!(machine.as_deref(), Some("netbook"));
    assert_eq!(w.offers.live(NOW), 0, "the code is used up all the same");
}

/// Carries bytes between two pipes and keeps a copy of everything, as someone on the wire would see it.
async fn tap(mut from: tokio::io::ReadHalf<DuplexStream>, mut to: tokio::io::WriteHalf<DuplexStream>) -> Vec<u8> {
    let mut seen = vec![];
    let mut buf = [0u8; 1024];
    while let Ok(n) = from.read(&mut buf).await {
        if n == 0 || to.write_all(&buf[..n]).await.is_err() {
            break;
        }
        seen.extend_from_slice(&buf[..n]);
    }
    let _ = to.shutdown().await;
    seen
}

#[tokio::test]
async fn nothing_private_crosses_the_wire_in_the_clear() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let b = w.binding(code.rendezvous_id());
    let (a, mid_a) = duplex(4096);
    let (mid_b, s) = duplex(4096);
    let (ar, aw) = tokio::io::split(mid_a);
    let (br, bw) = tokio::io::split(mid_b);
    let secret = Secret::Code(code.clone());
    let j = async {
        let mut a = a;
        join(&mut a, &secret, &b, &about()).await
    };
    let h = async {
        let mut s = s;
        host_on(&w, &mut s, &code).await
    };
    let (welcome, joined, up, down) = tokio::join!(j, h, tap(ar, bw), tap(br, aw));
    assert!(welcome.is_ok() && joined.is_ok());
    let has = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).any(|w| w == needle);
    for needle in
        [&b"tok-0123456789abcdef"[..], b"netbook", b"Netbook", b"Home Mac", b"relay.example", code.as_str().as_bytes()]
    {
        assert!(!has(&up, needle) && !has(&down, needle), "{:?}", String::from_utf8_lossy(needle));
    }
    assert!(!has(&down, w.home.id.to_string().as_bytes()));
    assert!(up.len() > 100 && down.len() > 100);
}

#[tokio::test]
async fn frames_that_break_the_rules_end_the_exchange() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let good = hello_for(&w, &code);
    let secret = Secret::Code(code.clone());

    // (what the joiner sends, what the host makes of it, the ABORT the joiner reads back)
    let cases = vec![
        ("over the limit", vec![0xff, 0xff, HELLO], is!(PairError::Protocol(_)), Some(vec![5])),
        ("empty frame", vec![0, 0, HELLO], is!(PairError::Protocol(_)), Some(vec![5])),
        ("cut short", vec![0, 40, HELLO, VERSION], is!(PairError::Io(_)), None),
        ("nothing", vec![], is!(PairError::Io(_)), None),
        ("out of order", vec![0, 2, CONFIRM_J, 0], is!(PairError::Protocol(_)), Some(vec![5])),
        ("another version", vec![0, 4, HELLO, 2, 0, 1], is!(PairError::Version(2)), Some(vec![6, VERSION])),
        ("no version", vec![0, 1, HELLO], is!(PairError::Protocol(_)), Some(vec![5])),
        ("odd hint", vec![0, 6, HELLO, VERSION, 3, 1, 2, 3], is!(PairError::Protocol(_)), Some(vec![5])),
        ("hint and no message", vec![0, 3, HELLO, VERSION, 0], is!(PairError::Protocol(_)), Some(vec![5])),
        (
            "not a key-exchange message",
            vec![0, 6, HELLO, VERSION, 0, 1, 2, 3],
            is!(PairError::Protocol(_)),
            Some(vec![5]),
        ),
    ];
    for (tag, bytes, check, reply) in cases {
        let (mut a, mut s) = duplex(4096);
        a.write_all(&bytes).await.unwrap();
        a.shutdown().await.unwrap();
        let e = host_on(&w, &mut s, &code).await.unwrap_err();
        assert!(check(&e), "{tag}: {e:?}");
        assert!(!e.message().is_empty());
        drop(s);
        let mut got = vec![];
        a.read_to_end(&mut got).await.unwrap();
        match reply {
            Some(body) => assert_eq!(got, [&[0, body.len() as u8 + 1, ABORT][..], &body].concat(), "{tag}"),
            None => assert!(got.is_empty(), "{tag}"),
        }
    }

    // a real hello, then something that is not the proof
    for (tag, then, want_fail) in [
        ("a wrong proof", [&[0, 33, CONFIRM_J][..], &[0u8; 32]].concat(), true),
        ("a short proof", vec![0, 2, CONFIRM_J, 7], true),
        ("a frame out of order", [&[0, 33, ABOUT][..], &[0u8; 32]].concat(), false),
    ] {
        let before = w.offers.fails(&secret).unwrap();
        let (mut a, mut s) = duplex(4096);
        write_frame(&mut a, HELLO, &good).await.unwrap();
        a.write_all(&then).await.unwrap();
        let e = host_on(&w, &mut s, &code).await.unwrap_err();
        match want_fail {
            true => assert!(matches!(e, PairError::WrongCode), "{tag}: {e:?}"),
            false => assert!(matches!(e, PairError::Protocol(_)), "{tag}: {e:?}"),
        }
        assert_eq!(w.offers.fails(&secret).unwrap() - before, u32::from(want_fail), "{tag}");
    }
    assert_eq!(w.minted(), 0);
}

#[tokio::test]
async fn a_joiner_is_not_fooled_by_a_home_that_breaks_the_rules() {
    let w = world();
    let code = ShortCode::generate();
    let b = w.binding(code.rendezvous_id());
    let secret = Secret::Code(code.clone());
    let reply = Spake::start(Role::Host, &secret.password(), &w.joiner.0, &b.host).1;
    let cases: Vec<(&str, Vec<Vec<u8>>, Check)> = vec![
        ("abort: wrong code", vec![vec![0, 2, ABORT, 1]], is!(PairError::WrongCode)),
        ("abort: gone", vec![vec![0, 2, ABORT, 2]], is!(PairError::Gone)),
        ("abort: busy", vec![vec![0, 2, ABORT, 3]], is!(PairError::Busy)),
        ("abort: refused", vec![vec![0, 2, ABORT, 4]], is!(PairError::Refused(_))),
        ("abort: version", vec![vec![0, 3, ABORT, 6, 9]], is!(PairError::Version(9))),
        ("abort: no reason", vec![vec![0, 1, ABORT]], is!(PairError::Protocol(_))),
        ("its own message back", vec![], is!(PairError::Protocol(_))),
        ("not a message", vec![vec![0, 4, REPLY, 1, 2, 3]], is!(PairError::Protocol(_))),
        ("out of order", vec![vec![0, 2, WELCOME, 0]], is!(PairError::Protocol(_))),
        (
            "a proof made without the code",
            vec![
                [&[0, reply.len() as u8 + 1, REPLY][..], &reply].concat(),
                [&[0, 33, CONFIRM_H][..], &[0u8; 32]].concat(),
            ],
            is!(PairError::WrongCode),
        ),
        (
            "gone after the reply",
            vec![[&[0, reply.len() as u8 + 1, REPLY][..], &reply].concat()],
            is!(PairError::Io(_)),
        ),
    ];
    for (tag, frames, check) in cases {
        let (mut a, mut s) = duplex(4096);
        let fake = async {
            let (kind, hello) = read_frame(&mut s).await.unwrap();
            assert_eq!((kind, hello[0], hello[1]), (HELLO, VERSION, 0));
            if tag == "its own message back" {
                write_frame(&mut s, REPLY, &hello[2..]).await.unwrap();
            }
            for f in &frames {
                s.write_all(f).await.unwrap();
            }
            // read whatever the joiner sends until it gives up, then go
            let mut sink = vec![];
            let _ = tokio::time::timeout(Duration::from_millis(50), s.read_to_end(&mut sink)).await;
            drop(s);
        };
        let me = about();
        let (r, ()) = tokio::join!(join(&mut a, &secret, &b, &me), fake);
        let e = r.unwrap_err();
        assert!(check(&e), "{tag}: {e:?}");
        assert!(!e.message().is_empty());
    }
}

#[tokio::test]
async fn random_bytes_never_panic_either_side() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let (link, _) = w.offers.mint_link(NOW);
    let good = hello_for(&w, &code);
    let secret = Secret::Code(code.clone());
    let b = w.binding(code.rendezvous_id());
    let mut dice = Dice(0xfa22_0001);
    for round in 0..600 {
        let noise = match round % 3 {
            // anything
            0 => dice.bytes(300),
            // well-formed frames with any type and body
            1 => {
                let mut out = vec![];
                for _ in 0..=dice.below(4) {
                    let body = dice.bytes(60);
                    out.extend_from_slice(&(body.len() as u16 + 1).to_be_bytes());
                    out.push([HELLO, REPLY, CONFIRM_J, CONFIRM_H, ABOUT, WELCOME, ABORT, 0, 200][dice.below(9)]);
                    out.extend_from_slice(&body);
                }
                out
            }
            // a real first frame, then anything
            _ => {
                let mut out = (good.len() as u16 + 1).to_be_bytes().to_vec();
                out.push(HELLO);
                out.extend_from_slice(&good);
                out.extend_from_slice(&dice.bytes(120));
                out
            }
        };
        // at the host, for a code and for a link
        for select in [Select::Code(&code), Select::Link] {
            let (mut a, mut s) = duplex(4096);
            a.write_all(&noise).await.unwrap();
            a.shutdown().await.unwrap();
            let r = host(&mut s, &w.offers, select, &b, &w.home, NOW, |_| async { Ok(grant()) }).await;
            assert!(r.is_err(), "round {round}");
        }
        // at the joiner
        let (mut a, mut s) = duplex(4096);
        s.write_all(&noise).await.unwrap();
        s.shutdown().await.unwrap();
        let r = join(&mut a, &secret, &b, &about()).await;
        assert!(r.is_err(), "round {round}");
        // a wrong proof may have ended the code: the rest of the run still has one to aim at
        if w.offers.fails(&secret).is_none() {
            break;
        }
    }
    assert!(w.offers.fails(&Secret::Link(link)).is_some(), "noise never guesses a link's hint");
}

#[tokio::test]
async fn a_device_that_stays_with_another_home_is_turned_down_before_anything_is_made_for_it() {
    let w = world();
    let (code, _) = w.offers.mint_code(NOW);
    let b = w.binding(code.rendezvous_id());
    let secret = Secret::Code(code.clone());
    let once = |stays_with: Option<DeviceId>| {
        let (w, secret, code) = (&w, &secret, &code);
        async move {
            let (x, y) = duplex(4096);
            let me = About { stays_with, ..about() };
            let j = async {
                let mut s = x;
                join(&mut s, secret, &b, &me).await
            };
            let h = async {
                let mut s = y;
                host(&mut s, &w.offers, Select::Code(code), &b, &w.home, NOW, |_| async {
                    w.minted.fetch_add(1, Ordering::Relaxed);
                    Ok(grant())
                })
                .await
            };
            tokio::join!(j, h)
        }
    };
    // paired with another home computer, and nobody said to replace it
    let other = DeviceKey::from_bytes([9; 32]).id();
    let (welcome, joined) = once(Some(other)).await;
    let (j, h) = (welcome.unwrap_err(), joined.unwrap_err());
    assert!(matches!(j, PairError::OtherHome) && matches!(h, PairError::OtherHome), "{j:?} {h:?}");
    assert_eq!((j.kind(), w.minted()), ("other_home", 0), "no place was made for it");
    assert!(j.message().contains("nothing was changed") && j.message().contains("replace"));
    assert_eq!((w.offers.live(NOW), w.offers.fails(&secret)), (1, Some(0)), "the code is as good as it was");
    // paired with this very home computer: pairing again is the usual way back
    let (welcome, _) = once(Some(w.home.id)).await;
    assert_eq!(welcome.unwrap().home_id, w.home.id);
    assert_eq!(w.minted(), 1);
    // and told to replace the old one, it says nothing of it and is taken
    let (code, _) = w.offers.mint_code(NOW);
    let b = w.binding(code.rendezvous_id());
    let (x, y) = duplex(4096);
    let (secret, me) = (Secret::Code(code.clone()), about());
    let j = async {
        let mut s = x;
        join(&mut s, &secret, &b, &me).await
    };
    let h = async {
        let mut s = y;
        host(&mut s, &w.offers, Select::Code(&code), &b, &w.home, NOW, |_| async { Ok(grant()) }).await
    };
    let (welcome, joined) = tokio::join!(j, h);
    assert!(welcome.is_ok() && joined.is_ok_and(|j| j.about.stays_with.is_none()));
    // what goes over the wire has the field only when there is something to say
    assert!(!serde_json::to_string(&about()).unwrap().contains("stays_with"));
    assert!(serde_json::to_string(&About { stays_with: Some(other), ..about() }).unwrap().contains("stays_with"));
}

#[test]
fn what_home_and_the_device_say_is_cleaned_and_checked() {
    let a = About { name: " a\u{0}b\n ".repeat(40), kind: Kind::Phone, os: "x".repeat(99), stays_with: None }.cleaned();
    assert!(a.name.chars().count() <= NAME_MAX && !a.name.chars().any(char::is_control));
    assert_eq!(a.os.len(), OS_MAX);
    assert!(serde_json::from_str::<About>(r#"{"name":"x","kind":"toaster"}"#).is_err());
    assert_eq!(serde_json::from_str::<About>(r#"{"name":"x","kind":"phone"}"#).unwrap().os, "");

    let ok = Welcome {
        home_id: DeviceKey::from_bytes([1; 32]).id(),
        home_name: "Home\u{7}".into(),
        relay: None,
        machine: None,
        token: None,
    };
    assert_eq!(ok.clone().checked().unwrap().home_name, "Home");
    let not_a_key = (0u8..=255).map(|b| DeviceId([b; 32])).find(|id| !id.is_key()).unwrap();
    let bad = [
        Welcome { home_id: not_a_key, ..ok.clone() },
        Welcome { relay: Some("javascript:alert(1)".into()), ..ok.clone() },
        Welcome { machine: Some("../etc".into()), ..ok.clone() },
        Welcome { machine: Some(String::new()), ..ok.clone() },
        Welcome { token: Some("a b".into()), ..ok.clone() },
        Welcome { token: Some(String::new()), ..ok.clone() },
        Welcome { token: Some("x".repeat(TOKEN_MAX + 1)), ..ok.clone() },
    ];
    for b in bad {
        assert!(matches!(b.clone().checked(), Err(PairError::Protocol(_))), "{b:?}");
    }
    // the largest answer home can make fits in a frame
    let big = Welcome {
        home_name: "é".repeat(NAME_MAX),
        relay: Some(format!("https://{}", "a".repeat(crate::code::RELAY_MAX - 8))),
        machine: Some("m".repeat(32)),
        token: Some("t".repeat(TOKEN_MAX)),
        ..ok
    };
    let sealed = seal(&[0; 32], WELCOME, &serde_json::to_vec(&big.checked().unwrap()).unwrap()).unwrap();
    assert!(sealed.len() <= FRAME_MAX, "{}", sealed.len());
}

#[test]
fn only_so_many_offers_live_at_once() {
    let offers = Offers::new();
    let (first, _) = offers.mint_code(NOW);
    let (link, _) = offers.mint_link(NOW);
    let later: Vec<ShortCode> = (0..MAX_OFFERS - 1).map(|_| offers.mint_code(NOW + 1).0).collect();
    assert_eq!(offers.live(NOW + 1), MAX_OFFERS);
    assert!(offers.fails(&Secret::Code(first)).is_none(), "the oldest made way");
    assert!(offers.fails(&Secret::Link(link)).is_some());
    assert!(offers.revoke(&Secret::Link(link)) && !offers.revoke(&Secret::Link(link)));
    assert_eq!(offers.codes(NOW + 1), later);
    assert_eq!(offers.live(NOW + 1 + OFFER_TTL), 0);
    assert_eq!(format!("{:?}", Secret::Link(link)), "Secret::Link(..)");
}
