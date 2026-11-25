//! Bolt Card payments.
//!
//! A Bolt Card holds an LNURL-withdraw link (LUD-03), which changes on every
//! tap. The terminal reads it with NFC and gives it to posd, and posd asks
//! the card's service to pay the sale's Lightning invoice. The payment then
//! arrives like any other: the watcher sees it and the sale is paid.
//!
//! posd, not the terminal, talks to the card's service: the browser could
//! only reach services that allow it with CORS, and posd decides which
//! invoice the card pays. As posd fetches URLs a terminal gives it, it
//! reaches only public addresses by https, unless the config allows private
//! ones for a test network, and follows no redirects.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use anyhow::{Context, anyhow};
use bitcoin::bech32;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde::Deserialize;
use serde_json::Value;
use url::Url;


/// The largest answer posd reads from a card's service.
const MAX_RESPONSE_LEN: usize = 64 * 1024;

/// The longest reason from a card's service posd passes on.
const MAX_REASON_LEN: usize = 200;

/// Why a card did not pay.
#[derive(Debug, thiserror::Error)]
pub enum CardError {
	/// What the terminal read is not a withdraw link posd may follow.
	#[error("{0}")]
	BadLink(String),
	/// The card's service said no, or the amount is outside the card's
	/// limits. The reason is the service's, for the cashier.
	#[error("{0}")]
	Declined(String),
	/// The card's service could not be reached or answered nonsense.
	#[error(transparent)]
	Unreachable(anyhow::Error),
}

/// The withdraw request a card's link gives, as LUD-03 has it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WithdrawRequest {
	callback: String,
	k1: String,
	/// In millisatoshis.
	min_withdrawable: u64,
	/// In millisatoshis.
	max_withdrawable: u64,
}

pub struct BoltCards {
	http: reqwest::Client,
	allow_private: bool,
}

impl BoltCards {
	/// With `allow_private`, card services may also be on private and local
	/// addresses and plain http. For test networks only.
	pub fn new(allow_private: bool) -> anyhow::Result<BoltCards> {
		let http = reqwest::Client::builder()
			.timeout(Duration::from_secs(10))
			.redirect(reqwest::redirect::Policy::none())
			.no_proxy()
			.dns_resolver(PublicResolver { allow_private })
			.build()?;
		Ok(BoltCards { http, allow_private })
	}

	/// Has the card whose link the terminal read pay `invoice` for
	/// `amount_sat`. The card's service pays after it answers, so the
	/// payment arrives a moment later.
	pub async fn pay(&self, link: &str, invoice: &str, amount_sat: u64) -> Result<(), CardError> {
		let url = decode_link(link)?;
		self.check_url(&url).map_err(CardError::BadLink)?;
		let answer = self.get(url).await?;
		let request = withdraw_request(answer)?;

		let amount_msat = amount_sat.saturating_mul(1000);
		if amount_msat < request.min_withdrawable || amount_msat > request.max_withdrawable {
			return Err(CardError::Declined(format!(
				"the card pays {} to {} sats at a time",
				request.min_withdrawable.div_ceil(1000), request.max_withdrawable / 1000,
			)));
		}

		let mut callback = Url::parse(&request.callback)
			.map_err(|_| CardError::Unreachable(anyhow!("the card's callback is not a URL")))?;
		self.check_url(&callback).map_err(|e| CardError::Unreachable(anyhow!("card callback: {}", e)))?;
		callback.query_pairs_mut().append_pair("k1", &request.k1).append_pair("pr", invoice);
		let answer = self.get(callback).await?;
		match answer.get("status").and_then(Value::as_str) {
			Some(s) if s.eq_ignore_ascii_case("OK") => Ok(()),
			_ => Err(CardError::Declined(reason(&answer))),
		}
	}

