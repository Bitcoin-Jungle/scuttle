// Reading a Bolt Card with Web NFC, which only Chrome on Android has. The
// terminal only reads the card's link and hands it to posd, which has the
// card's service pay the sale.

/** Whether this browser can read NFC cards. */
export function nfcSupported(): boolean {
	return typeof window !== 'undefined' && 'NDEFReader' in window
}

/** Why a card could not be read. */
export type CardReadError = 'not-a-card' | 'unreadable' | 'nfc-blocked' | 'nfc-off'

export class CardRead extends Error {
	constructor(readonly reason: CardReadError) {
		super(reason)
		this.name = 'CardRead'
	}
}

/**
 * The link in a tag's records: the first URL record, as Bolt Cards hold, or
 * a text record with an LNURL. Null when the tag has neither.
 */
export function linkFromRecords(records: readonly NDEFRecord[]): string | null {
	for (const r of records) {
		if (!r.data) continue
		if (r.recordType === 'url' || r.recordType === 'absolute-url') {
			return new TextDecoder().decode(r.data).trim()
		}
		if (r.recordType === 'text') {
			const text = new TextDecoder(r.encoding || 'utf-8').decode(r.data).trim()
			if (/^(lightning:)?lnurl|^lnurlw:\/\//i.test(text)) return text
		}
	}
	return null
}

/**
 * Waits for a card and resolves with its link. Rejects with CardRead, or
 * with an AbortError once `signal` aborts. Callers abort as soon as they
 * stop showing the card tab: a scan must not run in the background.
 */
export function readCard(signal: AbortSignal): Promise<string> {
	return new Promise((resolve, reject) => {
		if (!nfcSupported()) return reject(new CardRead('nfc-off'))
		if (signal.aborted) return reject(new DOMException('Aborted', 'AbortError'))
		const reader = new NDEFReader()
		let done = false
		const finish = (f: () => void) => {
			if (done) return
			done = true
			reader.onreading = null
			reader.onreadingerror = null
			signal.removeEventListener('abort', onAbort)
			f()
		}
		const onAbort = () => finish(() => reject(new DOMException('Aborted', 'AbortError')))
		signal.addEventListener('abort', onAbort, { once: true })
		reader.onreading = (event) => {
			const link = linkFromRecords(event.message.records)
			finish(() => (link ? resolve(link) : reject(new CardRead('not-a-card'))))
		}
		reader.onreadingerror = () => finish(() => reject(new CardRead('unreadable')))
		reader.scan({ signal }).catch((e: unknown) => {
			const name = e instanceof DOMException ? e.name : ''
			if (name === 'AbortError') return finish(() => reject(e))
			finish(() => reject(new CardRead(name === 'NotAllowedError' ? 'nfc-blocked' : 'nfc-off')))
		})
	})
}
