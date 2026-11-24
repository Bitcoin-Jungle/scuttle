//! Lightning address names: reserved ones and held ones.
//!
//! A name is unique on its brand's domain. Two kinds of name cannot be
//! taken by just any store:
//!
//! - A reserved name, which only the brand's operator assigns, for example
//!   to a user who moves from another service.
//! - A held name, which a store gave up by renaming, removing its address or
//!   moving to another brand. Payers may have saved the old address, so for
//!   [HOLD_DAYS] only that store can take it back; after that, anyone can.
//!   A payment to a held name fails rather than reaching another store.

use tokio_postgres::GenericClient;

use crate::brand::BrandKey;
use crate::error::{ApiResult, conflict};


/// How long a name a store gave up is held for it.
pub const HOLD_DAYS: i32 = 90;

/// Locks names on their domains until the transaction ends, so a store
/// giving a name up and another taking it cannot interleave. Locks are
/// taken in one order, so two stores swapping names cannot deadlock.
pub async fn lock(tx: &impl GenericClient, names: &[(&BrandKey, &str)]) -> ApiResult<()> {
	let mut keys = names.iter().map(|(brand, name)| format!("{}/{}", brand, name)).collect::<Vec<_>>();
	keys.sort();
	keys.dedup();
	for key in keys {
		tx.execute("SELECT pg_advisory_xact_lock(hashtextextended($1, 1))", &[&key]).await?;
	}
	Ok(())
}

/// Refuses `name` on `brand`'s domain if it is reserved: only the brand's
/// operator assigns it.
pub async fn refuse_reserved(conn: &impl GenericClient, brand: &BrandKey, name: &str) -> ApiResult<()> {
	let reserved = conn.query_opt(
		"SELECT 1 FROM reserved_name WHERE brand = $1 AND name = $2", &[&brand.as_str(), &name],
	).await?.is_some();
	if reserved {
		return Err(conflict("that Lightning address is reserved"));
	}
	Ok(())
}

/// Refuses `name` on `brand`'s domain if another store gave it up less
/// than [HOLD_DAYS] ago. `store_id` is the store that takes it, `None` for
/// a store being registered. A store taking back its own name ends the hold.
/// Call with the name locked.
pub async fn take_held(
	conn: &impl GenericClient,
	brand: &BrandKey,
	name: &str,
	store_id: Option<i64>,
) -> ApiResult<()> {
	let row = conn.query_opt("
		SELECT store_id, held_until FROM held_name
		WHERE brand = $1 AND name = $2 AND held_until > NOW()
	", &[&brand.as_str(), &name]).await?;
	let Some(row) = row else { return Ok(()) };
	if store_id != Some(row.get::<_, i64>("store_id")) {
		let until = row.get::<_, chrono::DateTime<chrono::Utc>>("held_until");
		return Err(conflict(format!(
			"that Lightning address was given up recently and is held until {}",
			until.format("%Y-%m-%d"),
		)));
	}
	conn.execute("DELETE FROM held_name WHERE brand = $1 AND name = $2", &[&brand.as_str(), &name]).await?;
	Ok(())
}

/// Holds `name` on `brand`'s domain for `store_id`, which just gave it up.
/// Call with the name locked.
pub async fn hold(conn: &impl GenericClient, brand: &BrandKey, name: &str, store_id: i64) -> ApiResult<()> {
	conn.execute("
		INSERT INTO held_name (brand, name, store_id, held_until)
		VALUES ($1, $2, $3, NOW() + make_interval(days => $4))
		ON CONFLICT (brand, name) DO UPDATE
			SET store_id = EXCLUDED.store_id, held_until = EXCLUDED.held_until
	", &[&brand.as_str(), &name, &store_id, &HOLD_DAYS]).await?;
	Ok(())
}

/// Moves a store's Lightning address from `old` to `new`, either of which
/// may be none: holds the name it gives up and checks the one it takes.
/// Reserved names are refused unless the brand's operator assigns them.
/// Call with both names locked, before the store row changes.
pub async fn change(
	conn: &impl GenericClient,
	store_id: i64,
	old: Option<(&BrandKey, &str)>,
	new: Option<(&BrandKey, &str)>,
	by_operator: bool,
) -> ApiResult<()> {
	if old == new {
		return Ok(());
	}
	if let Some((brand, name)) = new {
		if !by_operator {
			refuse_reserved(conn, brand, name).await?;
		}
		take_held(conn, brand, name, Some(store_id)).await?;
	}
	if let Some((brand, name)) = old {
		hold(conn, brand, name, store_id).await?;
	}
	Ok(())
}
