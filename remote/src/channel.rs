//! The encrypted channel between a desktop and its phone: Noise `KK` (both ends already hold the other's public
//! key from the pairing), so only that phone and that desktop can complete the handshake. Each connection adds
//! fresh ephemeral keys: recorded traffic stays closed even if a device's key is taken later.
//!
//! The phone starts (`Channel::initiator`, message `hs1`), the desktop answers (`Channel::responder`, `hs2`). After
//! that `seal` and `open` carry messages of any size up to `MAX_MESSAGE`, cut into Noise-sized pieces. Pieces must
//! arrive in order and once: anything else is `Error::Crypto` and the channel is dead (a new handshake is needed).

use x25519_dalek::{PublicKey, StaticSecret};

use crate::{Error, random};

const PATTERN: &str = "Noise_KK_25519_ChaChaPoly_SHA256";
/// Plain bytes per piece (a Noise message holds 65535 with its tag).
const PIECE: usize = 60_000;
/// The longest message a channel carries.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;

/// A device's long-term key pair (X25519).
#[derive(Clone)]
pub struct Keys {
    pub private: [u8; 32],
    pub public: [u8; 32],
}

impl Keys {
    pub fn generate() -> Self {
        Self::from_private(random::<32>())
    }

    pub fn from_private(private: [u8; 32]) -> Self {
        let public = PublicKey::from(&StaticSecret::from(private)).to_bytes();
        Self { private, public }
    }

    /// The shared secret with another device's public key (the same from either end).
    pub(crate) fn shared(&self, their_public: &[u8; 32]) -> [u8; 32] {
        StaticSecret::from(self.private).diffie_hellman(&PublicKey::from(*their_public)).to_bytes()
    }
}

/// The phone's half-done handshake, waiting for the desktop's answer.
pub struct Handshake(snow::HandshakeState);

pub struct Channel {
    transport: snow::TransportState,
    /// Pieces of a message still arriving.
    partial: Vec<u8>,
}

fn builder<'a>(keys: &'a Keys, their_public: &'a [u8; 32], prologue: &'a [u8]) -> Result<snow::Builder<'a>, Error> {
    let params: snow::params::NoiseParams = PATTERN.parse().map_err(|_| Error::Malformed("noise"))?;
    snow::Builder::new(params)
        .local_private_key(&keys.private)
        .and_then(|b| b.remote_public_key(their_public))
        .and_then(|b| b.prologue(prologue))
        .map_err(|_| Error::Malformed("clave"))
}

/// Both ends bind the handshake to their room: a handshake recorded in one room says nothing in another.
fn prologue(room: &str) -> Vec<u8> {
    format!("buddy-remote-v1:{room}").into_bytes()
}

impl Channel {
    /// The phone's side: returns the handshake to finish and the first message (`hs1`).
    pub fn initiator(keys: &Keys, their_public: &[u8; 32], room: &str) -> Result<(Handshake, Vec<u8>), Error> {
        let prologue = prologue(room);
        let mut state = builder(keys, their_public, &prologue)?.build_initiator().map_err(|_| Error::Crypto)?;
        let mut out = vec![0u8; 256];
        let n = state.write_message(&[], &mut out).map_err(|_| Error::Crypto)?;
        out.truncate(n);
        Ok((Handshake(state), out))
    }

    /// The desktop's side: reads `hs1` and returns the ready channel and the answer (`hs2`).
    pub fn responder(keys: &Keys, their_public: &[u8; 32], room: &str, hs1: &[u8]) -> Result<(Channel, Vec<u8>), Error> {
        let prologue = prologue(room);
        let mut state = builder(keys, their_public, &prologue)?.build_responder().map_err(|_| Error::Crypto)?;
        let mut scratch = vec![0u8; 256];
        state.read_message(hs1, &mut scratch).map_err(|_| Error::Crypto)?;
        let mut out = vec![0u8; 256];
        let n = state.write_message(&[], &mut out).map_err(|_| Error::Crypto)?;
        out.truncate(n);
        let transport = state.into_transport_mode().map_err(|_| Error::Crypto)?;
        Ok((Channel { transport, partial: Vec::new() }, out))
    }

    /// Encrypts one message as one or more pieces, to send in this order.
    pub fn seal(&mut self, message: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
        if message.len() > MAX_MESSAGE {
            return Err(Error::TooBig);
        }
        let mut pieces = Vec::new();
        let mut chunks = message.chunks(PIECE).peekable();
        // An empty message is still one (empty) piece.
        let mut first = true;
        while first || chunks.peek().is_some() {
            first = false;
            let chunk = chunks.next().unwrap_or(&[]);
            let mut plain = Vec::with_capacity(chunk.len() + 1);
            plain.push(u8::from(chunks.peek().is_some())); // 1: more pieces follow
            plain.extend_from_slice(chunk);
            let mut out = vec![0u8; plain.len() + 16];
            let n = self.transport.write_message(&plain, &mut out).map_err(|_| Error::Crypto)?;
            out.truncate(n);
            pieces.push(out);
        }
        Ok(pieces)
    }

