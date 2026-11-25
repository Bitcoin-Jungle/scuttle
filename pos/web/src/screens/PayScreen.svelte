<script lang="ts">
	// The payment screen: the amount, a QR code for the rail the payer picks,
	// and the payment state, kept current with posd's long-poll.
	import { untrack } from 'svelte'
	import { Bitcoin, Clock, Copy, Nfc, Search, Share2, X, Zap } from 'lucide-svelte'
	import { t } from '$lib/i18n/index.svelte'
	import { ApiError, type PaidRail, type Sale } from '$lib/api'
	import { restoreDraft } from '$lib/draft.svelte'
	import { formatFiat, formatSats } from '$lib/money'
	import { router } from '$lib/router.svelte'
	import { countdown, isLive, lightningQr, onchainQr, onchainSeen, partialPayment, railLabel, saleAmountLabel } from '$lib/sale'
	import { nfcSupported } from '$lib/bolt-card'
	import { retryDelay } from '$lib/retry'
	import { cachedSale, rememberSale } from '$lib/sale-cache'
	import { client, session } from '$lib/session.svelte'
	import { startSale } from '$lib/start-sale.svelte'
	import Spinner from '$lib/components/Spinner.svelte'
	import CardTap from '$lib/components/CardTap.svelte'
	import PaidScreen from '$lib/components/PaidScreen.svelte'
	import QrCard from '$lib/components/QrCard.svelte'
	import StatusScreen from '$lib/components/StatusScreen.svelte'

	let { id }: { id: string } = $props()

	let sale = $state<Sale | null>(untrack(() => cachedSale(id) ?? null))
	let notFound = $state(false)
	let offline = $state(false)
	let canceling = $state(false)
	let cancelError = $state<string | null>(null)
	let nowMs = $state(Date.now())
	/** What the payer pays with: a rail's code, or a Bolt Card, which pays by Lightning. */
	type Method = PaidRail | 'card'
	let rail = $state<Method>('lightning')
	// Only Chrome on Android reads NFC: elsewhere there is no card tab.
	const cards = nfcSupported()
	// Three tabs leave no room for icons on a narrow phone.
	const iconClass = cards ? 'shrink-0 max-[400px]:hidden' : 'shrink-0'
	let copied = $state(false)
	let gettingOnchain = $state(false)
	let onchainError = $state<string | null>(null)

	// Long-poll until the sale leaves its live states. Each request names the
	// state and the on-chain sats the terminal last saw, so after any drop the
	// next one returns the current sale at once and no change, a partial
	// payment included, is missed. On a network error the
	// loop retries with backoff. When the device wakes or comes back online,
	// it drops the request in flight, which a sleeping phone may have left
	// hanging, and asks again at once.
	$effect(() => {
		let stopped = false
		let request: AbortController | null = null
		let wake: (() => void) | null = null

		function nudge() {
			request?.abort()
			wake?.()
		}
		function onVisibility() {
			if (document.visibilityState === 'visible') nudge()
		}
		function onOffline() {
			offline = true
			nudge()
		}
		document.addEventListener('visibilitychange', onVisibility)
		window.addEventListener('online', nudge)
		window.addEventListener('offline', onOffline)

		function pause(ms: number): Promise<void> {
			return new Promise<void>((resolve) => {
				const timer = setTimeout(resolve, ms)
				wake = () => {
					clearTimeout(timer)
					resolve()
				}
			}).then(() => {
				wake = null
			})
		}

		async function run() {
			let current = untrack(() => sale)
			let failures = 0
			while (!stopped) {
				if (current && !isLive(current.state)) return
				const attempt = new AbortController()
				request = attempt
				try {
					const next = current
						? await client().waitSale(id, current.state, onchainSeen(current), attempt.signal)
						: await client().sale(id, attempt.signal)
					offline = false
					failures = 0
					current = next
					sale = next
					rememberSale(next)
				} catch (e) {
					if (stopped) return
					// Dropped by a wake-up: ask again now.
					if (attempt.signal.aborted && !(e instanceof ApiError)) continue
					if (e instanceof ApiError && e.status === 404) {
						notFound = true
						return
					}
					offline = e instanceof ApiError && e.status === 0
					failures += 1
					await pause(retryDelay(failures))
				}
			}
		}
		void run()
		return () => {
			stopped = true
			request?.abort()
			wake?.()
			document.removeEventListener('visibilitychange', onVisibility)
			window.removeEventListener('online', nudge)
			window.removeEventListener('offline', onOffline)
		}
	})

	$effect(() => {
		const timer = setInterval(() => (nowMs = Date.now()), 1000)
		return () => clearInterval(timer)
	})


	const amountLabel = $derived(sale ? saleAmountLabel(sale, (a, c) => formatFiat(a, c)) : '')
	const remaining = $derived(sale ? countdown(sale.expires_at, nowMs) : null)
	/** The sats under a fiat amount, or the fiat estimate under sats, and the note. */
	const amountDetail = $derived.by(() => {
		if (!sale) return ''
		const parts: string[] = []
		if (sale.fiat_amount !== null) parts.push(formatSats(sale.amount_sat))
		if (sale.memo) parts.push(sale.memo)
		return parts.join(' · ')
	})
	const onchain = $derived(sale ? onchainQr(sale) : null)
	/** Where on-chain starts, in the sale's currency when it has one. */
	const onchainFrom = $derived.by(() => {
		const min = session.terminal?.store.onchain_min_sat
		if (!sale || min === undefined) return ''
		if (sale.btc_price && sale.currency) return formatFiat((min / 1e8) * Number(sale.btc_price), sale.currency)
		return formatSats(min)
	})
	// No code under On-chain until the address arrives: showing the
	// Lightning code there would have the payer scan the wrong one.
	const qrValue = $derived(sale ? (rail === 'onchain' ? onchain : rail === 'card' ? null : lightningQr(sale)) : null)
	const partial = $derived(sale ? partialPayment(sale) : null)

	// The payment for the shown rail, as a wallet takes it pasted or shared:
	// the invoice, or a BIP 21 URI so the amount comes along.
	const paymentText = $derived(
		!sale || rail === 'card' ? '' : rail === 'onchain' ? (sale.onchain_uri ?? '') : sale.lightning_invoice,
	)

	/**
	 * Shows a rail. On-chain gets its address now, the first time: posd
	 * makes one only for a sale the payer may pay on-chain, so addresses the
	 * owner's wallet must scan for are not used up by Lightning sales.
	 */
	async function pick(next: Method) {
		rail = next
		copied = false
		onchainError = null
		if (next !== 'onchain' || !sale || sale.onchain_address || gettingOnchain) return
		gettingOnchain = true
		try {
			const withAddress = await client().addOnchain(sale.id)
			rememberSale(withAddress)
			sale = withAddress
		} catch (e) {
			rail = 'lightning'
			onchainError = e instanceof ApiError && e.status === 409 ? t('pay.onchainRefused') : t('pay.onchainFailed')
		} finally {
			gettingOnchain = false
		}
	}
	// The share sheet hands the payment to a wallet on the device, or to a
	// messenger for a payer who is not at the counter. Without it (most
	// desktop browsers), the button copies.
	const canShare = typeof navigator.share === 'function'

	async function share() {
		if (!paymentText) return
		if (canShare) {
			try {
				await navigator.share({ text: paymentText })
			} catch {
				/* dismissed */
			}
			return
		}
		try {
			await navigator.clipboard.writeText(paymentText)
			copied = true
			setTimeout(() => (copied = false), 1500)
		} catch {
			/* clipboard unavailable */
		}
	}

	function newSale() {
		router.go('/')
	}

	let retrying = $state(false)
	let retryError = $state<string | null>(null)

	/** Charges the same amount and note again, with a new payment code. */
	async function tryAgain() {
		if (!sale || retrying) return
		retrying = true
		retryError = null
		const memo = sale.memo ?? undefined
		// The amount as posd stored it, in the currency the sale was in.
		const body =
			sale.fiat_amount !== null && sale.currency
				? { fiat_amount: sale.fiat_amount, currency: sale.currency, memo }
				: { amount_sat: sale.amount_sat, memo }
		retryError = await startSale(body)
		retrying = false
	}

	async function cancel() {
		// Part of the amount arrived on-chain, so posd keeps the sale: it
		// stays open for the rest until it expires. Leave it as it is.
		if (!sale || sale.state !== 'open' || partial) {
			newSale()
			return
		}
		canceling = true
		cancelError = null
		try {
			const next = await client().cancelSale(sale.id)
			rememberSale(next)
			sale = next
			// Back to the keypad with the sale, to fix it or charge it again.
			restoreDraft(next)
			newSale()
		} catch (e) {
			// A payment arrived first: the long-poll shows it. Otherwise the
			// sale may still be open, and its code still pays, so stay here:
			// leaving would let staff charge again while the customer pays.
			if (!(e instanceof ApiError && e.status === 409)) {
				cancelError = e instanceof ApiError && e.status === 0 ? t('pay.cancelOffline') : t('pay.cancelFailed')
			} else {
				// posd refused because the sale moved on: a payment, or part
				// of one on-chain. Show what it is now rather than nothing.
				try {
					const fresh = await client().sale(sale.id)
					rememberSale(fresh)
					sale = fresh
					if (fresh.state === 'open' && !partialPayment(fresh)) cancelError = t('pay.cancelFailed')
				} catch {
					cancelError = t('pay.cancelFailed')
				}
			}
		} finally {
			canceling = false
		}
	}
