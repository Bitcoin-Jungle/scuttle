// The brand of the terminal: its name, mark and look. posd writes every
// brand it serves into the page (#pos-brands), with the page's brand
// already applied: its stylesheet, icons and browser colours. A paired
// terminal changes to its store's brand when that is another, and keeps it
// for the next start.

import { t } from './i18n/index.svelte'

export interface BrandInfo {
	key: string
	/** The brand's name, as in "Open Example". Not translated. */
	name: string
	/** The name of an installed terminal. */
	app_name: string
	/** The URL of the brand's mark. */
	mark: string
	/** The URL of the brand's loading animation, or null for a ring around the mark. */
	spinner: string | null
	theme_color: string
	theme_color_dark: string
}

export const STORAGE_KEY = 'pos:brand'

/** The neutral brand, for a page posd did not serve, as in development. */
const NEUTRAL: BrandInfo = {
	key: 'default',
	name: 'Point of sale',
	app_name: 'Point of sale',
	mark: '/brands/default/mark.svg',
	spinner: null,
	theme_color: '#f4f4f2',
	theme_color_dark: '#121212',
}

function readBrands(): { current: string; brands: Record<string, BrandInfo> } {
	try {
		const el = document.getElementById('pos-brands')
		if (el?.textContent) return JSON.parse(el.textContent)
	} catch {
		/* not served by posd */
	}
	return { current: NEUTRAL.key, brands: { [NEUTRAL.key]: NEUTRAL } }
}

const page = readBrands()

// index.html may have changed to the remembered brand before the first paint.
const state = $state<{ key: string }>({ key: document.documentElement.dataset.brand ?? page.current })

export const brand = {
	get key(): string {
		return state.key
	},
	get info(): BrandInfo {
		return page.brands[state.key] ?? page.brands[page.current] ?? NEUTRAL
	},
	/** The owner's wallet, as in "open Example": the brand's name, or a neutral phrase. */
	get wallet(): string {
		return this.info.key === NEUTRAL.key ? t('brand.wallet') : this.info.name
	},
}

/** Changes to `key`, if posd serves it, and remembers it for the next start. */
export function applyBrand(key: string): void {
	if (!page.brands[key]) return
	try {
		localStorage.setItem(STORAGE_KEY, key)
	} catch {
		/* storage unavailable: the brand lasts for this page */
	}
	if (key === state.key) return
	state.key = key
	syncHead(key)
}

/** Sets the document's brand: its attribute, stylesheet, icons and browser colours. */
function syncHead(key: string): void {
	const info = page.brands[key]
	if (!info) return
	document.documentElement.dataset.brand = key
	const css = document.querySelector<HTMLLinkElement>('link[data-brand-css]')
	const href = `/brands/${key}/brand.css`
	if (css && css.getAttribute('href') !== href) css.href = href
	for (const link of document.querySelectorAll<HTMLLinkElement>('link[data-brand-icon]')) {
		link.href = `/brands/${key}/icon-192.png`
	}
	for (const meta of document.querySelectorAll<HTMLMetaElement>('meta[name="theme-color"]')) {
		meta.content = meta.media.includes('dark') ? info.theme_color_dark : info.theme_color
	}
}

// index.html may have started in a remembered brand: bring the rest of the
// head in line with it.
if (state.key !== page.current) syncHead(state.key)