    /// Decrypts one piece: the whole message once its last piece is in, `None` while more are to come.
    pub fn open(&mut self, piece: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let mut plain = vec![0u8; piece.len()];
        let n = self.transport.read_message(piece, &mut plain).map_err(|_| Error::Crypto)?;
        let (more, body) = plain[..n].split_first().ok_or(Error::Crypto)?;
        if self.partial.len() + body.len() > MAX_MESSAGE {
            self.partial.clear();
            return Err(Error::TooBig);
        }
        self.partial.extend_from_slice(body);
        Ok(match more {
            0 => Some(std::mem::take(&mut self.partial)),
            _ => None,
        })
    }
}

impl Handshake {
    /// The phone reads the desktop's answer (`hs2`): the channel is ready.
    pub fn finish(mut self, hs2: &[u8]) -> Result<Channel, Error> {
        let mut scratch = vec![0u8; 256];
        self.0.read_message(hs2, &mut scratch).map_err(|_| Error::Crypto)?;
        let transport = self.0.into_transport_mode().map_err(|_| Error::Crypto)?;
        Ok(Channel { transport, partial: Vec::new() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: &str = "00112233445566778899aabbccddeeff";

    fn pair() -> (Channel, Channel) {
        let (desktop, phone) = (Keys::generate(), Keys::generate());
        let (hand, hs1) = Channel::initiator(&phone, &desktop.public, ROOM).unwrap();
        let (desk, hs2) = Channel::responder(&desktop, &phone.public, ROOM, &hs1).unwrap();
        (hand.finish(&hs2).unwrap(), desk)
    }

    fn carry(from: &mut Channel, to: &mut Channel, message: &[u8]) -> Vec<u8> {
        let pieces = from.seal(message).unwrap();
        let mut whole = None;
        for (i, piece) in pieces.iter().enumerate() {
            whole = to.open(piece).unwrap();
            assert_eq!(whole.is_some(), i == pieces.len() - 1, "only the last piece completes the message");
        }
        whole.unwrap()
    }

    #[test]
    fn paired_devices_talk_both_ways() {
        let (mut phone, mut desktop) = pair();
        assert_eq!(carry(&mut phone, &mut desktop, b"hola"), b"hola");
        assert_eq!(carry(&mut desktop, &mut phone, "¿qué tal?".as_bytes()), "¿qué tal?".as_bytes());
        assert_eq!(carry(&mut phone, &mut desktop, b""), b"");
        // What travels is not the text.
        let sealed = phone.seal(b"un secreto que no debe verse").unwrap();
        assert!(!sealed[0].windows(7).any(|w| w == b"secreto"));
    }

    #[test]
    fn a_long_message_travels_in_pieces() {
        let (mut phone, mut desktop) = pair();
        let long: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(phone.seal(&long).unwrap().len(), 5);
        assert_eq!(carry(&mut desktop, &mut phone, &long), long);
        // Exactly one piece's worth is one piece.
        assert_eq!(phone.seal(&long[..PIECE]).unwrap().len(), 1);
        assert_eq!(phone.seal(&vec![0u8; MAX_MESSAGE + 1]).err(), Some(Error::TooBig));
    }

    #[test]
    fn only_the_paired_keys_complete_the_handshake() {
        let (desktop, phone, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        // A stranger who knows the desktop's public key but is not the paired phone.
        let (_, hs1) = Channel::initiator(&stranger, &desktop.public, ROOM).unwrap();
        assert!(Channel::responder(&desktop, &phone.public, ROOM, &hs1).is_err());
        // The paired phone, but a handshake made for another room.
        let (_, hs1) = Channel::initiator(&phone, &desktop.public, "otra-sala").unwrap();
        assert!(Channel::responder(&desktop, &phone.public, ROOM, &hs1).is_err());
        // An answer that does not come from the desktop.
        let (hand, hs1) = Channel::initiator(&phone, &desktop.public, ROOM).unwrap();
        let (_, forged) = Channel::responder(&stranger, &phone.public, ROOM, &hs1).unwrap_or_else(|_| {
            let (other_hand, other_hs1) = Channel::initiator(&phone, &stranger.public, ROOM).unwrap();
            drop(other_hand);
            Channel::responder(&stranger, &phone.public, ROOM, &other_hs1).unwrap()
        });
        assert!(hand.finish(&forged).is_err());
    }

    #[test]
    fn a_repeated_altered_or_reordered_piece_is_refused() {
        let (mut phone, mut desktop) = pair();
        let first = phone.seal(b"uno").unwrap().remove(0);
        let second = phone.seal(b"dos").unwrap().remove(0);
        // Out of order.
        assert_eq!(desktop.open(&second).err(), Some(Error::Crypto));
        assert_eq!(desktop.open(&first).unwrap().unwrap(), b"uno");
        // Repeated.
        assert_eq!(desktop.open(&first).err(), Some(Error::Crypto));
        // Altered.
        let mut bent = second.clone();
        bent[3] ^= 1;
        assert_eq!(desktop.open(&bent).err(), Some(Error::Crypto));
        assert_eq!(desktop.open(&[]).err(), Some(Error::Crypto));
    }

    #[test]
    fn a_public_key_follows_from_its_private_key() {
        let keys = Keys::generate();
        assert_eq!(Keys::from_private(keys.private).public, keys.public);
        let other = Keys::generate();
        assert_ne!(keys.public, other.public);
        assert_eq!(keys.shared(&other.public), other.shared(&keys.public));
    }
}