	/// Whether posd may fetch `url`. Host names are checked again when they
	/// resolve, by [PublicResolver].
	fn check_url(&self, url: &Url) -> Result<(), String> {
		match url.scheme() {
			"https" => {},
			"http" if self.allow_private => {},
			_ => return Err("the card's link is not https".into()),
		}
		if !url.username().is_empty() || url.password().is_some() {
			return Err("the card's link has a user name".into());
		}
		let ip = match url.host() {
			Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip),
			Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip),
			Some(url::Host::Domain(_)) => return Ok(()),
			None => return Err("the card's link has no host".into()),
		};
		if !self.allow_private && !is_public(ip) {
			return Err("the card's link is not on a public address".into());
		}
		Ok(())
	}

	/// The JSON answer of a card's service.
	async fn get(&self, url: Url) -> Result<Value, CardError> {
		let host = url.host_str().unwrap_or_default().to_string();
		let mut res = self.http.get(url).header("Accept", "application/json").send().await
			.with_context(|| format!("card service {}", host))
			.map_err(CardError::Unreachable)?;
		let status = res.status();
		let mut body = Vec::new();
		while let Some(chunk) = res.chunk().await
			.with_context(|| format!("card service {}", host))
			.map_err(CardError::Unreachable)?
		{
			if body.len() + chunk.len() > MAX_RESPONSE_LEN {
				return Err(CardError::Unreachable(anyhow!("card service {} answered too much", host)));
			}
			body.extend_from_slice(&chunk);
		}
		// Services answer a LUD-06 error with a 4xx or 5xx status too: its
		// reason is for the cashier.
		match serde_json::from_slice::<Value>(&body) {
			Ok(v) if v.is_object() => Ok(v),
			_ => Err(CardError::Unreachable(anyhow!(
				"card service {} answered {} without JSON", host, status,
			))),
		}
	}
}

/// The withdraw request in a service's answer, or its reason for none.
fn withdraw_request(answer: Value) -> Result<WithdrawRequest, CardError> {
	if answer.get("status").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("ERROR")) {
		return Err(CardError::Declined(reason(&answer)));
	}
	if answer.get("tag").and_then(Value::as_str) != Some("withdrawRequest") {
		return Err(CardError::BadLink("the card's link is not a withdraw link".into()));
	}
	serde_json::from_value(answer)
		.map_err(|e| CardError::Unreachable(anyhow!("card withdraw request: {}", e)))
}

/// A service's reason for an error, as it gave it.
fn reason(answer: &Value) -> String {
	let reason = answer.get("reason").and_then(Value::as_str)
		.map(|r| r.trim()).filter(|r| !r.is_empty())
		.unwrap_or("the card refused the payment");
	reason.chars().filter(|c| !c.is_control()).take(MAX_REASON_LEN).collect()
}

/// The URL in what a terminal read from a card: an `lnurlw://` link, as
/// Bolt Cards hold, a bech32 `LNURL1…`, either after `lightning:`, or a
/// plain URL.
pub fn decode_link(raw: &str) -> Result<Url, CardError> {
	let bad = |msg: &str| CardError::BadLink(msg.into());
	let mut link = raw.trim();
	if link.len() >= 10 && link[..10].eq_ignore_ascii_case("lightning:") {
		link = &link[10..];
	}
	let text = if link.len() >= 6 && link[..6].eq_ignore_ascii_case("lnurl1") {
		let (hrp, data) = bech32::decode(link).map_err(|_| bad("the card's LNURL is not valid bech32"))?;
		if !hrp.as_str().eq_ignore_ascii_case("lnurl") {
			return Err(bad("the card's LNURL is not valid bech32"));
		}
		String::from_utf8(data).map_err(|_| bad("the card's LNURL is not a URL"))?
	} else if let Some((scheme, rest)) = link.split_once("://") {
		if scheme.eq_ignore_ascii_case("lnurlw") {
			// LUD-17: lnurlw:// is https://, or http:// for an onion.
			let host = rest.split(['/', '?', '#']).next().unwrap_or("");
			let scheme = if host.ends_with(".onion") { "http" } else { "https" };
			format!("{}://{}", scheme, rest)
		} else {
			link.to_string()
		}
	} else {
		return Err(bad("the card does not hold a Lightning withdraw link"));
	};
	Url::parse(&text).map_err(|_| bad("the card's link is not a URL"))
}

/// Resolves host names to public addresses only, unless private ones are
/// allowed, so that a card's link cannot point posd at its own network.
struct PublicResolver {
	allow_private: bool,
}

impl Resolve for PublicResolver {
	fn resolve(&self, name: Name) -> Resolving {
		let allow_private = self.allow_private;
		let host = name.as_str().to_string();
		Box::pin(async move {
			let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?
				.filter(|a| allow_private || is_public(a.ip()))
				.collect::<Vec<SocketAddr>>();
			if addrs.is_empty() {
				return Err(format!("{} has no public address", host).into());
			}
			Ok(Box::new(addrs.into_iter()) as Addrs)
		})
	}
}

