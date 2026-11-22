// Asking posd for a sale and opening its payment screen, from the keypad or
// from Try again.
import { ApiError, type NewSale } from './api'
import { applyBrand } from './brand.svelte'
import { t } from './i18n/index.svelte'
import { saleRequests } from './retry'
import { router } from './router.svelte'
import { rememberSale } from './sale-cache'
import { client, session } from './session.svelte'

/**
 * Creates the sale and opens it. Returns null on success, or the message
 * to show. A request repeated after a lost response gets the same sale.
 */
export async function startSale(body: NewSale): Promise<string | null> {
	try {
		const sale = await client().createSale(body, saleRequests.for(body))
		saleRequests.done()
		rememberSale(sale)
		router.go(`/pay/${sale.id}`)
		return null
	} catch (e) {
		// Without an answer the sale may exist: keep the key for the retry.
		if (!(e instanceof ApiError && e.status === 0)) saleRequests.done()
		if (!(e instanceof ApiError)) return t('common.genericError')
		if (e.status === 0) return t('keypad.unreachable')
		if (e.status === 502) return t('keypad.unavailable')
		if (e.status === 409 && 'currency' in body) {
			// The owner changed the store's currency: show the new one before
			// charging anything in it.
			try {
				const terminal = await client().terminal()
				session.terminal = terminal
				applyBrand(terminal.store.brand)
				return t('keypad.currencyChanged', { currency: terminal.store.currency })
			} catch {
				return t('common.genericError')
			}
		}
		return t('common.genericError')
	}
}
