//! The password-authenticated key exchange behind pairing, and the only file that names the crate doing it.
//!
//! Each side turns the pairing secret into one message and, from the other side's message, a shared key. The
//! keys match only when both used the same secret, and a side that used a wrong one learns nothing it could test
//! other guesses against: one guess for each exchange. pairing.rs needs exactly that and nothing else about the
//! scheme, so replacing SPAKE2 (the `spake2` crate, 0.4.0, which has had no audit) means another `Pake` here.

use spake2::{Ed25519Group, Identity, Password, Spake2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Joiner,
    Host,
}

pub trait Pake: Sized {
    /// Starts one exchange: the state to keep and the message to send. `joiner` and `host` are the two ends'
    /// names (their ids), which both sides must give the same way round.
    fn start(role: Role, password: &[u8], joiner: &[u8], host: &[u8]) -> (Self, Vec<u8>);

    /// The shared key, from the other side's message. None when the message is not one this scheme makes
    /// (wrong length, not a point, or this side's own message sent back).
    fn finish(self, peer: &[u8]) -> Option<Vec<u8>>;
}

/// SPAKE2 over Ed25519. The joiner is side A and the host side B.
pub struct Spake(Spake2<Ed25519Group>);

impl Pake for Spake {
    fn start(role: Role, password: &[u8], joiner: &[u8], host: &[u8]) -> (Spake, Vec<u8>) {
        let (pw, a, b) = (Password::new(password), Identity::new(joiner), Identity::new(host));
        let (state, msg) = match role {
            Role::Joiner => Spake2::<Ed25519Group>::start_a(&pw, &a, &b),
            Role::Host => Spake2::<Ed25519Group>::start_b(&pw, &a, &b),
        };
        (Spake(state), msg)
    }

    fn finish(self, peer: &[u8]) -> Option<Vec<u8>> {
        self.0.finish(peer).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Dice;

    fn run(pw_j: &[u8], pw_h: &[u8], ids_j: (&[u8], &[u8]), ids_h: (&[u8], &[u8])) -> (Vec<u8>, Vec<u8>) {
        let (j, mj) = Spake::start(Role::Joiner, pw_j, ids_j.0, ids_j.1);
        let (h, mh) = Spake::start(Role::Host, pw_h, ids_h.0, ids_h.1);
        (j.finish(&mh).unwrap(), h.finish(&mj).unwrap())
    }

    #[test]
    fn the_same_secret_and_names_give_the_same_key() {
        let (a, b) = run(b"code", b"code", (b"j", b"h"), (b"j", b"h"));
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        let (c, _) = run(b"code", b"code", (b"j", b"h"), (b"j", b"h"));
        assert_ne!(a, c, "every exchange has its own key");
    }

    #[test]
    fn a_different_secret_or_name_gives_different_keys() {
        let (a, b) = run(b"code", b"cod3", (b"j", b"h"), (b"j", b"h"));
        assert_ne!(a, b);
        let (a, b) = run(b"code", b"code", (b"j", b"h"), (b"j", b"other"));
        assert_ne!(a, b);
        let (a, b) = run(b"code", b"code", (b"j", b"h"), (b"h", b"j"));
        assert_ne!(a, b);
    }

    #[test]
    fn messages_that_are_not_messages_are_refused() {
        let (_, own) = Spake::start(Role::Joiner, b"code", b"j", b"h");
        let finish = |msg: &[u8]| Spake::start(Role::Joiner, b"code", b"j", b"h").0.finish(msg);
        assert!(finish(&[]).is_none());
        assert!(finish(&own[..own.len() - 1]).is_none());
        assert!(finish(&[own.as_slice(), &[0]].concat()).is_none());
        assert!(finish(&own).is_none(), "a message from the same side");
        let mut dice = Dice(0x9a4e_0001);
        for _ in 0..2000 {
            let _ = finish(&dice.bytes(40));
            // the right length and side, any 32 bytes after it: a key or a refusal, never a panic
            let mut msg = vec![b'B'];
            msg.extend((0..32).map(|_| dice.next() as u8));
            let _ = finish(&msg);
        }
    }
}
