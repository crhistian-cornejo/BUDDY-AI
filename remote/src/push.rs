//! Notices sealed for the phone while it is away: they travel through the relay and Apple, who see only these bytes.
//!
//! There is no live channel then, so they use a key both ends derive once, at pairing: HKDF over the two devices'
//! shared secret, salted with the pairing secret. Each notice carries a counter and its time, so one that is
//! replayed or held back for days is dropped (`open`).

use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305};
use hkdf::Hkdf;
use serde_json::{Value, json};
use sha2::Sha256;

use crate::{Error, Keys, random};

/// A notice older than this is not shown.
pub const MAX_AGE_SECS: i64 = 48 * 3600;
const NONCE: usize = 24;

/// The notices' key: the same on the desktop and on the phone.
pub fn key(mine: &Keys, their_public: &[u8; 32], pairing_secret: &[u8; 32], room: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(pairing_secret), &mine.shared(their_public))
        .expand(format!("buddy-push-v1:{room}").as_bytes(), &mut out)
        .expect("32 bytes is a valid HKDF output length");
    out
}

/// Seals `payload` as notice number `counter`, made at `now` (unix seconds).
pub fn seal(key: &[u8; 32], counter: u64, now: i64, payload: &Value) -> Vec<u8> {
    let nonce = random::<NONCE>();
    let plain = json!({ "c": counter, "t": now, "p": payload }).to_string();
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt((&nonce).into(), plain.as_bytes()).expect("encrypting in memory"));
    out
}

/// Opens a notice: its counter and payload. `last_counter` is the highest one already shown (0: none yet).
pub fn open(key: &[u8; 32], sealed: &[u8], last_counter: u64, now: i64) -> Result<(u64, Value), Error> {
    if sealed.len() < NONCE + 16 {
        return Err(Error::Malformed("aviso"));
    }
    let (nonce, body) = sealed.split_at(NONCE);
    let nonce: [u8; NONCE] = nonce.try_into().map_err(|_| Error::Malformed("aviso"))?;
    let plain = XChaCha20Poly1305::new(key.into()).decrypt((&nonce).into(), body).map_err(|_| Error::Crypto)?;
    let value: Value = serde_json::from_slice(&plain).map_err(|_| Error::Malformed("aviso"))?;
    let (Some(counter), Some(made)) = (value["c"].as_u64(), value["t"].as_i64()) else { return Err(Error::Malformed("aviso")) };
    if counter <= last_counter || now - made > MAX_AGE_SECS {
        return Err(Error::Stale);
    }
    Ok((counter, value["p"].clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: &str = "sala";

    fn keys() -> ([u8; 32], [u8; 32]) {
        let (desktop, phone) = (Keys::generate(), Keys::generate());
        let secret = [3u8; 32];
        (key(&desktop, &phone.public, &secret, ROOM), key(&phone, &desktop.public, &secret, ROOM))
    }

    #[test]
    fn both_ends_derive_the_same_key_and_nobody_else_does() {
        let (desktop, phone) = (Keys::generate(), Keys::generate());
        let secret = [3u8; 32];
        let ours = key(&desktop, &phone.public, &secret, ROOM);
        assert_eq!(ours, key(&phone, &desktop.public, &secret, ROOM));
        assert_ne!(ours, key(&desktop, &phone.public, &[4u8; 32], ROOM), "another pairing secret");
        assert_ne!(ours, key(&desktop, &phone.public, &secret, "otra"), "another room");
        assert_ne!(ours, key(&Keys::generate(), &phone.public, &secret, ROOM), "another desktop");
    }

    #[test]
    fn a_notice_opens_on_the_phone_and_hides_its_text_on_the_way() {
        let (desktop, phone) = keys();
        let sealed = seal(&desktop, 1, 1_000, &json!({ "title": "Buddy respondió", "body": "El informe está listo" }));
        assert!(!sealed.windows(7).any(|w| w == b"informe"));
        let (counter, payload) = open(&phone, &sealed, 0, 1_060).unwrap();
        assert_eq!((counter, payload["title"].as_str()), (1, Some("Buddy respondió")));
        // Two seals of the same notice differ (fresh nonce).
        assert_ne!(sealed, seal(&desktop, 1, 1_000, &json!({ "title": "Buddy respondió", "body": "El informe está listo" })));
    }

    #[test]
    fn a_replayed_late_or_altered_notice_is_dropped() {
        let (desktop, phone) = keys();
        let sealed = seal(&desktop, 5, 1_000, &json!({}));
        assert_eq!(open(&phone, &sealed, 5, 1_060).err(), Some(Error::Stale), "already shown");
        assert_eq!(open(&phone, &sealed, 9, 1_060).err(), Some(Error::Stale), "older than the last one");
        assert_eq!(open(&phone, &sealed, 0, 1_000 + MAX_AGE_SECS + 1).err(), Some(Error::Stale), "held back too long");
        let mut bent = sealed.clone();
        *bent.last_mut().unwrap() ^= 1;
        assert_eq!(open(&phone, &bent, 0, 1_060).err(), Some(Error::Crypto));
        assert_eq!(open(&[9u8; 32], &sealed, 0, 1_060).err(), Some(Error::Crypto), "another key");
        assert_eq!(open(&phone, &sealed[..20], 0, 1_060).err(), Some(Error::Malformed("aviso")));
    }
}