</script>

{#if !notFound && (!sale || sale.state === 'open')}
	<main
		class="mx-auto flex min-h-[100dvh] max-w-md flex-col gap-3.5 px-4 pt-3 pb-[max(1rem,env(safe-area-inset-bottom))] text-ink"
	>
		<header class="flex min-h-12 shrink-0 items-center justify-between">
			<button
				type="button"
				class="flex h-11 items-center gap-1.5 rounded-md bg-muted pr-3.5 pl-2.5 font-medium active:opacity-70 disabled:opacity-50"
				onclick={cancel}
				disabled={canceling}
			>
				<X size={20} />{partial ? t('pay.leave') : t('common.cancel')}
			</button>
			{#if remaining}
				<p class="flex items-center gap-1.5 font-medium tabular-nums text-ink-soft" aria-label={t('pay.timeLeft')}>
					<Clock size={18} />{remaining}
				</p>
			{/if}
		</header>

		<div class="flex flex-col items-center gap-0.5 text-center">
			<p class="font-display text-sale leading-[1.1] tabular-nums tracking-display">{amountLabel || '\u00a0'}</p>
			<p class="text-[0.9375rem] tabular-nums text-ink-soft">{amountDetail || '\u00a0'}</p>
		</div>

		<div class={`grid shrink-0 gap-1 rounded-lg bg-muted p-1 ${cards ? 'grid-cols-3' : 'grid-cols-2'}`} role="tablist" aria-label={t('pay.method')}>
			{#each [{ id: 'lightning', label: railLabel('lightning'), sub: t('pay.instant'), ok: true }, { id: 'onchain', label: railLabel('onchain'), sub: !sale || sale.onchain_available ? t('pay.onchainTime') : t('pay.onchainFrom', { amount: onchainFrom }), ok: !!sale?.onchain_available }, ...(cards ? [{ id: 'card', label: t('card.tab'), sub: t('card.tabSub'), ok: true }] : [])] as m (m.id)}
				<button
					type="button"
					role="tab"
					aria-selected={rail === m.id}
					disabled={!sale || !m.ok}
					class={`flex h-[58px] items-center justify-center gap-2 rounded-md px-2 ${
						rail === m.id ? 'bg-surface text-ink' : 'text-ink-soft'
					} ${m.ok ? '' : 'opacity-60'}`}
					onclick={() => pick(m.id as Method)}
				>
					{#if m.id === 'lightning'}<Zap size={20} class={iconClass} fill="currentColor" />{:else if m.id === 'card'}<Nfc size={20} class={iconClass} />{:else}<Bitcoin size={20} class={iconClass} />{/if}
					<span class="flex min-w-0 flex-col items-start leading-tight">
						<span class="font-semibold">{m.label}</span>
						<span class="max-w-full truncate text-[0.78rem] text-ink-soft">{m.sub}</span>
					</span>
				</button>
			{/each}
		</div>

		{#if rail === 'card' && sale}
			<CardTap saleId={sale.id} />
		{:else}
			<QrCard value={qrValue} label={rail === 'onchain' ? t('pay.qrOnchain') : t('pay.qrLightning')}>
				<div class="flex flex-col items-center gap-3.5 text-[0.9375rem] text-ink-soft">
					<Spinner size={40} />
					{rail === 'onchain' && sale ? t('pay.gettingAddress') : t('pay.creating')}
				</div>
			</QrCard>
		{/if}

		<div class="flex flex-col items-center gap-1.5 text-center">
			<p class="font-medium">{sale ? (rail === 'onchain' ? t('pay.scanOnchain') : rail === 'card' ? t('card.prompt') : t('pay.scanLightning')) : '\u00a0'}</p>
			<p role="status" class={`flex items-center gap-2 text-[0.9375rem] ${offline ? 'text-warn' : 'text-ink-soft'}`}>
				<span class="pulse h-[9px] w-[9px] shrink-0 rounded-full bg-current"></span>
				{!sale ? t('pay.gettingReady') : offline ? t('pay.reconnecting') : t('pay.waiting')}
			</p>
			{#if partial}
				<p role="status" class="max-w-[300px] rounded-md bg-warn-bg px-3.5 py-3 text-[0.9375rem] leading-snug text-warn">
					{t('pay.partial', { seen: formatSats(partial.seen), owed: formatSats(partial.owed) })}
					{t('pay.partialStaysOpen')}
				</p>
			{/if}
			{#if onchainError}
				<p role="alert" class="max-w-[300px] rounded-md bg-err-bg px-3.5 py-3 text-[0.9375rem] leading-snug text-err">
					{onchainError}
				</p>
			{/if}
			{#if cancelError}
				<p role="alert" class="max-w-[300px] rounded-md bg-err-bg px-3.5 py-3 text-[0.9375rem] leading-snug text-err">
					{cancelError}
				</p>
			{/if}
		</div>

		<div class="flex-1"></div>
		{#if sale && paymentText}
			<button
				type="button"
				class="flex h-11 items-center gap-2 self-center rounded-md px-4 text-[0.9375rem] font-medium text-ink-soft active:bg-muted"
				onclick={share}
			>
				{#if canShare}
					<Share2 size={18} />{rail === 'onchain' ? t('pay.shareAddress') : t('pay.shareInvoice')}
				{:else}
					<Copy size={18} />{copied ? t('common.copied') : rail === 'onchain' ? t('pay.copyAddress') : t('pay.copyInvoice')}
				{/if}
			</button>
		{/if}
	</main>
{:else if notFound}
	<StatusScreen
		title={t('pay.notFoundTitle')}
		body={t('pay.notFoundBody')}
		primary={t('pay.newSale')}
		onprimary={newSale}
		secondary={t('pay.openSales')}
		onsecondary={() => router.go('/history')}
	>
		{#snippet icon()}<Search size={34} />{/snippet}
	</StatusScreen>
{:else if sale?.state === 'paid'}
	<PaidScreen
		{amountLabel}
		meta={`${railLabel(sale.paid_rail)} · ${formatSats(sale.received_sat)}`}
		note={sale.received_sat > sale.amount_sat
			? t('pay.overpaid', { received: formatSats(sale.received_sat), extra: formatSats(sale.received_sat - sale.amount_sat) })
			: undefined}
		onnewsale={newSale}
		// Replace this finished sale in history, so the receipt's Back
		// returns to the keypad rather than to this screen.
		onreceipt={() => router.replace(`/receipt/${id}`)}
	/>
{:else if sale?.state === 'in-progress'}
	<StatusScreen
		tone="warn"
		title={t('pay.onItsWayTitle')}
		amount={`${amountLabel} · ${railLabel('onchain')}`}
		body={t('pay.onItsWayBody')}
		primary={t('pay.newSale')}
		onprimary={newSale}
		secondary={t('pay.viewInSales')}
		onsecondary={() => router.go('/history')}
	>
		{#snippet icon()}<Clock size={36} />{/snippet}
	</StatusScreen>
{:else if sale?.state === 'expired'}
	<StatusScreen
		title={t('pay.expiredTitle')}
		amount={amountLabel}
		body={t('pay.expiredBody')}
		error={retryError}
		busy={retrying}
		primary={t('common.tryAgain')}
		onprimary={tryAgain}
		secondary={t('pay.newSale')}
		onsecondary={newSale}
	>
		{#snippet icon()}<Clock size={36} />{/snippet}
	</StatusScreen>
{:else if sale?.state === 'canceled'}
	<StatusScreen
		title={t('pay.canceledTitle')}
		amount={amountLabel}
		body={t('pay.canceledBody')}
		primary={t('pay.newSale')}
		onprimary={newSale}
	>
		{#snippet icon()}<X size={34} strokeWidth={2.4} />{/snippet}
	</StatusScreen>
{/if}

<style>
	.pulse {
		animation: pulse 1.6s ease-in-out infinite;
	}
	@keyframes pulse {
		50% {
			opacity: 0.25;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.pulse {
			animation: none;
		}
	}
</style>
