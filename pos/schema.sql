--
-- PostgreSQL database dump
--



SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Name: payment_rail; Type: TYPE; Schema: public; Owner: -
--

CREATE TYPE public.payment_rail AS ENUM (
    'lightning',
    'onchain'
);


--
-- Name: sale_state; Type: TYPE; Schema: public; Owner: -
--

CREATE TYPE public.sale_state AS ENUM (
    'open',
    'in-progress',
    'paid',
    'expired',
    'canceled'
);


--
-- Name: device; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.device (
    id bigint NOT NULL,
    store_id bigint NOT NULL,
    name text NOT NULL,
    token_hash text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    revoked_at timestamp with time zone
);


--
-- Name: device_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.device_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: device_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.device_id_seq OWNED BY public.device.id;


--
-- Name: held_name; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.held_name (
    brand text NOT NULL,
    name text NOT NULL,
    store_id bigint NOT NULL,
    held_until timestamp with time zone NOT NULL
);


--
-- Name: invite; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.invite (
    id bigint NOT NULL,
    store_id bigint NOT NULL,
    code_hash text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    redeemed_at timestamp with time zone,
    device_id bigint
);


--
-- Name: invite_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.invite_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: invite_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.invite_id_seq OWNED BY public.invite.id;


--
-- Name: owner_session; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.owner_session (
    id bigint NOT NULL,
    store_id bigint NOT NULL,
    token_hash text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL
);


--
-- Name: owner_session_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.owner_session_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: owner_session_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.owner_session_id_seq OWNED BY public.owner_session.id;


--
-- Name: refinery_schema_history; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.refinery_schema_history (
    version integer NOT NULL,
    name character varying(255),
    applied_on character varying(255),
    checksum character varying(255)
);


--
-- Name: reserved_name; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.reserved_name (
    brand text NOT NULL,
    name text NOT NULL
);


--
-- Name: sale; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.sale (
    id bigint NOT NULL,
    public_id text NOT NULL,
    store_id bigint NOT NULL,
    device_id bigint,
    amount_sat bigint NOT NULL,
    fiat_amount text,
    currency text,
    btc_price text,
    memo text,
    state public.sale_state DEFAULT 'open'::public.sale_state NOT NULL,
    paid_rail public.payment_rail,
    received_sat bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    paid_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    request_key text,
    request_fingerprint text
);


--
-- Name: sale_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.sale_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: sale_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.sale_id_seq OWNED BY public.sale.id;


--
-- Name: sale_payment; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.sale_payment (
    id bigint NOT NULL,
    sale_id bigint NOT NULL,
    rail public.payment_rail NOT NULL,
    payment_hash text,
    invoice text,
    onchain_index bigint,
    onchain_address text,
    received_sat bigint DEFAULT 0 NOT NULL,
    pending_sat bigint DEFAULT 0 NOT NULL,
    txids text[],
    settled_at timestamp with time zone
);


--
-- Name: sale_payment_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.sale_payment_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: sale_payment_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.sale_payment_id_seq OWNED BY public.sale_payment.id;


--
-- Name: server_key; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.server_key (
    name text NOT NULL,
    key bytea NOT NULL
);


