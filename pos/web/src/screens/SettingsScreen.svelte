<script lang="ts">
	// The store's setup as this device sees it. The owner manages the store
	// from the wallet, so the device can only pick its language and remove
	// itself.
	import { ChevronDown } from 'lucide-svelte'
	import type { Rate } from '$lib/api'
	import { LOCALES, i18n, setLocalePreference, t, type LocalePreference } from '$lib/i18n/index.svelte'
	import { currencyName, currencySymbol, formatFiat, formatRate, formatSats } from '$lib/money'
	import { client, session } from '$lib/session.svelte'
	import { brand } from '$lib/brand.svelte'
	import BrandMark from '$lib/components/BrandMark.svelte'
	import Button from '$lib/components/Button.svelte'
	import ScreenHeader from '$lib/components/ScreenHeader.svelte'
	import Sheet from '$lib/components/Sheet.svelte'

	const store = $derived(session.terminal?.store ?? null)

	let rate = $state<Rate | null>(null)
	let confirming = $state(false)

	$effect(() => {
		client()
			.rate()
			.then((r) => (rate = r))
			.catch(() => {})
	})

	const rows = $derived.by(() => {
		if (!store) return []
		const onchainFrom = rate
			? `${formatSats(store.onchain_min_sat)} (${formatFiat((store.onchain_min_sat / 1e8) * Number(rate.btc_price), rate.currency)})`
			: formatSats(store.onchain_min_sat)
		return [
			{ label: t('settings.store'), value: store.name },
			{ label: t('settings.device'), value: session.terminal?.device_name ?? '—' },
			{ label: t('settings.currency'), value: `${currencyName(store.currency)} (${currencySymbol(store.currency)})` },
			{ label: t('settings.rate'), value: rate ? `1 BTC = ${formatRate(rate.btc_price, rate.currency).replace(/\/BTC$/, '')}` : '—' },
			{ label: t('settings.lightningAddress'), value: store.lightning_address ?? t('settings.notSet') },
			{ label: t('settings.onchainFrom'), value: onchainFrom },
		]
	})

	function remove() {
		session.signOut()
		location.hash = '#/'
	}
</script>

<main
	class="mx-auto flex min-h-[100dvh] max-w-md flex-col gap-3.5 px-4 pt-3 pb-[max(1rem,env(safe-area-inset-bottom))] text-ink"
>
	<ScreenHeader title={t('settings.title')} back="#/" />

	<div class="flex flex-col gap-2">
		<dl class="overflow-hidden rounded-lg bg-surface text-[0.9375rem]">
			{#each rows as row, i (row.label)}
				<div class={`flex min-h-[54px] items-center justify-between gap-4 px-4 py-2 ${i > 0 ? 'border-t border-line' : ''}`}>
					<dt class="min-w-[5.5rem] text-ink-soft">{row.label}</dt>
					<dd class="min-w-0 text-right font-semibold tabular-nums [overflow-wrap:anywhere]">{row.value}</dd>
				</div>
			{/each}
		</dl>
		<p class="px-1 text-sm leading-snug text-ink-soft">{t('settings.ownerNote', { wallet: brand.wallet })}</p>
	</div>

	<label class="flex min-h-[54px] items-center justify-between gap-4 rounded-lg bg-surface px-4 text-[0.9375rem]">
		<span class="shrink-0 text-ink-soft">{t('settings.language')}</span>
		<span class="relative min-w-0">
			<select
				class="w-full appearance-none truncate bg-transparent py-3 pr-6 text-right font-semibold text-ink outline-none"
				value={i18n.preference}
				onchange={(e) => setLocalePreference(e.currentTarget.value as LocalePreference)}
			>
				<option value="auto">{t('settings.languageAuto', { language: LOCALES[i18n.deviceLocale].name })}</option>
				{#each Object.entries(LOCALES) as [code, locale] (code)}
					<option value={code}>{locale.name}</option>
				{/each}
			</select>
			<ChevronDown size={18} class="pointer-events-none absolute top-1/2 right-0 -translate-y-1/2 text-ink-soft" />
		</span>
	</label>

	<div class="flex-1"></div>
	<Button variant="danger-soft" size="md" onclick={() => (confirming = true)}>{t('settings.remove')}</Button>
	<p class="flex items-center justify-center gap-2 text-[0.8125rem] text-ink-soft">
		<BrandMark size={20} />{brand.info.name}
	</p>
</main>

{#if confirming}
	<Sheet label={t('settings.removeTitle')} onclose={() => (confirming = false)}>
		<h2 class="font-display text-heading brand-case tracking-display">{t('settings.removeTitle')}</h2>
		<p class="leading-normal text-ink-soft">{t('settings.removeNote')}</p>
		<Button variant="danger" onclick={remove}>{t('settings.removeAction')}</Button>
		<Button variant="ghost" size="md" onclick={() => (confirming = false)}>{t('settings.keep')}</Button>
	</Sheet>
{/if}