/// Whether `ip` is on the public internet.
pub fn is_public(ip: IpAddr) -> bool {
	match ip {
		IpAddr::V4(ip) => is_public_v4(ip),
		IpAddr::V6(ip) => {
			if let Some(v4) = ip.to_ipv4_mapped() {
				return is_public_v4(v4);
			}
			let s = ip.segments();
			!(ip.is_unspecified() || ip.is_loopback() || ip.is_multicast()
				// Unique local fc00::/7 and link-local fe80::/10.
				|| (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
				// Documentation 2001:db8::/32.
				|| (s[0] == 0x2001 && s[1] == 0x0db8)
				// IPv4-compatible and NAT64 well-known prefix.
				|| (s[0..6] == [0; 6]) || (s[0] == 0x64 && s[1] == 0xff9b))
		},
	}
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
	let o = ip.octets();
	!(ip.is_unspecified() || ip.is_loopback() || ip.is_private() || ip.is_link_local()
		|| ip.is_broadcast() || ip.is_documentation() || ip.is_multicast()
		|| o[0] == 0
		// Shared address space 100.64.0.0/10.
		|| (o[0] == 100 && (o[1] & 0xc0) == 64)
		// IETF protocol assignments 192.0.0.0/24 and benchmarking 198.18.0.0/15.
		|| (o[0] == 192 && o[1] == 0 && o[2] == 0)
		|| (o[0] == 198 && (o[1] & 0xfe) == 18)
		// Reserved 240.0.0.0/4.
		|| o[0] >= 240)
}

#[cfg(test)]
mod test {
	use super::*;
	use serde_json::json;

	#[test]
	fn decodes_links() {
		let url = decode_link("lnurlw://card.example.com/ln?p=AB&c=CD").unwrap();
		assert_eq!(url.as_str(), "https://card.example.com/ln?p=AB&c=CD");
		let url = decode_link("LNURLW://abc.onion/ln?p=1").unwrap();
		assert_eq!(url.scheme(), "http");
		assert_eq!(decode_link(" https://card.example.com/w ").unwrap().as_str(), "https://card.example.com/w");

		// LUD-01's example.
		let lnurl = "LNURL1DP68GURN8GHJ7UM9WFMXJCM99E3K7MF0V9CXJ0M385EKVCENXC6R2C35XVUKXEFCV5MKVV34X5EKZD3EV56NYD3HXQURZEPEXEJXXEPNXSCRVWFNV9NXZCN9XQ6XYEFHVGCXXCMYXYMNSERXFQ5FNS";
		let url = decode_link(lnurl).unwrap();
		assert_eq!(url.host_str(), Some("service.com"));
		assert_eq!(decode_link(&format!("lightning:{}", lnurl.to_lowercase())).unwrap(), url);

		assert!(matches!(decode_link("bitcoin:bc1qxyz"), Err(CardError::BadLink(_))));
		assert!(matches!(decode_link("LNURL1QQQQ"), Err(CardError::BadLink(_))));
	}

	#[test]
	fn public_addresses() {
		for ip in ["1.1.1.1", "8.8.8.8", "2606:4700::1111"] {
			assert!(is_public(ip.parse().unwrap()), "{}", ip);
		}
		for ip in [
			"127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1",
			"0.0.0.0", "255.255.255.255", "::1", "::", "fd00::1", "fe80::1", "::ffff:127.0.0.1",
			"::ffff:10.0.0.1", "64:ff9b::a00:1",
		] {
			assert!(!is_public(ip.parse().unwrap()), "{}", ip);
		}
	}

	#[test]
	fn checks_urls() {
		let cards = BoltCards::new(false).unwrap();
		assert!(cards.check_url(&Url::parse("https://card.example.com/w").unwrap()).is_ok());
		assert!(cards.check_url(&Url::parse("http://card.example.com/w").unwrap()).is_err());
		assert!(cards.check_url(&Url::parse("https://127.0.0.1/w").unwrap()).is_err());
		assert!(cards.check_url(&Url::parse("https://[::1]/w").unwrap()).is_err());
		assert!(cards.check_url(&Url::parse("https://u:p@card.example.com/w").unwrap()).is_err());
		let test = BoltCards::new(true).unwrap();
		assert!(test.check_url(&Url::parse("http://127.0.0.1:8080/w").unwrap()).is_ok());
	}

	#[test]
	fn reads_withdraw_requests() {
		let ok = withdraw_request(json!({
			"tag": "withdrawRequest", "callback": "https://card.example.com/cb", "k1": "abc",
			"minWithdrawable": 1000, "maxWithdrawable": 100_000_000, "defaultDescription": "",
		})).unwrap();
		assert_eq!(ok.max_withdrawable, 100_000_000);
		match withdraw_request(json!({ "status": "ERROR", "reason": "card is disabled" })) {
			Err(CardError::Declined(r)) => assert_eq!(r, "card is disabled"),
			other => panic!("{:?}", other),
		}
		assert!(matches!(withdraw_request(json!({ "tag": "payRequest" })), Err(CardError::BadLink(_))));
		assert!(matches!(
			withdraw_request(json!({ "tag": "withdrawRequest", "k1": "abc" })),
			Err(CardError::Unreachable(_)),
		));
	}
}
