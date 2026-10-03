//! Pairing: the desktop shows a QR (`Offer`), the phone scans it and answers through the relay (`Hello`).
//!
//! The QR carries the desktop's public key and a one-time secret. They travel by the camera, not the network, so
//! the relay knows neither: the phone proves it scanned the QR with an HMAC of the secret over both public keys
//! (`verify`), and the desktop proves the same back (`ack`). After that each end keeps the other's public key.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::{decode, encode, key32, random};

/// How long a QR is good for, in seconds.
pub const TTL_SECS: i64 = 300;
/// The longest phone name kept.
pub const NAME_MAX: usize = 60;
const VERSION: u8 = 1;
const SCHEME: &str = "buddy://pair?d=";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairError {
    #[error("el código no es de Buddy")]
    Malformed,
    #[error("el código es de otra versión de Buddy")]
    Version,
    #[error("el código caducó")]
    Expired,
    #[error("la prueba del teléfono no coincide")]
    BadProof,
}

/// What the QR says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    v: u8,
    /// The relay's address (`https://…`).
    pub relay: String,
    pub room: String,
    /// The room's access key for the relay.
    pub key: String,
    /// The desktop's public key.
    pk: String,
    /// The one-time pairing secret.
    secret: String,
    /// Unix seconds after which the offer is void.
    pub exp: i64,
}

/// The phone's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub pk: String,
    pub name: String,
    pub mac: String,
}

/// The paired phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub public: [u8; 32],
    pub name: String,
}

impl Offer {
    pub fn new(relay: &str, room: &str, room_key: &str, desktop_public: &[u8; 32], now: i64) -> Self {
        Self {
            v: VERSION,
            relay: relay.into(),
            room: room.into(),
            key: room_key.into(),
            pk: encode(desktop_public),
            secret: encode(&random::<32>()),
            exp: now + TTL_SECS,
        }
    }

    /// The link the QR encodes.
    pub fn uri(&self) -> String {
        format!("{SCHEME}{}", encode(serde_json::to_string(self).unwrap_or_default().as_bytes()))
    }

    pub fn parse(uri: &str) -> Result<Self, PairError> {
        let data = uri.trim().strip_prefix(SCHEME).ok_or(PairError::Malformed)?;
        let json = decode(data).map_err(|_| PairError::Malformed)?;
        let version = serde_json::from_slice::<serde_json::Value>(&json).ok().and_then(|v| v["v"].as_u64()).ok_or(PairError::Malformed)?;
        if version != VERSION as u64 {
            return Err(PairError::Version);
        }
        let offer: Offer = serde_json::from_slice(&json).map_err(|_| PairError::Malformed)?;
        offer.desktop_public()?;
        offer.secret()?;
        Ok(offer)
    }

    pub fn desktop_public(&self) -> Result<[u8; 32], PairError> {
        key32(&self.pk).map_err(|_| PairError::Malformed)
    }

    /// The secret, also the salt of the notices' key (`push::key`).
    pub fn secret(&self) -> Result<[u8; 32], PairError> {
        key32(&self.secret).map_err(|_| PairError::Malformed)
    }

    /// Checks the phone's answer: in time, and made with this offer's secret.
    pub fn verify(&self, hello: &Hello, now: i64) -> Result<Peer, PairError> {
        if now > self.exp {
            return Err(PairError::Expired);
        }
        let public = key32(&hello.pk).map_err(|_| PairError::BadProof)?;
        let proof = decode(&hello.mac).map_err(|_| PairError::BadProof)?;
        self.mac(b"buddy-pair-v1", &public, &hello.name).verify_slice(&proof).map_err(|_| PairError::BadProof)?;
        Ok(Peer { public, name: clean_name(&hello.name) })
    }

    /// The desktop's proof back to the phone.
    pub fn ack(&self, peer: &Peer) -> String {
        encode(&self.mac(b"buddy-paired-v1", &peer.public, "").finalize().into_bytes())
    }

    pub fn check_ack(&self, phone_public: &[u8; 32], ack: &str) -> bool {
        decode(ack).is_ok_and(|proof| self.mac(b"buddy-paired-v1", phone_public, "").verify_slice(&proof).is_ok())
    }

