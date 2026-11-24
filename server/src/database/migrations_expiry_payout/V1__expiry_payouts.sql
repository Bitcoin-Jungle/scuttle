-- The mode of the server, in one row. The server sets expiry_payouts
-- when it creates the database, before it co-signs any VTXO, and never
-- changes it. A flag is enough: the mode has no other state.
CREATE TABLE server_mode (
	id INTEGER PRIMARY KEY CHECK (id = 1),
	expiry_payouts BOOLEAN NOT NULL
);
INSERT INTO server_mode (id, expiry_payouts) VALUES (1, FALSE);

-- The id of the signed intent of a lightning send. A request that is sent
-- again finds its attempt by this id and starts no other payment.
ALTER TABLE lightning_payment_attempt
	ADD COLUMN intent_id TEXT UNIQUE CHECK (intent_id IS NULL OR length(intent_id) = 64);

-- The owner of a board, bound when the server co-signs its exit.
ALTER TABLE board_exit
	ADD COLUMN owner_mailbox TEXT
	CHECK (owner_mailbox IS NULL OR length(owner_mailbox) = 66);
ALTER TABLE board_exit
	ADD COLUMN user_pubkey TEXT
	CHECK (user_pubkey IS NULL OR length(user_pubkey) = 66);
CREATE INDEX board_exit_exit_txid_idx ON board_exit(exit_txid);

-- A round participation that an expiry payout cancelled. The server can
-- delete a superseded participation, so this row has no foreign key and
-- keeps the unlock hash, which must never get its preimage.
CREATE TABLE expiry_cancelled_participation (
	participation_id BIGINT PRIMARY KEY,
	unlock_hash TEXT UNIQUE
);

-- The payout of one expired tree. The decision never changes. The spends
-- of its inputs on-chain give its state, see v_expiry_payout_state.
CREATE TABLE expiry_payout (
	id BIGSERIAL PRIMARY KEY,
	expiry_height INTEGER NOT NULL CHECK (expiry_height >= 0),
	broadcast_at TIMESTAMPTZ
);

-- The tables below hold rows of an expiry payout that nothing references,
-- so each one uses its natural key.

-- The tree root that a payout spends.
CREATE TABLE expiry_payout_input (
	expiry_payout_id BIGINT NOT NULL REFERENCES expiry_payout(id),
	vtxo_id TEXT NOT NULL REFERENCES vtxo(vtxo_id),
	PRIMARY KEY (expiry_payout_id, vtxo_id)
);

-- Each signed payout transaction. An earlier attempt can confirm after a
-- fee bump, so every attempt stays.
CREATE TABLE expiry_payout_attempt (
	expiry_payout_id BIGINT NOT NULL REFERENCES expiry_payout(id),
	txid TEXT NOT NULL CHECK (length(txid) = 64),
	signed_tx BYTEA NOT NULL,
	fee_sat BIGINT NOT NULL CHECK (fee_sat >= 0),
	PRIMARY KEY (expiry_payout_id, txid),
	UNIQUE (txid)
);

-- The decision for each VTXO of the tree: who gets its value, and how
-- much. A VTXO with a row is decided, and the spend guard refuses any
-- change to it.
CREATE TABLE expiry_payout_vtxo (
	expiry_payout_id BIGINT NOT NULL REFERENCES expiry_payout(id),
	vtxo_id TEXT NOT NULL REFERENCES vtxo(vtxo_id),
	owner_mailbox TEXT CHECK (owner_mailbox IS NULL OR length(owner_mailbox) = 66),
	disposition TEXT NOT NULL CHECK (disposition IN ('owner', 'operator')),
	gross_sat BIGINT NOT NULL CHECK (gross_sat > 0),
	fee_sat BIGINT NOT NULL CHECK (fee_sat >= 0),
	payout_sat BIGINT NOT NULL CHECK (payout_sat >= 0),
	-- A decision that rests on a lightning payment is made again after a
	-- conflict. Any other decision is repeated as it is.
	replannable BOOLEAN NOT NULL,
	PRIMARY KEY (expiry_payout_id, vtxo_id),
	CHECK ((disposition = 'owner' AND owner_mailbox IS NOT NULL
		AND gross_sat = fee_sat + payout_sat)
		OR (disposition = 'operator' AND fee_sat = 0 AND payout_sat = 0))
);
CREATE INDEX expiry_payout_vtxo_vtxo_id_idx ON expiry_payout_vtxo(vtxo_id);

-- The owner mailbox of each VTXO that an owner holds, recorded when the
-- VTXO is created. A side table keeps the vtxo table unchanged, and the
-- VTXO id is its key because each VTXO has at most one row.
CREATE TABLE vtxo_owner_mailbox (
	vtxo_id TEXT PRIMARY KEY REFERENCES vtxo(vtxo_id),
	mailbox_pk TEXT NOT NULL CHECK (length(mailbox_pk) = 66)
);

-- Whether one payout confirmed or conflicted, from the spends of its inputs.
CREATE FUNCTION expiry_payout_observed(payout_id BIGINT)
RETURNS TABLE (confirmed BOOLEAN, conflicted BOOLEAN, confirmed_txid TEXT, height BIGINT)
LANGUAGE SQL STABLE
BEGIN ATOMIC
	SELECT COUNT(i.vtxo_id) > 0
			AND BOOL_AND(v.onchain_spent_txid IS NOT NULL AND b.height IS NOT NULL AND p.txid IS NOT NULL)
			AND COUNT(DISTINCT v.onchain_spent_txid) = 1,
		BOOL_OR(v.onchain_spent_txid IS NOT NULL AND p.txid IS NULL)
			OR COUNT(DISTINCT v.onchain_spent_txid) > 1,
		MIN(v.onchain_spent_txid),
		MIN(b.height)
	FROM expiry_payout_input i
	LEFT JOIN vtxo v ON v.vtxo_id = i.vtxo_id
	LEFT JOIN watchmand_block b ON b.height = v.onchain_spent_height
	LEFT JOIN expiry_payout_attempt p ON p.expiry_payout_id = i.expiry_payout_id AND p.txid = v.onchain_spent_txid
	WHERE i.expiry_payout_id = payout_id;
