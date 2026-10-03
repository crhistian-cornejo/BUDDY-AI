//! Buddy's phone link, the part both ends share: pairing by QR (`pairing`), the end-to-end encrypted channel
//! (`channel`, Noise KK), notices sealed for delivery through Apple while the phone is away (`push`), and the shapes
//! on the wire (`wire`). No I/O and no clock: the core and the phone app bring the sockets and the time.
//!
//! The relay between the two ends only forwards these bytes: it can delay or drop them, never read or forge them.

pub mod channel;
pub mod pairing;
pub mod push;
pub mod wire;

pub use channel::{Channel, Handshake, Keys};


use base64::Engine;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A key, a frame or an envelope that is not what it must be.
    #[error("datos no válidos: {0}")]
    Malformed(&'static str),
    /// The other end does not hold the paired key, or the bytes were altered, repeated or out of order.
    #[error("no se pudo descifrar")]
    Crypto,
    /// A sealed notice older than allowed, or one already seen.
    #[error("aviso repetido o caducado")]
    Stale,
    #[error("demasiado grande")]
    TooBig,
}

/// `N` random bytes from the system.
pub fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the system's random source");
    bytes
}

/// Base64 for links and JSON fields (URL-safe, no padding).
pub fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn decode(text: &str) -> Result<Vec<u8>, Error> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text).map_err(|_| Error::Malformed("base64"))
}

pub(crate) fn key32(text: &str) -> Result<[u8; 32], Error> {
    decode(text)?.try_into().map_err(|_| Error::Malformed("clave"))
}
