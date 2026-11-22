<script lang="ts">
	// The device's sales from posd, a page at a time, with a CSV export.
	import { Download } from 'lucide-svelte'
	import { t } from '$lib/i18n/index.svelte'
	import type { Sale } from '$lib/api'
	import { isoDate } from '$lib/sale'
	import { client } from '$lib/session.svelte'
	import Button from '$lib/components/Button.svelte'
	import SalesList from '$lib/components/SalesList.svelte'
	import ScreenHeader from '$lib/components/ScreenHeader.svelte'
	import Sheet from '$lib/components/Sheet.svelte'

	const PAGE = 50

	let rows = $state<Sale[]>([])
	let loaded = $state(false)
	let loading = $state(true)
	let more = $state(false)
	let errorMsg = $state<string | null>(null)

	const today = new Date()
	let exportOpen = $state(false)
	let from = $state(isoDate(new Date(today.getFullYear(), today.getMonth(), 1)))
	let to = $state(isoDate(today))
	let exporting = $state(false)
	let exportError = $state<string | null>(null)

	async function load(before?: string) {
		loading = true
		errorMsg = null
		try {
			const page = await client().sales({ before, limit: PAGE })
			rows = before ? [...rows, ...page] : page
			more = page.length === PAGE
			loaded = true
		} catch {
			errorMsg = t('history.loadFailed')
		} finally {
			loading = false
		}
	}

	void load()

	async function exportCsv() {
		if (exporting || !from || !to) return
		exporting = true
		exportError = null
		try {
			const blob = await client().salesCsv(from, to)
			const url = URL.createObjectURL(blob)
			const a = document.createElement('a')
			a.href = url
			a.download = `sales-${from}-to-${to}.csv`
			document.body.appendChild(a)
			a.click()
			a.remove()
			setTimeout(() => URL.revokeObjectURL(url), 1000)
			exportOpen = false
		} catch {
			exportError = t('history.exportFailed')
		} finally {
			exporting = false
		}
	}
</script>

<main
	class="mx-auto flex h-[100dvh] max-w-md flex-col gap-3.5 px-4 pt-3 pb-[max(1rem,env(safe-area-inset-bottom))] text-ink"
>
	<ScreenHeader title={t('history.title')} back="#/">
		{#snippet action()}
			{#if rows.length > 0}
				<button
					type="button"
					class="flex h-11 items-center gap-1.5 rounded-md bg-muted px-3.5 text-[0.9375rem] font-medium active:opacity-70"
					onclick={() => (exportOpen = true)}
				>
					<Download size={18} />{t('history.export')}
				</button>
			{/if}
		{/snippet}
	</ScreenHeader>

	{#if errorMsg}
		<p role="alert" class="rounded-md bg-err-bg px-3.5 py-3 text-center text-[0.9375rem] text-err">{errorMsg}</p>
	{/if}

	{#if rows.length > 0}
		<div class="flex min-h-0 flex-1 flex-col gap-3.5 overflow-y-auto">
			<SalesList {rows} />
			{#if more}
				<Button variant="secondary" size="md" disabled={loading} onclick={() => load(rows[rows.length - 1]?.id)}>
					{loading ? t('common.loading') : t('history.loadMore')}
				</Button>
			{/if}
		</div>
	{:else if loaded}
		<div class="flex flex-1 flex-col items-center justify-center gap-2 pb-[60px] text-center">
			<p class="font-display text-heading brand-case tracking-display">{t('history.emptyTitle')}</p>
			<p class="text-ink-soft">{t('history.empty')}</p>
		</div>
	{/if}
</main>

{#if exportOpen}
	<Sheet label={t('history.exportTitle')} onclose={() => (exportOpen = false)}>
		<h2 class="font-display text-heading brand-case tracking-display">{t('history.exportTitle')}</h2>
		<div class="grid grid-cols-2 gap-2.5">
			<label class="flex min-w-0 flex-col gap-1.5 text-sm text-ink-soft">
				{t('history.from')}
				<input type="date" bind:value={from} class="h-[52px] w-full rounded-md border-[1.5px] border-line bg-surface px-2.5 text-base text-ink" />
			</label>
			<label class="flex min-w-0 flex-col gap-1.5 text-sm text-ink-soft">
				{t('history.to')}
				<input type="date" bind:value={to} class="h-[52px] w-full rounded-md border-[1.5px] border-line bg-surface px-2.5 text-base text-ink" />
			</label>
		</div>
		{#if exportError}
			<p role="alert" class="text-center text-[0.9375rem] text-err">{exportError}</p>
		{/if}
		<Button disabled={exporting || !from || !to} onclick={exportCsv}>
			<Download size={20} />{exporting ? t('history.exporting') : t('history.exportCsv')}
		</Button>
		<Button variant="ghost" size="md" onclick={() => (exportOpen = false)}>{t('common.cancel')}</Button>
	</Sheet>
{/if}
