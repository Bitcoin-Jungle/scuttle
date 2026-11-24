// Presentation of a Sale: labels, tones and the receipt number.
import type { PaidRail, Sale, SaleState } from './api'
import { t } from './i18n/index.svelte'
import { formatSats } from './money'

export function railLabel(rail: PaidRail | null): string {
	switch (rail) {
		case 'lightning':
			return t('rail.lightning')
		case 'onchain':
			return t('rail.onchain')
		default:
			return '—'
	}
}

/** The state in full, as on a receipt. */
export function stateLabel(state: SaleState): string {
	switch (state) {
		case 'open':
			return t('state.open')
		case 'in-progress':
			return t('state.inProgress')
		case 'paid':
			return t('state.paid')
		case 'expired':
			return t('state.expired')
		case 'canceled':
			return t('state.canceled')
	}
}

/** The state in a word, for a list. */
export function stateShortLabel(state: SaleState): string {
	return state === 'open' ? t('state.openShort') : stateLabel(state)
}

/** The color of a state's chip: done, still coming, or over without a payment. */
export function stateTone(state: SaleState): 'ok' | 'warn' | 'muted' {
	switch (state) {
		case 'paid':
			return 'ok'
		case 'in-progress':
		case 'open':
			return 'warn'
		default:
			return 'muted'
	}
}

/** The sale is still waiting for its payment. */
export function isLive(state: SaleState): boolean {
	return state === 'open' || state === 'in-progress'
}

/** "12-3456" from the last six characters of the sale ID. */
export function receiptNumber(id: string): string {
	const clean = id.replace(/-/g, '').toUpperCase()
	const last6 = clean.slice(-6).padStart(6, '0')
	return `${last6.slice(0, 2)}-${last6.slice(2)}`
}

/** The headline amount: fiat when the sale was keyed in fiat, else sats. */
export function saleAmountLabel(sale: Sale, format: (amount: string, currency: string) => string): string {
	if (sale.fiat_amount !== null && sale.currency) return format(sale.fiat_amount, sale.currency)
	return formatSats(sale.amount_sat)
}

/** Remaining time as "m:ss", or null once expired. */
export function countdown(expiresAt: string, nowMs: number): string | null {
	const seconds = Math.ceil((Date.parse(expiresAt) - nowMs) / 1000)
	if (!Number.isFinite(seconds) || seconds <= 0) return null
	return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`
}

/** "YYYY-MM-DD" for a local date. */
export function isoDate(d: Date): string {
	const pad = (n: number) => n.toString().padStart(2, '0')
	return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

/*
 * What each rail's QR code carries. A terminal shows one rail at a time so
 * the code stays sparse: cheap phones with weak cameras must scan it on the
 * first try. Upper case lets the QR use its alphanumeric mode, which packs
 * bech32 text in about 40% fewer modules; BOLT 11, bech32 addresses and
 * the URI scheme are all case-insensitive.
 */

/** The Lightning QR: the bare invoice, in upper case. */
export function lightningQr(sale: Sale): string {
	return sale.lightning_invoice.toUpperCase()
}

/**
 * The on-chain QR: the BIP 21 URI with the scheme in upper case, and a
 * bech32 address too. A base58 address is case-sensitive and kept. Null
 * until the sale has its address.
 */
export function onchainQr(sale: Sale): string | null {
	const uri = sale.onchain_uri
	if (!uri || !sale.onchain_address) return null
	const address = /^(bc|tb|bcrt)1/i.test(sale.onchain_address)
		? sale.onchain_address.toUpperCase()
		: sale.onchain_address
	return `BITCOIN:${address}${uri.slice(`bitcoin:${sale.onchain_address}`.length)}`
}

/** The sats seen on the sale's address, confirmed or in the mempool. */
export function onchainSeen(sale: Sale): number {
	return sale.onchain_received_sat + sale.onchain_pending_sat
}

/**
 * Part of the amount arrived on-chain, and the rest is still to pay: the
 * sats seen and the sats owed, or null.
 */
export function partialPayment(sale: Sale): { seen: number; owed: number } | null {
	const seen = onchainSeen(sale)
	if (sale.state !== 'open' || seen === 0 || seen >= sale.amount_sat) return null
	return { seen, owed: sale.amount_sat - seen }
}
