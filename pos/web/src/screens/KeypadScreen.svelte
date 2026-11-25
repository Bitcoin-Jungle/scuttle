<script lang="ts">
	// The sale screen: the store and device, the amount in the store's
	// currency or in sats, an optional note, the keypad and Charge.
	import { Pencil, History, Settings } from 'lucide-svelte'
	import type { Rate } from '$lib/api'
	import { t } from '$lib/i18n/index.svelte'
	import { applyAmountInput } from '$lib/amount-input'
	import { clearDraft, draft, type AmountUnit } from '$lib/draft.svelte'
	import {
		currencyPrecision,
		currencySymbol,
		fiatToSats,
		formatFiat,
		formatFiatAmount,
		formatSats,
		normalizeFiat,
	} from '$lib/money'
	import { client, session } from '$lib/session.svelte'
	import { startSale } from '$lib/start-sale.svelte'
	import BrandMark from '$lib/components/BrandMark.svelte'
	import Button from '$lib/components/Button.svelte'
	import IconButton from '$lib/components/IconButton.svelte'
	import Keypad from '$lib/components/Keypad.svelte'
	import Sheet from '$lib/components/Sheet.svelte'

	const RATE_REFRESH_MS = 60_000
	const MEMO_MAX = 200

	const store = $derived(session.terminal?.store ?? null)
	const currency = $derived(store?.currency ?? 'USD')

	let creating = $state(false)
	let errorMsg = $state<string | null>(null)
	let fetchedRate = $state<Rate | null>(null)
	let rateError = $state(false)
	let memoDraft = $state<string | null>(null)
	// A price in another currency is no price: after the owner changes the
	// store's currency, the old one would be off by the exchange rate.
	const rate = $derived(fetchedRate?.currency === currency ? fetchedRate : null)

	// Reloads at once when the store's currency changes, and every minute.
	$effect(() => {
		void currency
		let stopped = false
		async function load() {
			try {
				const next = await client().rate()
				if (!stopped) {
					fetchedRate = next
					rateError = false
				}
			} catch {
				if (!stopped) rateError = true
			}
		}
		void load()
		const timer = setInterval(load, RATE_REFRESH_MS)
		return () => {
			stopped = true
			clearInterval(timer)
		}
	})

	// Without a price the terminal can only charge in sats.
	const rateDown = $derived(rateError && rate === null)
	$effect(() => {
		if (rateDown && draft.unit === 'fiat') setUnit('sat')
	})

	// posd's decimal places bound what the keypad accepts.
	const precision = $derived(
		draft.unit === 'sat' ? 0 : Math.min(currencyPrecision(currency), store?.currency_decimals ?? 2),
	)
	const sats = $derived(
		draft.unit === 'sat' ? Math.trunc(Number(draft.amount || '0')) : rate ? fiatToSats(draft.amount, rate.btc_price) : 0,
	)
	const amountLabel = $derived(
		draft.unit === 'sat'
			? formatSats(Math.trunc(Number(draft.amount || '0')))
			: formatFiatAmount(draft.amount, currency, precision),
	)
	const amountSize = $derived(
		amountLabel.length > 11 ? 'text-amount-sm' : amountLabel.length > 9 ? 'text-amount-md' : 'text-amount',
	)
	// Older posd versions send no minimum: 1 sat, as before.
	const minSale = $derived(store?.min_sale_sat ?? 1)
	const belowMinimum = $derived(sats > 0 && sats < minSale)
	const estimate = $derived.by(() => {
		if (belowMinimum) return t('keypad.belowMinimum', { amount: formatSats(minSale) })
		if (!draft.amount || !rate || sats === 0) return null
		if (draft.unit === 'fiat') return t('keypad.approx', { amount: formatSats(sats) })
		return t('keypad.approx', { amount: formatFiat((sats / 1e8) * Number(rate.btc_price), rate.currency) })
	})
	const canCharge = $derived(
		sats > 0 && !belowMinimum && (draft.unit === 'sat' || rate !== null) && !creating,
	)

	function applyInput(key: string) {
		draft.amount = applyAmountInput(draft.amount, key, precision)
		errorMsg = null
	}

	function setUnit(next: AmountUnit) {
		if (draft.unit === next) return
		draft.unit = next
		draft.amount = ''
		errorMsg = null
	}

	function saveMemo() {
		draft.memo = (memoDraft ?? '').trim()
		memoDraft = null
	}

	async function charge() {
		if (!canCharge) return
		creating = true
		errorMsg = null
		const memo = draft.memo.trim() || undefined
		const body =
			draft.unit === 'sat'
				? { amount_sat: sats, memo }
				: { fiat_amount: normalizeFiat(draft.amount, precision), currency, memo }
		errorMsg = await startSale(body)
		if (errorMsg === null) clearDraft()
		creating = false
	}
