// Typed client for the posd HTTP API. The contract is pos/openapi.json.

export type SaleState = 'open' | 'in-progress' | 'paid' | 'expired' | 'canceled'
export type PaidRail = 'lightning' | 'onchain'

export interface Sale {
	id: string
	created_at: string
	expires_at: string
	state: SaleState
	amount_sat: number
	fiat_amount: string | null
	currency: string | null
	btc_price: string | null
	memo: string | null
	lightning_invoice: string
	/** The sale can be paid on-chain; the address comes with addOnchain. */
	onchain_available: boolean
	/** Null until the cashier picks on-chain. */
	onchain_address: string | null
	/** The address and the full amount as a BIP 21 URI, null with the address. */
	onchain_uri: string | null
	/** Confirmed to the on-chain address so far. */
	onchain_received_sat: number
	/** In the mempool to the on-chain address. */
	onchain_pending_sat: number
	paid_at: string | null
	paid_rail: PaidRail | null
	received_sat: number
	device_name: string | null
}

export interface Store {
	name: string
	currency: string
	/** The decimal places a sale in `currency` may have. */
	currency_decimals: number
	onchain_min_sat: number
	/** The smallest sale in sats: below it the Ark server's Lightning
	 * receive fee would take the whole amount. */
	min_sale_sat: number
	lightning_address: string | null
	/** The key of the store's brand. */
	brand: string
}

export interface Terminal {
	store: Store
	device_name: string
}

export interface Rate {
	currency: string
	btc_price: string
	fetched_at: string
}

export type NewSale =
	| { fiat_amount: string; currency: string; memo?: string }
	| { amount_sat: number; memo?: string }

export class ApiError extends Error {
	constructor(
		readonly status: number,
		message: string,
	) {
		super(message)
		this.name = 'ApiError'
	}

	/** The device token is missing, not valid, or revoked. */
	get isUnauthorized(): boolean {
		return this.status === 401
	}
}

export type Fetch = typeof fetch

export interface ClientOptions {
	/** Prefix for every path; '' when posd serves this app. */
	baseUrl?: string
	token?: string | null
	fetch?: Fetch
	/** Called on every 401, before the error is thrown. */
	onUnauthorized?: () => void
}

/** Long-poll requests wait up to 25 s on the server. */
const WAIT_TIMEOUT_MS = 35_000
const DEFAULT_TIMEOUT_MS = 15_000
const CARD_TIMEOUT_MS = 30_000

export class PosClient {
	private readonly baseUrl: string
	private readonly token: string | null
	private readonly fetchFn: Fetch
	private readonly onUnauthorized?: () => void

	constructor(opts: ClientOptions = {}) {
		this.baseUrl = opts.baseUrl ?? ''
		this.token = opts.token ?? null
		this.fetchFn = opts.fetch ?? ((input, init) => fetch(input, init))
		this.onUnauthorized = opts.onUnauthorized
	}

	pair(code: string, deviceName: string): Promise<{ token: string; store: Store }> {
		return this.json('POST', '/api/v1/pair', { code, device_name: deviceName }, { auth: false })
	}

	terminal(): Promise<Terminal> {
		return this.json('GET', '/api/v1/terminal')
	}

	rate(): Promise<Rate> {
		return this.json('GET', '/api/v1/rate')
	}

	/**
	 * Creates a sale. A repeat with the same `requestKey`, after a response
	 * that got lost, returns the sale of the first request.
	 */
	createSale(sale: NewSale, requestKey?: string): Promise<Sale> {
		return this.json('POST', '/api/v1/sales', sale, {
			headers: requestKey ? { 'Idempotency-Key': requestKey } : undefined,
		})
	}

	sale(id: string, signal?: AbortSignal): Promise<Sale> {
		return this.json('GET', `/api/v1/sales/${encodeURIComponent(id)}`, undefined, { signal })
	}

	/**
	 * Returns when the sale is no longer in `state`, or when the sats seen
	 * on-chain are no longer `onchainSat`, or after the server's wait limit.
	 */
	waitSale(id: string, state: SaleState, onchainSat: number, signal?: AbortSignal): Promise<Sale> {
		const q = new URLSearchParams({ wait: state, onchain_sat: String(onchainSat) })
		const path = `/api/v1/sales/${encodeURIComponent(id)}?${q}`
		return this.json('GET', path, undefined, { timeoutMs: WAIT_TIMEOUT_MS, signal })
	}