END;

-- The state of each expiry payout, from the spends of its inputs seen by
-- the watchman. It is confirmed when one of its attempts spends every
-- input. It is conflicted when another transaction spends an input, or
-- when another payout of one of its VTXOs confirmed. Else it is pending.
CREATE VIEW v_expiry_payout_state AS
SELECT a.id, a.expiry_height,
	CASE WHEN o.confirmed THEN o.confirmed_txid ELSE latest.txid END AS payout_txid,
	CASE WHEN o.confirmed THEN winner.signed_tx ELSE latest.signed_tx END AS payout_tx,
	a.broadcast_at,
	CASE WHEN o.confirmed THEN 'confirmed'
		WHEN o.conflicted OR EXISTS (
			SELECT 1 FROM expiry_payout_vtxo c
			JOIN expiry_payout_vtxo paid ON paid.vtxo_id = c.vtxo_id AND paid.expiry_payout_id <> a.id
			CROSS JOIN LATERAL expiry_payout_observed(paid.expiry_payout_id) prior
			WHERE c.expiry_payout_id = a.id AND prior.confirmed)
		THEN 'conflicted' ELSE 'pending' END AS status,
	CASE WHEN o.confirmed THEN o.height END AS confirmed_height
FROM expiry_payout a
CROSS JOIN LATERAL expiry_payout_observed(a.id) o
LEFT JOIN expiry_payout_attempt winner
	ON winner.expiry_payout_id = a.id AND winner.txid = o.confirmed_txid
-- Each replacement raises the absolute fee, so the attempt with the
-- highest fee is the latest.
LEFT JOIN LATERAL (SELECT p.txid, p.signed_tx FROM expiry_payout_attempt p
	WHERE p.expiry_payout_id = a.id ORDER BY p.fee_sat DESC LIMIT 1) latest ON TRUE;

-- Match VtxoInserts::owned_vtxo_ids for known states and policies. The
-- first byte is the protocol policy tag (currently 0x00 through 0x0b).
-- Unknown states and tags require a binding until explicitly classified.
CREATE FUNCTION vtxo_requires_owner_mailbox(state spend_state, policy BYTEA)
RETURNS BOOLEAN LANGUAGE SQL IMMUTABLE STRICT
RETURN CASE
	WHEN state IN ('spent', 'pool', 'ln-spent', 'round-forfeit', 'offboard-forfeit', 'offboard-connector') THEN FALSE
	WHEN octet_length(policy) = 0 THEN TRUE
	WHEN get_byte(policy, 0) > 11 THEN TRUE
	WHEN state IN ('spendable', 'unregistered', 'unclaimed', 'htlc-recv-unclaimed') THEN NOT (
		policy = '\x07'::BYTEA -- server-owned
		OR (get_byte(policy, 0) = 4 AND octet_length(policy) = 33) -- expiry
		OR (state = 'unclaimed' AND policy = -- round padding pubkey policy
			'\x0002dfa52f6690299d2d6a08323083e290597b56fee125063e5f4e2957731639c42c'::BYTEA)
	)
	ELSE TRUE
END;

CREATE FUNCTION require_vtxo_owner_mailbox() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
	payouts BOOLEAN;
BEGIN
	SELECT expiry_payouts INTO STRICT payouts FROM server_mode WHERE id = 1;
	IF payouts AND NOT EXISTS (SELECT 1 FROM vtxo_owner_mailbox WHERE vtxo_id = NEW.vtxo_id) THEN
		RAISE EXCEPTION 'VTXO % has no owner mailbox', NEW.vtxo_id
			USING ERRCODE = 'foreign_key_violation', CONSTRAINT = 'vtxo_require_owner_mailbox';
	END IF;
	RETURN NULL;
END;
$$;

-- Queue the check from the row as written, even if it becomes spent
-- later in this transaction. The binding itself may be inserted later.
CREATE CONSTRAINT TRIGGER vtxo_require_owner_mailbox
AFTER INSERT OR UPDATE OF vtxo_id, spend_state, policy ON vtxo
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
WHEN (vtxo_requires_owner_mailbox(NEW.spend_state, NEW.policy))
EXECUTE FUNCTION require_vtxo_owner_mailbox();

-- A binding remains relevant after spending, for example when the
-- watchman refunds an unregistered arkoor to its input owner.
CREATE FUNCTION preserve_vtxo_owner_mailbox() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
	RAISE EXCEPTION 'VTXO owner mailbox bindings are immutable'
		USING ERRCODE = 'integrity_constraint_violation';
END;
$$;

CREATE TRIGGER vtxo_owner_mailbox_immutable
BEFORE UPDATE OR DELETE ON vtxo_owner_mailbox FOR EACH ROW
EXECUTE FUNCTION preserve_vtxo_owner_mailbox();

CREATE TRIGGER vtxo_owner_mailbox_no_truncate
BEFORE TRUNCATE ON vtxo_owner_mailbox FOR EACH STATEMENT
EXECUTE FUNCTION preserve_vtxo_owner_mailbox();
