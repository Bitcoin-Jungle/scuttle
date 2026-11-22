//! A lock manager with its own keyspace per wallet.
//!
//! SDK test wallets use a custom persister and share a process. Bark's
//! in-memory lock manager shares keys across that process. Prefixing each
//! key with the wallet's datadir lets independent wallets recover at the
//! same time and keeps two handles to the same wallet mutually exclusive.

use std::path::Path;
use std::time::Duration;

use bark::lock_manager::{LockGuard, LockManager};
use bark::lock_manager::memory::MemoryLockManager;
use bitcoin::hashes::{Hash, sha256};


/// bark's in-memory lock manager with every key prefixed by a tag of the
/// wallet's datadir. Two wallets of one datadir still share their locks.
#[derive(Debug)]
pub struct WalletLockManager {
	inner: MemoryLockManager,
	prefix: String,
}

impl WalletLockManager {
	pub fn for_datadir(datadir: &Path) -> WalletLockManager {
		let tag = sha256::Hash::hash(datadir.as_os_str().as_encoded_bytes()).to_string();
		WalletLockManager { inner: MemoryLockManager::new(), prefix: format!("w{}", &tag[..16]) }
	}

	fn key(&self, key: &str) -> String {
		format!("{}.{}", self.prefix, key)
	}
}

#[async_trait::async_trait]
impl LockManager for WalletLockManager {
	async fn try_lock(&self, key: &str) -> Option<Box<dyn LockGuard>> {
		self.inner.try_lock(&self.key(key)).await
	}

	async fn lock(&self, key: &str, timeout: Duration) -> anyhow::Result<Box<dyn LockGuard>> {
		self.inner.lock(&self.key(key), timeout).await
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[tokio::test]
	async fn wallets_have_their_own_keys() {
		let a = WalletLockManager::for_datadir(Path::new("/test/a"));
		let a2 = WalletLockManager::for_datadir(Path::new("/test/a"));
		let b = WalletLockManager::for_datadir(Path::new("/test/b"));
		let held = a.try_lock("recovery").await.expect("free");
		assert!(b.try_lock("recovery").await.is_some(), "another wallet has its own key");
		assert!(a2.try_lock("recovery").await.is_none(), "the same wallet shares its key");
		drop(held);
	}
}
