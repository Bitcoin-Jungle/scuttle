// Retrying requests on a poor network.

/**
 * How long to wait before retry number `failures` (1 for the first): 1, 2,
 * 4 and 8 seconds, then 15, each spread by ±30% so terminals that lost the
 * server together do not all come back in the same second.
 */
export function retryDelay(failures: number, random: () => number = Math.random): number {
	const base = Math.min(15_000, 1_000 * 2 ** Math.max(0, failures - 1))
	return Math.round(base * (0.7 + 0.6 * random()))
}

/** A random key for a request, from the browser's CSPRNG. */
export function newRequestKey(): string {
	const bytes = crypto.getRandomValues(new Uint8Array(16))
	return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
}

/**
 * Names requests so posd can recognise a repeat: the same body keeps its
 * key until `done`, so tapping again after a lost response asks for the
 * same sale instead of a second one. A different body gets a new key.
 */
export class RequestKeys {
	private body: string | null = null
	private key: string | null = null

	constructor(private readonly generate: () => string = newRequestKey) {}

	for(body: unknown): string {
		const json = JSON.stringify(body)
		if (this.key === null || json !== this.body) {
			this.body = json
			this.key = this.generate()
		}
		return this.key
	}

	/** The request got an answer: the next one is a new request. */
	done(): void {
		this.body = null
		this.key = null
	}
}

/** Keys for the sales this terminal asks for. */
export const saleRequests = new RequestKeys()
