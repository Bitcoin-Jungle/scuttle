//! Owner login challenges.
//!
//! A challenge is stateless: posd authenticates it with an HMAC under a key
//! of its own instead of storing it, so asking for challenges costs no
//! storage. The owner signs the whole challenge, MAC line included. posd
//! records a challenge's nonce only when a login with it succeeds, which
//! needs a store owner's signature, to refuse it a second time.

use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
use bitcoin::hex::{DisplayHex, FromHex};
use bitcoin::secp256k1::rand::{RngCore, thread_rng};
use chrono::{DateTime, Utc};

use crate::token;


/// The first line of a challenge. It changes when the format changes.
pub const CHALLENGE_HEADER: &str = "posd login v1";

const MAC_LINE: &str = "\nmac:";

/// The key that authenticates challenges. It lives in the database, so
/// challenges outlive a restart and work across posd instances.
#[derive(Clone)]
pub struct LoginKey([u8; 32]);

impl LoginKey {
	pub fn random() -> LoginKey {
		let mut key = [0u8; 32];
		thread_rng().fill_bytes(&mut key);
		LoginKey(key)
	}

	pub fn from_slice(key: &[u8]) -> Option<LoginKey> {
		Some(LoginKey(key.try_into().ok()?))
	}

	pub fn as_bytes(&self) -> &[u8] {
		&self.0
	}

	fn mac(&self, body: &str) -> [u8; 32] {
		let mut engine = HmacEngine::<sha256::Hash>::new(&self.0);
		engine.input(body.as_bytes());
		Hmac::<sha256::Hash>::from_engine(engine).to_byte_array()
	}

	/// A new challenge for `ark_address`, valid until `expires_at`.
	pub fn challenge(&self, ark_address: &str, expires_at: DateTime<Utc>) -> String {
		let body = format!(
			"{}\nark_address:{}\nnonce:{}\nexpires_at:{}",
			CHALLENGE_HEADER, ark_address, token::random_hex(16), expires_at.timestamp(),
		);
		format!("{}{}{}", body, MAC_LINE, self.mac(&body).as_hex())
	}

	/// Checks that posd made `challenge` for `ark_address` and that it has
	/// not expired at `now`. Returns its nonce and expiry.
	pub fn verify(
		&self,
		challenge: &str,
		ark_address: &str,
		now: DateTime<Utc>,
	) -> Result<Verified, ChallengeError> {
		let (body, mac) = challenge.rsplit_once(MAC_LINE).ok_or(ChallengeError::Invalid)?;
		let mac = <[u8; 32]>::from_hex(mac).map_err(|_| ChallengeError::Invalid)?;
		// Compare every byte, so the time taken says nothing about a guess.
		let diff = self.mac(body).iter().zip(mac.iter()).fold(0u8, |d, (a, b)| d | (a ^ b));
		if diff != 0 {
			return Err(ChallengeError::Invalid);
		}

		let mut lines = body.split('\n');
		if lines.next() != Some(CHALLENGE_HEADER) {
			return Err(ChallengeError::Invalid);
		}
		let mut field = |name: &str| lines.next()
			.and_then(|l| l.strip_prefix(name))
			.and_then(|l| l.strip_prefix(':'))
			.ok_or(ChallengeError::Invalid);
		if field("ark_address")? != ark_address {
			return Err(ChallengeError::Invalid);
		}
		let nonce = field("nonce")?.to_string();
		let expires_at = field("expires_at")?.parse::<i64>().ok()
			.and_then(|t| DateTime::from_timestamp(t, 0))
			.ok_or(ChallengeError::Invalid)?;
		if now >= expires_at {
			return Err(ChallengeError::Expired);
		}
		Ok(Verified { nonce, expires_at })
	}
}

/// A challenge posd made and that has not expired.
#[derive(Debug, PartialEq, Eq)]
pub struct Verified {
	pub nonce: String,
	pub expires_at: DateTime<Utc>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChallengeError {
	#[error("the challenge is not one posd made for this address")]
	Invalid,
	#[error("the challenge has expired")]
	Expired,
}

#[cfg(test)]
mod test {
	use chrono::Duration;

	use super::*;

	#[test]
	fn challenges() {
		let key = LoginKey::random();
		let now = Utc::now();
		let expires_at = now + Duration::minutes(5);
		let c = key.challenge("ark1abc", expires_at);
		assert!(c.starts_with("posd login v1\nark_address:ark1abc\nnonce:"));

		let v = key.verify(&c, "ark1abc", now).unwrap();
		assert_eq!(v.expires_at.timestamp(), expires_at.timestamp());
		assert_eq!(v.nonce.len(), 32);
		assert_ne!(key.challenge("ark1abc", expires_at), c, "each challenge has its own nonce");

		// Another address, another key, a changed line or a late login.
		assert_eq!(key.verify(&c, "ark1xyz", now), Err(ChallengeError::Invalid));
		assert_eq!(LoginKey::random().verify(&c, "ark1abc", now), Err(ChallengeError::Invalid));
		let later = c.replace(&format!("expires_at:{}", expires_at.timestamp()), "expires_at:9999999999");
		assert_eq!(key.verify(&later, "ark1abc", now), Err(ChallengeError::Invalid));
		assert_eq!(key.verify(&c, "ark1abc", expires_at), Err(ChallengeError::Expired));
		assert_eq!(key.verify("posd login v1", "ark1abc", now), Err(ChallengeError::Invalid));
	}
}