--
-- Name: store; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.store (
    id bigint NOT NULL,
    name text NOT NULL,
    ark_address text NOT NULL,
    onchain_descriptor text NOT NULL,
    next_onchain_index bigint DEFAULT 0 NOT NULL,
    silent_payment_address text,
    currency text NOT NULL,
    brand text NOT NULL,
    onchain_min_sat bigint NOT NULL,
    lightning_address_name text,
    registration_signature text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: store_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.store_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: store_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.store_id_seq OWNED BY public.store.id;


--
-- Name: used_login_nonce; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.used_login_nonce (
    nonce text NOT NULL,
    expires_at timestamp with time zone NOT NULL
);


--
-- Name: device id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.device ALTER COLUMN id SET DEFAULT nextval('public.device_id_seq'::regclass);


--
-- Name: invite id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.invite ALTER COLUMN id SET DEFAULT nextval('public.invite_id_seq'::regclass);


--
-- Name: owner_session id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.owner_session ALTER COLUMN id SET DEFAULT nextval('public.owner_session_id_seq'::regclass);


--
-- Name: sale id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale ALTER COLUMN id SET DEFAULT nextval('public.sale_id_seq'::regclass);


--
-- Name: sale_payment id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale_payment ALTER COLUMN id SET DEFAULT nextval('public.sale_payment_id_seq'::regclass);


--
-- Name: store id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.store ALTER COLUMN id SET DEFAULT nextval('public.store_id_seq'::regclass);


--
-- Name: device device_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.device
    ADD CONSTRAINT device_pkey PRIMARY KEY (id);


--
-- Name: device device_token_hash_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.device
    ADD CONSTRAINT device_token_hash_key UNIQUE (token_hash);


--
-- Name: held_name held_name_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.held_name
    ADD CONSTRAINT held_name_pkey PRIMARY KEY (brand, name);


--
-- Name: invite invite_code_hash_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.invite
    ADD CONSTRAINT invite_code_hash_key UNIQUE (code_hash);


--
-- Name: invite invite_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.invite
    ADD CONSTRAINT invite_pkey PRIMARY KEY (id);


--
-- Name: owner_session owner_session_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.owner_session
    ADD CONSTRAINT owner_session_pkey PRIMARY KEY (id);


--
-- Name: owner_session owner_session_token_hash_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.owner_session
    ADD CONSTRAINT owner_session_token_hash_key UNIQUE (token_hash);


--
-- Name: refinery_schema_history refinery_schema_history_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.refinery_schema_history
    ADD CONSTRAINT refinery_schema_history_pkey PRIMARY KEY (version);


--
-- Name: reserved_name reserved_name_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.reserved_name
    ADD CONSTRAINT reserved_name_pkey PRIMARY KEY (brand, name);


--
-- Name: sale_payment sale_payment_payment_hash_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale_payment
    ADD CONSTRAINT sale_payment_payment_hash_key UNIQUE (payment_hash);


--
-- Name: sale_payment sale_payment_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale_payment
    ADD CONSTRAINT sale_payment_pkey PRIMARY KEY (id);


--
-- Name: sale_payment sale_payment_sale_id_rail_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale_payment
    ADD CONSTRAINT sale_payment_sale_id_rail_key UNIQUE (sale_id, rail);


--
-- Name: sale sale_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale
    ADD CONSTRAINT sale_pkey PRIMARY KEY (id);


--
-- Name: sale sale_public_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale
    ADD CONSTRAINT sale_public_id_key UNIQUE (public_id);


--
-- Name: server_key server_key_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.server_key
    ADD CONSTRAINT server_key_pkey PRIMARY KEY (name);


--
-- Name: store store_ark_address_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.store
    ADD CONSTRAINT store_ark_address_key UNIQUE (ark_address);


--
-- Name: store store_brand_lightning_address_name_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.store
    ADD CONSTRAINT store_brand_lightning_address_name_key UNIQUE (brand, lightning_address_name);


--
-- Name: store store_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.store
    ADD CONSTRAINT store_pkey PRIMARY KEY (id);


--
-- Name: used_login_nonce used_login_nonce_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.used_login_nonce
    ADD CONSTRAINT used_login_nonce_pkey PRIMARY KEY (nonce);


--
-- Name: device_store_id_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX device_store_id_idx ON public.device USING btree (store_id);


--
-- Name: sale_payment_lightning_unsettled_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX sale_payment_lightning_unsettled_idx ON public.sale_payment USING btree (sale_id) WHERE ((rail = 'lightning'::public.payment_rail) AND (settled_at IS NULL));


--
-- Name: sale_payment_onchain_address_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX sale_payment_onchain_address_idx ON public.sale_payment USING btree (onchain_address) WHERE (onchain_address IS NOT NULL);


--
-- Name: sale_payment_onchain_unsettled_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX sale_payment_onchain_unsettled_idx ON public.sale_payment USING btree (sale_id) WHERE ((rail = 'onchain'::public.payment_rail) AND (settled_at IS NULL));


--
-- Name: sale_request_key_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX sale_request_key_idx ON public.sale USING btree (device_id, request_key) WHERE (request_key IS NOT NULL);


--
-- Name: sale_store_created_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX sale_store_created_idx ON public.sale USING btree (store_id, created_at);


--
-- Name: sale_store_id_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX sale_store_id_idx ON public.sale USING btree (store_id, id DESC);


--
-- Name: sale_watch_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX sale_watch_idx ON public.sale USING btree (state, expires_at);


--
-- Name: device device_store_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.device
    ADD CONSTRAINT device_store_id_fkey FOREIGN KEY (store_id) REFERENCES public.store(id);


--
-- Name: held_name held_name_store_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.held_name
    ADD CONSTRAINT held_name_store_id_fkey FOREIGN KEY (store_id) REFERENCES public.store(id);


--
-- Name: invite invite_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.invite
    ADD CONSTRAINT invite_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.device(id);


--
-- Name: invite invite_store_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.invite
    ADD CONSTRAINT invite_store_id_fkey FOREIGN KEY (store_id) REFERENCES public.store(id);


--
-- Name: owner_session owner_session_store_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.owner_session
    ADD CONSTRAINT owner_session_store_id_fkey FOREIGN KEY (store_id) REFERENCES public.store(id);


--
-- Name: sale sale_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale
    ADD CONSTRAINT sale_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.device(id);


--
-- Name: sale_payment sale_payment_sale_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale_payment
    ADD CONSTRAINT sale_payment_sale_id_fkey FOREIGN KEY (sale_id) REFERENCES public.sale(id);


--
-- Name: sale sale_store_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sale
    ADD CONSTRAINT sale_store_id_fkey FOREIGN KEY (store_id) REFERENCES public.store(id);


--
-- PostgreSQL database dump complete
--


