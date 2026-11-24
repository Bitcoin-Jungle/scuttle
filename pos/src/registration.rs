//! The store registration that the owner's wallet signs.
//!
//! The wallet signs [Registration::message] with bark's message signing,
//! using the key of the Ark address that receives the payments. Any wallet
//! with that signing call can register a store: no wallet code is specific
//! to the point of sale.

use std::str::FromStr;

use bitcoin::secp256k1::{PublicKey, schnorr};
use serde::{Deserialize, Serialize};

use crate::brand::BrandKey;


/// The first line of the message. It changes when the format changes.
pub const REGISTRATION_HEADER: &str = "posd registration v1";

/// The fields of a store registration. The owner signs [Registration::message]
/// of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Registration {
	#[schema(example = "Corner Café")]
	pub name: String,
	/// The Ark address that receives the payments. Its key owns the store.
	#[schema(example = "ark1…")]
	pub ark_address: String,
	/// The public descriptor of the on-chain account the host app keeps.
	#[schema(example = "tr([fingerprint/86'/0'/1']xpub…/0/*)")]
	pub onchain_descriptor: String,
	/// The owner's BIP 352 silent payment address, which the store's
	/// Lightning address offers to payers that support it. Empty or absent
	/// for none.
	#[serde(default)]
	#[schema(example = "sp1q…")]
	pub silent_payment_address: String,
	/// A currency posd's price source quotes, such as `USD` or `CRC`.
	#[schema(example = "CRC")]
	pub currency: String,
	/// The key of the store's brand in posd's config: its terminals' look
	/// and the domain of its Lightning address and pairing links.
	pub brand: BrandKey,
	/// The name of the store's Lightning address on the brand's domain, if
	/// it wants one. Empty or absent for none.
	#[serde(default)]
	#[schema(example = "cornercafe", pattern = "^[a-z0-9._-]{0,64}$")]
	pub lightning_address_name: String,
	/// Seconds since the UNIX epoch, within 10 minutes of posd's clock.
	#[schema(example = 1795190400)]
	pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistrationError {
	#[error("field {0} is empty")]
	Empty(&'static str),
	#[error("field {0} contains a line break")]
	LineBreak(&'static str),
	#[error("not a valid Ark address")]
	BadAddress,
	#[error("the Ark address is for another Ark server")]
	ServerMismatch,
	#[error("the signature is not valid for the Ark address")]
	BadSignature,
}

impl Registration {
	/// The text the owner signs: the header, then one `field:value` line
	/// per field, joined by `\n`, with no trailing newline.
	pub fn message(&self) -> String {
		format!(
			"{}\nname:{}\nark_address:{}\nonchain_descriptor:{}\nsilent_payment_address:{}\n\
			currency:{}\nbrand:{}\nlightning_address_name:{}\ntimestamp:{}",
			REGISTRATION_HEADER, self.name, self.ark_address, self.onchain_descriptor,
			self.silent_payment_address, self.currency, self.brand, self.lightning_address_name,
			self.timestamp,
		)
	}

	/// Checks the fields, that the address is for the Ark server with
	/// `server_pubkey`, and the signature. The service checks the
	/// timestamp, the descriptor, the silent payment address and the
	/// currency itself.
	pub fn verify(
		&self,
		signature: &schnorr::Signature,
		server_pubkey: PublicKey,
	) -> Result<ark::Address, RegistrationError> {
		let fields = [
			("name", &self.name),
			("ark_address", &self.ark_address),
			("onchain_descriptor", &self.onchain_descriptor),
			("currency", &self.currency),
		];
		for (field, value) in fields {
			if value.is_empty() {
				return Err(RegistrationError::Empty(field));
			}
		}
		// A line break would let one field pose as another.
		let optional = [
			("silent_payment_address", &self.silent_payment_address),
			("lightning_address_name", &self.lightning_address_name),
		];
		for (field, value) in fields.into_iter().chain(optional) {
			if value.contains(['\n', '\r']) {
				return Err(RegistrationError::LineBreak(field));
			}
		}
		let address = ark::Address::from_str(&self.ark_address)
			.map_err(|_| RegistrationError::BadAddress)?;
		if !address.is_for_server(server_pubkey) {
			return Err(RegistrationError::ServerMismatch);
		}
		if !ark::message::verify_for_address(&address, self.message().as_bytes(), signature) {
			return Err(RegistrationError::BadSignature);
		}
		Ok(address)
	}
}

#[cfg(test)]
mod test {
	use super::*;

	use bitcoin::secp256k1::{Secp256k1, SecretKey};

	fn registration() -> Registration {
		Registration {
			name: "Corner Café".into(),
			ark_address: "tark1placeholder".into(),
			onchain_descriptor: "tr(xpub/0/*)".into(),
			silent_payment_address: "sp1qplaceholder".into(),
			currency: "CRC".into(),
			brand: "example".parse().unwrap(),
			lightning_address_name: "cornercafe".into(),
			timestamp: 1795190400,
		}
	}

	#[test]
	fn message_format() {
		assert_eq!(registration().message(),
			"posd registration v1\nname:Corner Café\nark_address:tark1placeholder\n\
			onchain_descriptor:tr(xpub/0/*)\nsilent_payment_address:sp1qplaceholder\n\
			currency:CRC\nbrand:example\nlightning_address_name:cornercafe\ntimestamp:1795190400",
		);
		let none = Registration {
			silent_payment_address: String::new(),
			lightning_address_name: String::new(),
			..registration()
		};
		assert!(none.message().contains("\nsilent_payment_address:\ncurrency:"));
		assert!(none.message().contains("\nlightning_address_name:\ntimestamp:"));
	}

	#[test]
	fn refuses_bad_fields() {
		let server = SecretKey::from_slice(&[3u8; 32]).unwrap().public_key(&Secp256k1::new());
		let sig = schnorr::Signature::from_slice(&[1u8; 64]).unwrap();
		let mut reg = registration();
		reg.name = "a\ncurrency:x".into();
		assert_eq!(reg.verify(&sig, server), Err(RegistrationError::LineBreak("name")));
		let mut reg = registration();
		reg.silent_payment_address = "sp1q\ncurrency:x".into();
		assert_eq!(reg.verify(&sig, server), Err(RegistrationError::LineBreak("silent_payment_address")));
		let mut reg = registration();
		reg.currency = String::new();
		assert_eq!(reg.verify(&sig, server), Err(RegistrationError::Empty("currency")));
		assert_eq!(registration().verify(&sig, server), Err(RegistrationError::BadAddress));
	}
}
