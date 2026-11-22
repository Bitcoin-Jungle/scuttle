// Installing the terminal as an app.
//
// Where the app runs decides where the device token lives. On iOS a
// home-screen app has storage of its own, apart from Safari's, so a till
// paired in Safari is not paired in the installed app, and the single-use
// link is spent. Safari also clears a site's storage after 7 days without
// use, which unpairs a till kept in a tab. So on iOS the app is installed
// first and paired from the home screen. On Android and desktop, an
// installed app shares the browser's storage, and Chromium offers an install
// prompt that the page can show from a button.

import { detectPlatform, type Platform } from './platform'

/** Chromium's install prompt event, not in the DOM typings. */
interface BeforeInstallPromptEvent extends Event {
	prompt(): Promise<void>
	readonly userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>
}

export const platform: Platform = detectPlatform(navigator.userAgent, navigator.maxTouchPoints ?? 0)

/** Whether the app runs installed, from the home screen or dock. */
export const standalone: boolean =
	window.matchMedia('(display-mode: standalone)').matches ||
	(navigator as Navigator & { standalone?: boolean }).standalone === true

const state = $state<{ prompt: BeforeInstallPromptEvent | null; installed: boolean }>({
	prompt: null,
	installed: false,
})

// Chromium fires this once, early: listen from the first import, not from
// the screen that shows the button.
window.addEventListener('beforeinstallprompt', (e) => {
	e.preventDefault()
	state.prompt = e as BeforeInstallPromptEvent
})
window.addEventListener('appinstalled', () => {
	state.installed = true
	state.prompt = null
})

export const install = {
	/** The browser can install the app from a button. */
	get canPrompt(): boolean {
		return state.prompt !== null
	},
	/** The app was installed from this page. */
	get installed(): boolean {
		return state.installed
	},
	/** Shows the browser's install prompt. True when the user accepted. */
	async prompt(): Promise<boolean> {
		const prompt = state.prompt
		if (!prompt) return false
		// A prompt event works once.
		state.prompt = null
		await prompt.prompt()
		const { outcome } = await prompt.userChoice
		if (outcome === 'accepted') state.installed = true
		return outcome === 'accepted'
	},
}
