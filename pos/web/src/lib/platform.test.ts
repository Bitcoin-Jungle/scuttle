import { describe, expect, it } from 'vitest'
import { detectPlatform, isIosSafari } from './platform'

const IPHONE_SAFARI =
	'Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1'
const IPHONE_CHROME =
	'Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/131.0.6778.73 Mobile/15E148 Safari/604.1'
const IPAD_DESKTOP_MODE =
	'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15'
const ANDROID_CHROME =
	'Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36'
const WINDOWS_CHROME =
	'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'

describe('install platform', () => {
	it('tells iOS, Android and desktop apart', () => {
		expect(detectPlatform(IPHONE_SAFARI, 5)).toBe('ios')
		expect(detectPlatform(IPHONE_CHROME, 5)).toBe('ios')
		expect(detectPlatform(ANDROID_CHROME, 5)).toBe('android')
		expect(detectPlatform(WINDOWS_CHROME, 0)).toBe('desktop')
	})

	it('sees an iPad through its desktop user agent by its touch screen', () => {
		expect(detectPlatform(IPAD_DESKTOP_MODE, 5)).toBe('ios')
		expect(detectPlatform(IPAD_DESKTOP_MODE, 0)).toBe('desktop')
	})

	it('knows Safari from the other iOS browsers', () => {
		expect(isIosSafari(IPHONE_SAFARI)).toBe(true)
		expect(isIosSafari(IPHONE_CHROME)).toBe(false)
	})
})
