//! Keys, pairing and the tunnel, and the endpoint that carries them.
//!
//! Every computer has one key (`identity`). Home keeps the list of devices it paired with (`paired`), and only
//! those may open a tunnel. A device joins with a short code or a link (`code`), which both ends prove they hold
//! in one exchange (`pairing`). After that, each connection an app makes to a loopback port is carried to home
//! as one stream that starts with a line saying where it goes (`tunnel`).
//!
//! Home also signs its list of devices (`roster`), which every paired device keeps: with it a computer that is
//! not home can tell which devices may reach its own chats while the home computer is away.
//!
//! A computer answers for its own chats as HTTP on a tunnel stream; `http1` is the little of HTTP that asking it
//! takes.
//!
//! Those modules do not know how the two ends reach each other. Pairing and the tunnel run over anything that
//! is `AsyncRead + AsyncWrite`, and a peer is named by the 32 bytes of its public key, so their tests use
//! `tokio::io::duplex` and local TCP. `endpoint` is what carries the streams between computers, and the only
//! module that names the network library.

/// A sealed copy of the home computer, opened only with its recovery code.
pub mod backup;
pub mod code;
pub mod control;
pub mod endpoint;
pub mod http1;
pub mod identity;
pub mod paired;
pub mod pairing;
/// How a device that is not home reaches the other computers on its list while home is away, and keeps the list
/// fresh. With the stand-in, behind the same feature.
#[cfg(feature = "standin")]
pub mod reach;
pub mod roster;
/// What a device's loopback ports answer while the home computer is away. Behind the `standin` feature: a
/// computer's helper and a phone's library turn it on; nothing else needs it.
#[cfg(feature = "standin")]
pub mod standin;
pub mod tunnel;

/// (the helper wipes key bytes it handled with the same code)
pub use zeroize;

/// Lowercase hex, and back for an exact number of bytes.
pub(crate) mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
            out.push(char::from_digit(u32::from(b & 15), 16).unwrap_or('0'));
        }
        out
    }

    /// `N` bytes from exactly `2 * N` hex digits of either case. Anything else is None.
    pub fn decode<const N: usize>(text: &str) -> Option<[u8; N]> {
        let t = text.as_bytes();
        if t.len() != N * 2 {
            return None;
        }
        let mut out = [0u8; N];
        for (byte, [hi, lo]) in out.iter_mut().zip(t.as_chunks::<2>().0) {
            let hi = char::from(*hi).to_digit(16)?;
            let lo = char::from(*lo).to_digit(16)?;
            *byte = (hi * 16 + lo) as u8;
        }
        Some(out)
    }
}

/// Bytes from the system's random source.
pub(crate) fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the system random source");
    b
}

/// Text a person or another device supplied, made safe to show and store: no control characters, at most `max`
/// characters, no space at either end.
pub(crate) fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

/// A machine name as the hub knows it (sb-hub ticket.rs `valid_machine`): 1 to 32 lowercase letters and digits.
pub(crate) fn valid_machine(m: &str) -> bool {
    !m.is_empty() && m.len() <= 32 && m.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A new empty folder for one test.
    pub fn scratch(tag: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "sb-link-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The same "random" bytes on every run, so a failure can be repeated (xorshift64*).
    pub struct Dice(pub u64);

    impl Dice {
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        pub fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }

        pub fn bytes(&mut self, max: usize) -> Vec<u8> {
            let n = self.below(max + 1);
            (0..n).map(|_| self.next() as u8).collect()
        }
    }

    #[test]
    fn hex_round_trip_and_refusals() {
        let b = [0u8, 1, 0x7f, 0x80, 0xff];
        assert_eq!(crate::hex::encode(&b), "00017f80ff");
        assert_eq!(crate::hex::decode::<5>("00017F80ff"), Some(b));
        assert_eq!(crate::hex::decode::<5>("00017f80f"), None);
        assert_eq!(crate::hex::decode::<5>("00017f80fg"), None);
        assert_eq!(crate::hex::decode::<5>("+0017f80ff"), None);
        assert_eq!(crate::hex::decode::<2>("ééé"), None);
    }
}