	/** Gives the sale its on-chain address, when the cashier picks on-chain. */
	addOnchain(id: string): Promise<Sale> {
		return this.json('POST', `/api/v1/sales/${encodeURIComponent(id)}/onchain`)
	}

	/**
	 * Has a Bolt Card pay the sale: posd follows the link the terminal read
	 * from the card. The sale is still open when this returns; the payment
	 * arrives a moment later. A 422 carries the card's reason.
	 */
	payWithCard(id: string, link: string): Promise<Sale> {
		// posd waits for the card's service, up to 10 s per request.
		return this.json('POST', `/api/v1/sales/${encodeURIComponent(id)}/bolt-card`, { link }, { timeoutMs: CARD_TIMEOUT_MS })
	}

	cancelSale(id: string): Promise<Sale> {
		return this.json('POST', `/api/v1/sales/${encodeURIComponent(id)}/cancel`)
	}

	async sales(opts: { before?: string; limit?: number } = {}): Promise<Sale[]> {
		const q = new URLSearchParams()
		if (opts.before) q.set('before', opts.before)
		if (opts.limit) q.set('limit', String(opts.limit))
		const qs = q.toString()
		const res = await this.json<{ sales: Sale[] }>('GET', `/api/v1/sales${qs ? `?${qs}` : ''}`)
		return res.sales
	}

	/**
	 * CSV export for the local dates `from` to `to` (YYYY-MM-DD), both
	 * included. The range is sent as times, so the store's own days count,
	 * not UTC days.
	 */
	async salesCsv(from: string, to: string): Promise<Blob> {
		const q = new URLSearchParams(localDayRange(from, to))
		const res = await this.request('GET', `/api/v1/sales.csv?${q}`)
		return res.blob()
	}

	private async json<T>(
		method: string,
		path: string,
		body?: unknown,
		opts: { auth?: boolean; timeoutMs?: number; signal?: AbortSignal; headers?: Record<string, string> } = {},
	): Promise<T> {
		const res = await this.request(method, path, body, opts)
		return (await res.json()) as T
	}

	private async request(
		method: string,
		path: string,
		body?: unknown,
		opts: { auth?: boolean; timeoutMs?: number; signal?: AbortSignal; headers?: Record<string, string> } = {},
	): Promise<Response> {
		const headers: Record<string, string> = { Accept: 'application/json', ...opts.headers }
		if (body !== undefined) headers['Content-Type'] = 'application/json'
		if (opts.auth !== false && this.token) headers.Authorization = `Bearer ${this.token}`

		const timeout = new AbortController()
		const timer = setTimeout(() => timeout.abort(), opts.timeoutMs ?? DEFAULT_TIMEOUT_MS)
		const onAbort = () => timeout.abort()
		opts.signal?.addEventListener('abort', onAbort)

		let res: Response
		try {
			res = await this.fetchFn(`${this.baseUrl}${path}`, {
				method,
				headers,
				body: body === undefined ? undefined : JSON.stringify(body),
				signal: timeout.signal,
			})
		} catch (e) {
			if (opts.signal?.aborted) throw e
			throw new ApiError(0, 'The server is not reachable')
		} finally {
			clearTimeout(timer)
			opts.signal?.removeEventListener('abort', onAbort)
		}

		if (!res.ok) {
			if (res.status === 401) this.onUnauthorized?.()
			throw new ApiError(res.status, await errorMessage(res))
		}
		return res
	}
}

async function errorMessage(res: Response): Promise<string> {
	try {
		const body = (await res.json()) as { error?: unknown }
		if (typeof body.error === 'string' && body.error) return body.error
	} catch {
		/* not JSON */
	}
	return `Request failed (HTTP ${res.status})`
}

/** The start of the local day `from` and the start of the day after `to`. */
export function localDayRange(from: string, to: string): { from: string; to: string } {
	const start = new Date(`${from}T00:00:00`)
	const end = new Date(`${to}T00:00:00`)
	end.setDate(end.getDate() + 1)
	return { from: start.toISOString(), to: end.toISOString() }
}
