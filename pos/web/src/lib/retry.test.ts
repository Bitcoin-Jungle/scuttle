import { describe, expect, it } from 'vitest'
import { RequestKeys, newRequestKey, retryDelay } from './retry'

describe('retryDelay', () => {
	it('doubles from 1 s up to 15 s', () => {
		const mid = () => 0.5
		expect([1, 2, 3, 4, 5, 9].map((n) => retryDelay(n, mid))).toEqual([1000, 2000, 4000, 8000, 15000, 15000])
	})

	it('spreads each delay by 30%', () => {
		expect(retryDelay(1, () => 0)).toBe(700)
		expect(retryDelay(1, () => 1)).toBe(1300)
	})
})

describe('RequestKeys', () => {
	it('keeps the key for a repeated body until done', () => {
		let n = 0
		const keys = new RequestKeys(() => `k${++n}`)
		expect(keys.for({ amount_sat: 1 })).toBe('k1')
		expect(keys.for({ amount_sat: 1 })).toBe('k1')
		expect(keys.for({ amount_sat: 2 })).toBe('k2')
		keys.done()
		expect(keys.for({ amount_sat: 2 })).toBe('k3')
	})

	it('makes keys posd accepts', () => {
		expect(newRequestKey()).toMatch(/^[0-9a-f]{32}$/)
	})
})
