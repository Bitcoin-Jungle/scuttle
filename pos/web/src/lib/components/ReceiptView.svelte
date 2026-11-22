<script lang="ts">
	// A sale's receipt, on screen and on 58 mm thermal paper (see app.css).
	import type { Sale } from '$lib/api'
	import { i18n, t } from '$lib/i18n/index.svelte'
	import { formatFiat, formatSats } from '$lib/money'
	import { railLabel, receiptNumber, saleAmountLabel } from '$lib/sale'
	import StateChip from './StateChip.svelte'

	let { sale, storeName }: { sale: Sale; storeName: string } = $props()

	const rows = $derived.by(() => {
		const at = new Date(sale.paid_at ?? sale.created_at)
		const date = `${at.toLocaleDateString(i18n.tag, { dateStyle: 'medium' })} · ${at.toLocaleTimeString(i18n.tag, {
			hour: 'numeric',
			minute: '2-digit',
		})}`
		// 58 mm paper has room for the short form on one line.
		const printDate = at.toLocaleString(i18n.tag, { dateStyle: 'short', timeStyle: 'short' })
		const list: { label: string; value: string; print?: string }[] = [
			{ label: t('receipt.date'), value: date, print: printDate },
		]
		if (sale.paid_rail) {
			list.push({
				label: sale.state === 'paid' ? t('receipt.paidWith') : t('receipt.method'),
				value: railLabel(sale.paid_rail),
			})
		}
		list.push({
			label: t('receipt.inBitcoin'),
			value: formatSats(sale.state === 'paid' ? sale.received_sat : sale.amount_sat),
		})
		// A payment that arrived after the sale ended, kept by the owner's wallet.
		if (sale.state !== 'paid' && sale.received_sat > 0) {
			list.push({ label: t('receipt.received'), value: formatSats(sale.received_sat) })
		}
		return list
	})
</script>

<article class="receipt-paper flex flex-col gap-4 rounded-lg bg-surface px-5 py-[22px] text-ink">
	<header class="flex flex-col items-center gap-1 text-center">
		<h2 class="receipt-store text-[1.3125rem] font-bold">{storeName}</h2>
		<p class="receipt-small text-sm text-ink-soft">{sale.device_name ?? t('receipt.lightningAddress')}</p>
		<div class="mt-2"><StateChip state={sale.state} /></div>
	</header>
	<hr class="border-t-[1.5px] border-dashed border-line" />
	<dl class="flex flex-col gap-3 text-[0.9375rem]">
		{#each rows as row (row.label)}
			<div class="flex justify-between gap-4">
				<dt class="text-ink-soft">{row.label}</dt>
				<dd class="text-right tabular-nums">
					{#if row.print}
						<span class="receipt-screen-only">{row.value}</span><span class="receipt-print-only">{row.print}</span>
					{:else}
						{row.value}
					{/if}
				</dd>
			</div>
		{/each}
	</dl>
	{#if sale.memo}
		<hr class="border-t-[1.5px] border-dashed border-line" />
		<div class="flex flex-col gap-1 text-[0.9375rem]">
			<span class="text-ink-soft">{t('receipt.note')}</span>
			<span>{sale.memo}</span>
		</div>
	{/if}
	<hr class="border-t-[1.5px] border-dashed border-line" />
	<div class="flex items-baseline justify-between gap-4">
		<span class="receipt-total text-lg font-bold">{t('receipt.total')}</span>
		<span class="receipt-total text-[1.375rem] font-bold tabular-nums">{saleAmountLabel(sale, (a, c) => formatFiat(a, c))}</span>
	</div>
	<p class="receipt-small text-center text-[0.8125rem] tabular-nums text-ink-soft">{t('receipt.number', { number: receiptNumber(sale.id) })}</p>
</article>
