import { describe, expect, it } from 'vitest'
import { en } from './en'
import { LOCALES, matchLocale, t } from './index.svelte'

function placeholders(message: string): string[] {
	return [...message.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort()
}

describe('language', () => {
	it('picks the first device language it has, by base language', () => {
		expect(matchLocale(['es-CR', 'en-US'])).toEqual({ code: 'es', tag: 'es-CR' })
		expect(matchLocale(['de-DE', 'fr-CA', 'en'])).toEqual({ code: 'fr', tag: 'fr-CA' })
		expect(matchLocale(['de-DE', 'en-GB'])).toEqual({ code: 'en', tag: 'en-GB' })
	})

	it('falls back to English', () => {
		expect(matchLocale(['de-DE', 'ja']).code).toBe('en')
		expect(matchLocale([]).code).toBe('en')
	})

	it('fills placeholders', () => {
		expect(t('unit.sats', { amount: '1,000' })).toBe('1,000 sats')
		expect(t('pay.overpaid', { received: 'a', extra: 'b' })).toBe('Received a, b more than the sale.')
	})
})

describe('catalogs', () => {
	for (const [code, locale] of Object.entries(LOCALES)) {
		it(`${code} has every message, with the same placeholders as English`, () => {
			for (const key of Object.keys(en) as (keyof typeof en)[]) {
				const message = locale.messages[key]
				expect(message, `${code} ${key}`).toBeTruthy()
				expect(placeholders(message), `${code} ${key}`).toEqual(placeholders(en[key]))
			}
		})
	}
})
