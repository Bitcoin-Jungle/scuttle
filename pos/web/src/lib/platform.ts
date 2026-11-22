// Which install steps a browser needs, from its user agent.

export type Platform = 'ios' | 'android' | 'desktop'

/** The platform, for which install steps to show. */
export function detectPlatform(ua: string, maxTouchPoints: number): Platform {
	if (/iPhone|iPad|iPod/.test(ua)) return 'ios'
	// iPadOS asks for desktop sites and says it is a Mac, but has touch.
	if (/Macintosh/.test(ua) && maxTouchPoints > 1) return 'ios'
	if (/Android/.test(ua)) return 'android'
	return 'desktop'
}

/** Whether an iOS browser is Safari. The others name themselves. */
export function isIosSafari(ua: string): boolean {
	return !/CriOS|FxiOS|EdgiOS|OPiOS|GSA\//.test(ua)
}
