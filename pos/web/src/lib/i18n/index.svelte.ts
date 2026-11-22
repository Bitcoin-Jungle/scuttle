// The terminal's language. It follows the device's languages unless the
// staff pick one in Settings, which is remembered on the device.
//
// To add a language, add a catalog next to en.ts with every key of en.ts and
// register it in LOCALES. A catalog with a missing or extra key does not
// type-check, and a test checks that each keeps the {placeholders}.
import { en } from './en'
import { es } from './es'
import { fr } from './fr'

export type MessageKey = keyof typeof en
export type Messages = Record<MessageKey, string>

export const LOCALES = {
	en: { name: 'English', messages: en as Messages },
	es: { name: 'Español', messages: es },
	fr: { name: 'Français', messages: fr },
} satisfies Record<string, { name: string; messages: Messages }>

export type LocaleCode = keyof typeof LOCALES
/** A language, or 'auto' to follow the device. */
export type LocalePreference = LocaleCode | 'auto'

export const DEFAULT_LOCALE: LocaleCode = 'en'
const STORAGE_KEY = 'pos:locale'

function isLocaleCode(value: unknown): value is LocaleCode {
	return typeof value === 'string' && Object.hasOwn(LOCALES, value)
}

function baseLanguage(tag: string): string {
	return tag.toLowerCase().split('-')[0] ?? ''
}

/**
 * The first of the device's languages that the terminal has, by base
 * language ("es-CR" picks "es"), with the device's tag for formatting.
 */
export function matchLocale(languages: readonly string[]): { code: LocaleCode; tag: string } {
	for (const tag of languages) {
		const base = baseLanguage(tag)
		if (isLocaleCode(base)) return { code: base, tag }
	}
	return { code: DEFAULT_LOCALE, tag: DEFAULT_LOCALE }
}

function deviceLanguages(): readonly string[] {
	if (typeof navigator === 'undefined') return []
	return navigator.languages?.length ? navigator.languages : [navigator.language].filter(Boolean)
}

function readPreference(): LocalePreference {
	try {
		const stored = localStorage.getItem(STORAGE_KEY)
		if (stored === 'auto' || isLocaleCode(stored)) return stored
	} catch {
		/* storage unavailable */
	}
	return 'auto'
}

const state = $state<{ preference: LocalePreference }>({ preference: readPreference() })

/** The language in use and the tag Intl formats with. */
function resolve(preference: LocalePreference): { code: LocaleCode; tag: string } {
	const device = deviceLanguages()
	if (preference === 'auto') return matchLocale(device)
	// The device's regional variant of the chosen language formats numbers and
	// dates as people there expect, for example es-CR.
	const regional = device.find((tag) => baseLanguage(tag) === preference)
	return { code: preference, tag: regional ?? preference }
}

const current = $derived(resolve(state.preference))

export const i18n = {
	get preference(): LocalePreference {
		return state.preference
	},
	/** The language in use. */
	get locale(): LocaleCode {
		return current.code
	},
	/** The BCP 47 tag for Intl formatting. */
	get tag(): string {
		return current.tag
	},
	/** The language the device would pick. */
	get deviceLocale(): LocaleCode {
		return matchLocale(deviceLanguages()).code
	},
}

/** Uses `preference` and remembers it on this device. */
export function setLocalePreference(preference: LocalePreference): void {
	state.preference = preference
	try {
		localStorage.setItem(STORAGE_KEY, preference)
	} catch {
		/* storage unavailable: the choice lasts for this page */
	}
	syncDocumentLanguage()
}

/** Sets <html lang>, for screen readers and hyphenation. */
export function syncDocumentLanguage(): void {
	if (typeof document !== 'undefined') document.documentElement.lang = current.code
}

/** The message for `key`, with each {name} replaced by params[name]. */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
	const message = LOCALES[current.code].messages[key] ?? en[key]
	if (!params) return message
	return message.replace(/\{(\w+)\}/g, (match, name: string) =>
		Object.hasOwn(params, name) ? String(params[name]) : match,
	)
}
