import { describe, expect, it } from 'vitest'
import { linkFromRecords } from './bolt-card'

function record(recordType: string, text: string): NDEFRecord {
	return { recordType, data: new DataView(new TextEncoder().encode(text).buffer) }
}

describe('linkFromRecords', () => {
	it('takes the URL record of a Bolt Card', () => {
		const link = 'lnurlw://card.example.com/ln?p=AB&c=CD'
		expect(linkFromRecords([record('url', link)])).toBe(link)
	})

	it('takes an LNURL from a text record', () => {
		expect(linkFromRecords([record('text', ' LNURL1DP68GURN8GHJ7 ')])).toBe('LNURL1DP68GURN8GHJ7')
		expect(linkFromRecords([record('text', 'lightning:lnurl1dp68')])).toBe('lightning:lnurl1dp68')
	})

	it('ignores tags without a link', () => {
		expect(linkFromRecords([])).toBeNull()
		expect(linkFromRecords([record('text', 'hello')])).toBeNull()
		expect(linkFromRecords([{ recordType: 'url' }])).toBeNull()
	})
})
