// Web NFC (Android Chrome), which TypeScript's DOM types leave out. Code
// checks `'NDEFReader' in window` before it uses it.

interface NDEFRecord {
	recordType: string
	mediaType?: string
	id?: string
	data?: DataView
	encoding?: string
	lang?: string
}

interface NDEFMessage {
	records: NDEFRecord[]
}

interface NDEFReadingEvent extends Event {
	serialNumber: string
	message: NDEFMessage
}

interface NDEFReader extends EventTarget {
	scan(options?: { signal?: AbortSignal }): Promise<void>
	onreading: ((this: NDEFReader, ev: NDEFReadingEvent) => void) | null
	onreadingerror: ((this: NDEFReader, ev: Event) => void) | null
}

declare const NDEFReader: {
	new (): NDEFReader
}
