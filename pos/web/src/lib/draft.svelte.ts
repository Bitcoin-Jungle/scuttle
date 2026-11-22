// The sale being keyed in. It outlives the keypad, so a trip to Sales or
// Settings keeps it, and canceling a payment puts the sale back to edit.
import type { Sale } from './api'

export type AmountUnit = 'fiat' | 'sat'

export const draft = $state<{ unit: AmountUnit; amount: string; memo: string }>({ unit: 'fiat', amount: '', memo: '' })

export function clearDraft(): void {
	draft.amount = ''
	draft.memo = ''
}

/** Puts `sale` back on the keypad, in the unit it was keyed in. */
export function restoreDraft(sale: Sale): void {
	if (sale.fiat_amount !== null) {
		draft.unit = 'fiat'
		// "12500.00" is typed as "12500", "12.50" as "12.5".
		draft.amount = String(Number(sale.fiat_amount))
	} else {
		draft.unit = 'sat'
		draft.amount = String(sale.amount_sat)
	}
	draft.memo = sale.memo ?? ''
}