</script>

<main
	class="mx-auto flex h-[100dvh] max-w-md flex-col gap-2.5 overflow-hidden px-4 pt-3 pb-[max(1rem,env(safe-area-inset-bottom))] text-ink"
>
	<header class="flex min-h-12 shrink-0 items-center gap-2.5">
		<BrandMark size={32} />
		<div class="flex min-w-0 flex-1 flex-col leading-tight">
			<span class="truncate font-semibold">{store?.name ?? t('keypad.pointOfSale')}</span>
			<span class="truncate text-[0.8125rem] text-ink-soft">{session.terminal?.device_name ?? ''}</span>
		</div>
		<IconButton label={t('header.history')} href="#/history"><History size={22} /></IconButton>
		<IconButton label={t('header.settings')} href="#/settings"><Settings size={22} /></IconButton>
	</header>

	<section class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2.5">
		<div class="flex gap-1 rounded-md bg-muted p-1" role="tablist" aria-label={t('keypad.amountUnit')}>
			{#each [['fiat', `${currencySymbol(currency)} ${currency}`], ['sat', t('keypad.sats')]] as [value, label] (value)}
				{@const disabled = value === 'fiat' && rateDown}
				<button
					type="button"
					role="tab"
					aria-selected={draft.unit === value}
					{disabled}
					class={`h-10 min-w-[76px] rounded-[calc(var(--pos-radius-md)-3px)] px-3.5 text-[0.9375rem] font-semibold ${
						draft.unit === value ? 'bg-surface text-ink' : 'text-ink-soft'
					} ${disabled ? 'line-through' : ''}`}
					onclick={() => setUnit(value as AmountUnit)}
				>
					{label}
				</button>
			{/each}
		</div>
		<p
			class={`whitespace-nowrap font-display leading-[1.1] tabular-nums tracking-display ${amountSize} ${
				draft.amount ? 'text-ink' : 'text-ink-soft'
			}`}
		>
			{amountLabel}
		</p>
		<p class="min-h-5 text-base tabular-nums text-ink-soft">{estimate ?? ' '}</p>
		<button
			type="button"
			class={`flex h-10 max-w-full items-center gap-1.5 rounded-full px-3.5 text-[0.9375rem] font-medium active:bg-muted ${
				draft.memo ? 'text-ink' : 'text-ink-soft'
			}`}
			onclick={() => (memoDraft = draft.memo)}
		>
			<Pencil size={16} class="shrink-0" />
			<span class="truncate">{draft.memo || t('keypad.addNote')}</span>
		</button>
	</section>

	{#if rateDown}
		<p role="status" class="shrink-0 rounded-md bg-warn-bg px-3.5 py-3 text-center text-[0.9375rem] leading-snug text-warn">
			{t('keypad.rateUnavailable')}
		</p>
	{/if}
	{#if errorMsg}
		<p role="alert" class="shrink-0 rounded-md bg-err-bg px-3.5 py-3 text-center text-[0.9375rem] leading-snug text-err">
			{errorMsg}
		</p>
	{/if}

	<Keypad {precision} onInput={applyInput} />

	<div class="shrink-0">
		<Button disabled={!canCharge} onclick={charge}>
			{#if creating}
				{t('keypad.preparing')}
			{:else if sats > 0}
				{t('keypad.chargeAmount', { amount: amountLabel })}
			{:else}
				{t('keypad.charge')}
			{/if}
		</Button>
	</div>
</main>

{#if memoDraft !== null}
	<Sheet label={t('keypad.note')} onclose={() => (memoDraft = null)}>
		<form
			class="flex flex-col gap-3.5"
			onsubmit={(e) => {
				e.preventDefault()
				saveMemo()
			}}
		>
			<label class="flex flex-col gap-3.5">
				<span class="font-display text-heading brand-case tracking-display">{t('keypad.note')}</span>
				<!-- svelte-ignore a11y_autofocus -->
				<input
					class="h-14 w-full rounded-md border-[1.5px] border-accent bg-surface px-3.5 text-lg text-ink outline-none"
					bind:value={memoDraft}
					placeholder={t('keypad.notePlaceholder')}
					maxlength={MEMO_MAX}
					autocomplete="off"
					autofocus
				/>
			</label>
			<p class="text-sm text-ink-soft">{t('keypad.noteHint')}</p>
			<div class="grid grid-cols-2 gap-2.5">
				<Button variant="secondary" size="md" onclick={() => (memoDraft = null)}>{t('common.cancel')}</Button>
				<Button size="md" type="submit">{t('common.save')}</Button>
			</div>
		</form>
	</Sheet>
{/if}
