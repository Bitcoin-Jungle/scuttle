//! BIP 352 silent payment addresses.
//!
//! posd never pays to or scans a silent payment address: it only checks the
//! one an owner registers before handing it to payers, so a typo or an
//! address for another network is refused up front.

use bitcoin::Network;
use bitcoin::bech32::{Bech32m, Fe32};
use bitcoin::bech32::primitives::decode::CheckedHrpstring;
use bitcoin::secp256k1::PublicKey;


/// A version 0 address carries the scan key and the spend key.
const V0_PAYLOAD_LEN: usize = 66;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SilentPaymentError {
	#[error("not a bech32m string")]
	Encoding,
	#[error("a silent payment address on {0} starts with {1}1")]
	WrongNetwork(Network, &'static str),
	#[error("only version 0 silent payment addresses are supported")]
	Version,
	#[error("the address does not hold a scan key and a spend key")]
	Keys,
}

/// The human-readable part of addresses on `network`.
fn hrp(network: Network) -> &'static str {
	match network {
		Network::Bitcoin => "sp",
		_ => "tsp",
	}
}

/// Checks that `address` is a version 0 silent payment address for
/// `network` with two valid public keys. Returns it in lower case.
pub fn parse(address: &str, network: Network) -> Result<String, SilentPaymentError> {
	let mut checked = CheckedHrpstring::new::<Bech32m>(address).map_err(|_| SilentPaymentError::Encoding)?;
	let want = hrp(network);
	if checked.hrp().to_lowercase() != want {
		return Err(SilentPaymentError::WrongNetwork(network, want));
	}
	if checked.remove_witness_version() != Some(Fe32::Q) {
		return Err(SilentPaymentError::Version);
	}
	let payload = checked.byte_iter().collect::<Vec<u8>>();
	if payload.len() != V0_PAYLOAD_LEN {
		return Err(SilentPaymentError::Keys);
	}
	let (scan, spend) = payload.split_at(33);
	PublicKey::from_slice(scan).map_err(|_| SilentPaymentError::Keys)?;
	PublicKey::from_slice(spend).map_err(|_| SilentPaymentError::Keys)?;
	Ok(address.to_lowercase())
}

#[cfg(test)]
mod test {
	use super::*;

	use bitcoin::bech32::{ByteIterExt, Fe32IterExt, Hrp};
	use bitcoin::secp256k1::{Secp256k1, SecretKey};

	fn encode(hrp: &str, version: Fe32, payload: &[u8]) -> String {
		let hrp = Hrp::parse(hrp).unwrap();
		payload.iter().copied().bytes_to_fes()
			.with_checksum::<Bech32m>(&hrp)
			.with_witness_version(version)
			.chars().collect()
	}

	fn keys() -> Vec<u8> {
		let secp = Secp256k1::new();
		let scan = SecretKey::from_slice(&[1u8; 32]).unwrap().public_key(&secp);
		let spend = SecretKey::from_slice(&[2u8; 32]).unwrap().public_key(&secp);
		[scan.serialize(), spend.serialize()].concat()
	}

	#[test]
	fn accepts_v0_addresses() {
		let address = encode("sp", Fe32::Q, &keys());
		// Longer than the 90 characters of a segwit address.
		assert!(address.len() > 90);
		assert_eq!(parse(&address, Network::Bitcoin), Ok(address.clone()));
		assert_eq!(parse(&address.to_uppercase(), Network::Bitcoin), Ok(address));
		// From the BIP 352 test vectors.
		let bip = "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv";
		assert_eq!(parse(bip, Network::Bitcoin), Ok(bip.to_string()));
		let test = encode("tsp", Fe32::Q, &keys());
		assert_eq!(parse(&test, Network::Signet), Ok(test.clone()));
		assert_eq!(parse(&test, Network::Regtest), Ok(test));
	}

	#[test]
	fn refuses_other_addresses() {
		let address = encode("sp", Fe32::Q, &keys());
		assert_eq!(parse(&address, Network::Signet), Err(SilentPaymentError::WrongNetwork(Network::Signet, "tsp")));
		assert_eq!(parse(&encode("sp", Fe32::P, &keys()), Network::Bitcoin), Err(SilentPaymentError::Version));
		assert_eq!(parse(&encode("sp", Fe32::Q, &keys()[..33]), Network::Bitcoin), Err(SilentPaymentError::Keys));
		let mut bad_key = keys();
		bad_key[33] = 0x07;
		assert_eq!(parse(&encode("sp", Fe32::Q, &bad_key), Network::Bitcoin), Err(SilentPaymentError::Keys));
		let mut typo = address.clone().into_bytes();
		let last = typo.len() - 1;
		typo[last] = if typo[last] == b'q' { b'p' } else { b'q' };
		assert_eq!(parse(&String::from_utf8(typo).unwrap(), Network::Bitcoin), Err(SilentPaymentError::Encoding));
		assert_eq!(parse("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", Network::Bitcoin), Err(SilentPaymentError::Encoding));
	}
}