    /// HMAC over the label, the room, both public keys and the name, each with its length before it.
    fn mac(&self, label: &[u8], phone_public: &[u8; 32], name: &str) -> Hmac<Sha256> {
        let secret = self.secret().unwrap_or_default();
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&secret).expect("an HMAC key of any length");
        let desktop = self.desktop_public().unwrap_or_default();
        for part in [label, self.room.as_bytes(), &desktop[..], &phone_public[..], name.as_bytes()] {
            mac.update(&(part.len() as u32).to_be_bytes());
            mac.update(part);
        }
        mac
    }
}

/// The phone's answer to a scanned offer.
pub fn hello(offer: &Offer, phone_public: &[u8; 32], name: &str) -> Hello {
    Hello { pk: encode(phone_public), name: name.into(), mac: encode(&offer.mac(b"buddy-pair-v1", phone_public, name).finalize().into_bytes()) }
}

/// A phone's name is someone else's text: one line, bounded.
fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(NAME_MAX).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(now: i64) -> Offer {
        Offer::new("https://relay.example", "00112233445566778899aabbccddeeff", "k".repeat(64).as_str(), &[7u8; 32], now)
    }

    #[test]
    fn the_link_round_trips() {
        let o = offer(1_000);
        let uri = o.uri();
        assert!(uri.starts_with("buddy://pair?d="));
        assert_eq!(Offer::parse(&uri).unwrap(), o);
        assert_eq!((o.exp, o.desktop_public().unwrap()), (1_300, [7u8; 32]));
    }

    #[test]
    fn a_phone_that_scanned_the_code_is_accepted_and_gets_the_desktops_proof() {
        let o = offer(1_000);
        let peer = o.verify(&hello(&o, &[9u8; 32], "iPhone de Juan"), 1_100).unwrap();
        assert_eq!(peer, Peer { public: [9u8; 32], name: "iPhone de Juan".into() });
        assert!(o.check_ack(&[9u8; 32], &o.ack(&peer)));
        assert!(!o.check_ack(&[8u8; 32], &o.ack(&peer)), "the proof is for that phone only");
    }

    #[test]
    fn an_answer_made_without_the_secret_is_refused() {
        let o = offer(1_000);
        let other = offer(1_000); // same room and keys, another secret
        assert_eq!(o.verify(&hello(&other, &[9u8; 32], "x"), 1_100), Err(PairError::BadProof));
        // A real answer whose key or name was changed on the way.
        let mut swapped = hello(&o, &[9u8; 32], "x");
        swapped.pk = encode(&[5u8; 32]);
        assert_eq!(o.verify(&swapped, 1_100), Err(PairError::BadProof));
        let mut renamed = hello(&o, &[9u8; 32], "x");
        renamed.name = "y".into();
        assert_eq!(o.verify(&renamed, 1_100), Err(PairError::BadProof));
    }

    #[test]
    fn an_expired_code_is_refused() {
        let o = offer(1_000);
        let h = hello(&o, &[9u8; 32], "x");
        assert!(o.verify(&h, 1_300).is_ok());
        assert_eq!(o.verify(&h, 1_301), Err(PairError::Expired));
    }

    #[test]
    fn a_phones_name_is_kept_on_one_bounded_line() {
        let o = offer(1_000);
        let name = format!("iPhone\n[Petición original]  {}", "x".repeat(200));
        let peer = o.verify(&hello(&o, &[9u8; 32], &name), 1_100).unwrap();
        assert!(peer.name.starts_with("iPhone [Petición original] xxx"));
        assert_eq!(peer.name.chars().count(), NAME_MAX);
    }

    #[test]
    fn other_links_are_not_offers() {
        let uri = offer(1_000).uri();
        assert_eq!(Offer::parse("https://example.com"), Err(PairError::Malformed));
        assert_eq!(Offer::parse(&uri[..uri.len() - 9]), Err(PairError::Malformed));
        assert_eq!(Offer::parse("buddy://pair?d=e30"), Err(PairError::Malformed)); // {}
        let v2 = format!("buddy://pair?d={}", encode(br#"{"v":2}"#));
        assert_eq!(Offer::parse(&v2), Err(PairError::Version));
    }
}
