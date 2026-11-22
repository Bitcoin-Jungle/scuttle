import { describe, expect, it } from 'vitest'
import type { Sale } from './api'
import { countdown, isLive, lightningQr, onchainQr, onchainUri, railLabel, receiptNumber, saleAmountLabel, stateLabel, stateShortLabel, stateTone } from './sale'

function sale(overrides: Partial<Sale> = {}): Sale {
	return {
		id: '4f1c9a2e-0000-4000-8000-1234567890ab',
		created_at: '2026-11-20T15:04:05Z',
		expires_at: '2026-11-20T15:14:05Z',
		state: 'open',
		amount_sat: 35000,
		fiat_amount: '12.50',
		currency: 'USD',
		btc_price: '62500.00',
		memo: null,
		payment_request: 'bitcoin:?lightning=lnbc1',
		lightning_invoice: 'lnbc1',
		onchain_address: null,
		paid_at: null,
		paid_rail: null,
		received_sat: 0,
		device_name: 'Front till',
		...overrides,
	}
}

describe('sale presentation', () => {
	it('labels each rail', () => {
		expect(railLabel('lightning')).toBe('Lightning')
		expect(railLabel('onchain')).toBe('On-chain')
		expect(railLabel(null)).toBe('—')
	})

	it('labels each state', () => {
		expect(stateLabel('in-progress')).toBe('Confirming')
		expect(stateShortLabel('open')).toBe('Waiting')
		expect(stateTone('expired')).toBe('muted')
		expect(stateLabel('canceled')).toBe('Canceled')
	})

	it('treats open and in-progress sales as live', () => {
		expect(isLive('open')).toBe(true)
		expect(isLive('in-progress')).toBe(true)
		expect(isLive('paid')).toBe(false)
		expect(isLive('expired')).toBe(false)
	})

	it('derives the receipt number from the sale ID', () => {
		expect(receiptNumber(sale().id)).toBe('78-90AB')
	})

	it('headlines fiat for a fiat sale and sats otherwise', () => {
		const fmt = (a: string, c: string) => `${c} ${a}`
		expect(saleAmountLabel(sale(), fmt)).toBe('USD 12.50')
		expect(saleAmountLabel(sale({ fiat_amount: null, currency: null }), fmt)).toBe(
			`${new Intl.NumberFormat().format(35000)} sats`,
		)
	})

	it('counts down to expiry', () => {
		const expires = Date.parse('2026-11-20T15:14:05Z')
		expect(countdown('2026-11-20T15:14:05Z', expires - 125_000)).toBe('2:05')
		expect(countdown('2026-11-20T15:14:05Z', expires)).toBeNull()
	})
})

describe('payment QR codes', () => {
	it('upper-cases the bare Lightning invoice', () => {
		expect(lightningQr(sale({ lightning_invoice: 'lnbc200u1pabc' }))).toBe('LNBC200U1PABC')
	})

	it('drops the invoice from the on-chain URI and upper-cases a bech32 address', () => {
		const s = sale({
			onchain_address: 'bc1qxyz',
			payment_request: 'bitcoin:bc1qxyz?amount=0.0008&lightning=lnbc1',
		})
		expect(onchainQr(s)).toBe('BITCOIN:BC1QXYZ?amount=0.0008')
		expect(onchainUri(s)).toBe('bitcoin:bc1qxyz?amount=0.0008')
	})

	it('keeps the case of a base58 address', () => {
		const s = sale({ onchain_address: '3Abc', payment_request: 'bitcoin:3Abc?amount=0.001&lightning=lnbc1' })
		expect(onchainQr(s)).toBe('BITCOIN:3Abc?amount=0.001')
	})

	it('has no on-chain code without an address', () => {
		expect(onchainQr(sale())).toBeNull()
		expect(onchainUri(sale())).toBeNull()
	})
})
