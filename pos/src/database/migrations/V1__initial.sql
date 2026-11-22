-- A store is registered by the Ark address that receives its payments.
-- Lightning address names held back on a brand's domain, for example the
-- usernames of users who move from another service. Registration and owners
-- cannot take them; only the brand's operator API assigns them.
CREATE TABLE reserved_name (
	brand TEXT NOT NULL,
	name TEXT NOT NULL,
	PRIMARY KEY (brand, name)
);

CREATE TABLE store (
	id BIGSERIAL PRIMARY KEY,
	name TEXT NOT NULL,
	ark_address TEXT NOT NULL UNIQUE,
	onchain_descriptor TEXT NOT NULL,
	-- The next derivation index to reserve for an on-chain sale.
	next_onchain_index BIGINT NOT NULL DEFAULT 0,
	currency TEXT NOT NULL,
	-- The brand of the store, a key of posd's [brands] config.
	brand TEXT NOT NULL,
	onchain_min_sat BIGINT NOT NULL,
	-- The name of the store's Lightning address on its brand's domain.
	lightning_address_name TEXT,
	registration_signature TEXT NOT NULL,
	created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
	updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
	-- Each brand's domain has its own names.
	UNIQUE (brand, lightning_address_name)
);

-- A paired terminal. Only the hash of its token is stored.
CREATE TABLE device (
	id BIGSERIAL PRIMARY KEY,
	store_id BIGINT NOT NULL REFERENCES store(id),
	name TEXT NOT NULL,
	token_hash TEXT NOT NULL UNIQUE,
	created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
	revoked_at TIMESTAMPTZ
);
CREATE INDEX device_store_id_idx ON device (store_id);

-- A single-use pairing code.
CREATE TABLE invite (
	id BIGSERIAL PRIMARY KEY,
	store_id BIGINT NOT NULL REFERENCES store(id),
	code_hash TEXT NOT NULL UNIQUE,
	created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
	expires_at TIMESTAMPTZ NOT NULL,
	redeemed_at TIMESTAMPTZ,
	device_id BIGINT REFERENCES device(id)
);

-- Keys posd makes for itself, such as the one that authenticates login
-- challenges.
CREATE TABLE server_key (
	name TEXT PRIMARY KEY,
	key BYTEA NOT NULL
);

-- The nonce of each login challenge used, until the challenge expires. Only
-- a successful login adds one, so only store owners add rows.
CREATE TABLE used_login_nonce (
	nonce TEXT PRIMARY KEY,
	expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE owner_session (
	id BIGSERIAL PRIMARY KEY,
	store_id BIGINT NOT NULL REFERENCES store(id),
	token_hash TEXT NOT NULL UNIQUE,
	created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
	expires_at TIMESTAMPTZ NOT NULL
);

CREATE TYPE sale_state AS ENUM ('open', 'in-progress', 'paid', 'expired', 'canceled');
CREATE TYPE payment_rail AS ENUM ('lightning', 'onchain');

CREATE TABLE sale (
	id BIGSERIAL PRIMARY KEY,
	-- The identifier the API shows. It is random so that sales cannot be
	-- enumerated.
	public_id TEXT NOT NULL UNIQUE,
	store_id BIGINT NOT NULL REFERENCES store(id),
	-- No device for a Lightning address payment.
	device_id BIGINT REFERENCES device(id),
	amount_sat BIGINT NOT NULL,
	fiat_amount TEXT,
	currency TEXT,
	btc_price TEXT,
	memo TEXT,
	state sale_state NOT NULL DEFAULT 'open',
	paid_rail payment_rail,
	received_sat BIGINT NOT NULL DEFAULT 0,
	created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
	expires_at TIMESTAMPTZ NOT NULL,
	paid_at TIMESTAMPTZ,
	updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
	-- A terminal names each sale it asks for, so a request repeated after a
	-- lost response returns the first sale instead of making a second one.
	-- The fingerprint is the request as sent, to refuse a reused key for
	-- another sale.
	request_key TEXT,
	request_fingerprint TEXT
);
CREATE INDEX sale_store_id_idx ON sale (store_id, id DESC);
CREATE INDEX sale_watch_idx ON sale (state, expires_at);
CREATE UNIQUE INDEX sale_request_key_idx ON sale (device_id, request_key)
	WHERE request_key IS NOT NULL;

-- One row per payment rail offered for a sale.
CREATE TABLE sale_payment (
	id BIGSERIAL PRIMARY KEY,
	sale_id BIGINT NOT NULL REFERENCES sale(id),
	rail payment_rail NOT NULL,
	payment_hash TEXT UNIQUE,
	invoice TEXT,
	onchain_index BIGINT,
	onchain_address TEXT,
	-- The amount this rail delivered: the invoice amount once delivered, or
	-- the sum of the confirmed outputs to the address.
	received_sat BIGINT NOT NULL DEFAULT 0,
	-- The amount in unconfirmed outputs to the address.
	pending_sat BIGINT NOT NULL DEFAULT 0,
	txids TEXT[],
	-- When the rail stopped needing checks: a Lightning payment delivered,
	-- canceled or expired. On-chain payments stay unsettled: their watch
	-- window decides how long they are checked.
	settled_at TIMESTAMPTZ,
	UNIQUE (sale_id, rail)
);
-- One payment per address, across stores, so one transfer cannot pay two
-- sales.
CREATE UNIQUE INDEX sale_payment_onchain_address_idx ON sale_payment (onchain_address)
	WHERE onchain_address IS NOT NULL;
-- The watcher reads the unsettled Lightning payments on every pass.
CREATE INDEX sale_payment_lightning_unsettled_idx ON sale_payment (sale_id)
	WHERE rail = 'lightning' AND settled_at IS NULL;
