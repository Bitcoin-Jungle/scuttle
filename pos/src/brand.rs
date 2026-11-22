//! Brands: the look of a store's terminals and the domain of its Lightning
//! address and pairing links.
//!
//! A brand is configuration, not code. posd's `[brands.<key>]` sections name
//! each brand, its public URL and an optional directory of assets: a
//! `brand.css` that sets the terminal's colour and type tokens, a mark, the
//! install icons and any fonts the CSS loads. A brand without a file uses the
//! built-in neutral one. The owner's wallet picks the brand's key when it
//! registers the store.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};

use crate::config::Brand;


/// The key of a brand, as in `[brands.<key>]`: 1 to 32 lowercase letters,
/// digits and `-`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, utoipa::ToSchema)]
#[schema(value_type = String, example = "example", pattern = "^[a-z0-9-]{1,32}$")]
pub struct BrandKey(String);

impl BrandKey {
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// A key read back from the database, where posd only writes valid
	/// keys.
	pub(crate) fn from_db(key: String) -> BrandKey {
		BrandKey(key)
	}
}

impl fmt::Display for BrandKey {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a brand key: 1 to 32 lowercase letters, digits and '-'")]
pub struct InvalidBrandKey(pub String);

impl FromStr for BrandKey {
	type Err = InvalidBrandKey;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let valid = !s.is_empty() && s.len() <= 32
			&& s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
		if valid { Ok(BrandKey(s.to_string())) } else { Err(InvalidBrandKey(s.to_string())) }
	}
}

impl<'de> Deserialize<'de> for BrandKey {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
	}
}

/// The files a brand's asset directory can hold. Only these are served.
pub const ASSET_FILES: &[&str] = &[
	"brand.css", "mark.svg", "mark.png", "spinner.svg", "icon-192.png", "icon-512.png",
];

/// Font files a brand's CSS may load, by extension.
const FONT_EXTENSIONS: &[&str] = &["woff2", "woff"];

/// Whether `name` is a file a brand's asset directory may serve: a known
/// asset, or a font. A single path component, so nothing outside the
/// directory is reachable.
pub fn servable(name: &str) -> bool {
	if name.is_empty() || name.len() > 64 || name.starts_with('.') {
		return false;
	}
	if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) {
		return false;
	}
	ASSET_FILES.contains(&name)
		|| name.rsplit_once('.').is_some_and(|(_, ext)| FONT_EXTENSIONS.contains(&ext))
}

/// What the terminal needs to know about a brand. posd puts it in the page.
#[derive(Debug, Clone, Serialize)]
pub struct BrandInfo {
	pub key: String,
	/// The brand's name, as in "Open Example". Not translated.
	pub name: String,
	/// The name of an installed terminal.
	pub app_name: String,
	/// The name under its home screen icon.
	pub short_name: String,
	/// The URL of the brand's mark.
	pub mark: String,
	/// The URL of the brand's loading animation, if it has one. Without one
	/// the terminal turns a ring around the mark.
	pub spinner: Option<String>,
	pub theme_color: String,
	pub theme_color_dark: String,
}

/// The brand posd shows where no configured brand applies.
pub const DEFAULT_KEY: &str = "default";

impl BrandInfo {
	/// The neutral brand, from the files built into posd.
	pub fn neutral() -> BrandInfo {
		BrandInfo {
			key: DEFAULT_KEY.into(),
			name: "Point of sale".into(),
			app_name: "Point of sale".into(),
			short_name: "POS".into(),
			mark: asset_url(DEFAULT_KEY, "mark.svg"),
			spinner: None,
			theme_color: DEFAULT_THEME_COLOR.into(),
			theme_color_dark: DEFAULT_THEME_COLOR_DARK.into(),
		}
	}

	/// Every brand of `cfg`, and the neutral one, by key.
	pub fn all(cfg: &crate::config::Config) -> std::collections::BTreeMap<String, BrandInfo> {
		let mut all = std::collections::BTreeMap::new();
		all.insert(DEFAULT_KEY.to_string(), BrandInfo::neutral());
		for (key, brand) in &cfg.brands {
			all.insert(key.to_string(), BrandInfo::new(key, brand));
		}
		all
	}

	pub fn new(key: &BrandKey, brand: &Brand) -> BrandInfo {
		let has = |file: &str| brand.asset(file).is_some();
		let mark = if has("mark.svg") {
			asset_url(key.as_str(), "mark.svg")
		} else if has("mark.png") {
			asset_url(key.as_str(), "mark.png")
		} else {
			asset_url(DEFAULT_KEY, "mark.svg")
		};
		BrandInfo {
			key: key.to_string(),
			name: brand.name.clone(),
			app_name: brand.app_name(),
			short_name: brand.short_name.clone().unwrap_or_else(|| brand.app_name()),
			mark,
			spinner: has("spinner.svg").then(|| asset_url(key.as_str(), "spinner.svg")),
			theme_color: brand.theme_color.clone().unwrap_or_else(|| DEFAULT_THEME_COLOR.into()),
			theme_color_dark: brand.theme_color_dark.clone()
				.unwrap_or_else(|| DEFAULT_THEME_COLOR_DARK.into()),
		}
	}
}

/// The page background of the neutral brand, in light and dark mode.
pub const DEFAULT_THEME_COLOR: &str = "#f4f4f2";
pub const DEFAULT_THEME_COLOR_DARK: &str = "#121212";

/// Where posd serves a brand's file.
pub fn asset_url(key: &str, file: &str) -> String {
	format!("/brands/{}/{}", key, file)
}

/// The path of `file` in an asset directory, if the directory has it.
pub fn asset_in(dir: &Path, file: &str) -> Option<PathBuf> {
	if !servable(file) {
		return None;
	}
	let path = dir.join(file);
	path.is_file().then_some(path)
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn keys() {
		assert_eq!("example".parse::<BrandKey>().unwrap().as_str(), "example");
		assert!("my-brand-2".parse::<BrandKey>().is_ok());
		for bad in ["", "Upper", "a b", "a/b", "a.b", &"x".repeat(33)] {
			assert!(bad.parse::<BrandKey>().is_err(), "{bad}");
		}
		assert!(serde_json::from_str::<BrandKey>("\"UPPER\"").is_err());
	}

	#[test]
	fn servable_files() {
		for ok in ["brand.css", "mark.svg", "icon-512.png", "BebasNeue-Regular.woff2"] {
			assert!(servable(ok), "{ok}");
		}
		for bad in ["", "../brand.css", "a/brand.css", ".hidden.woff2", "notes.txt", "x.css"] {
			assert!(!servable(bad), "{bad}");
		}
	}
}
