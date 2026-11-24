<script lang="ts">
	// Takes a Bolt Card: reads the card with NFC while shown, hands its
	// link to posd, which has the card's service pay the sale. The payment
	// itself shows up in the sale's long-poll, like any Lightning payment.
	// The scan stops when this unmounts, and while a tap is being paid.
	import { CircleAlert, SmartphoneNfc } from 'lucide-svelte'
	import { t } from '$lib/i18n/index.svelte'
	import { ApiError } from '$lib/api'
	import { CardRead, readCard } from '$lib/bolt-card'
	import { client } from '$lib/session.svelte'
	import Spinner from './Spinner.svelte'

	let { saleId }: { saleId: string } = $props()

	/** How long after the card's service said yes a new tap is taken again. */
	const SENT_GRACE_MS = 20_000

	type Phase = 'scanning' | 'paying' | 'sent' | 'stopped'
	let phase = $state<Phase>('scanning')
	let error = $state<string | null>(null)
	/** Bumped to scan again, after NFC was turned on or allowed. */
	let attempt = $state(0)

	function cardError(e: unknown): string {
		if (e instanceof CardRead) {
			switch (e.reason) {
				case 'not-a-card':
					return t('card.notACard')
				case 'unreadable':
					return t('card.unreadable')
				case 'nfc-blocked':
					return t('card.nfcBlocked')
				case 'nfc-off':
					return t('card.nfcOff')
			}
		}
		if (e instanceof ApiError) {
			if (e.status === 422) return t('card.declined', { reason: e.message })
			if (e.status === 400) return t('card.notACard')
			if (e.status === 0) return t('card.offline')
			if (e.status === 502) return t('card.serviceDown')
		}
		return t('card.failed')
	}

	$effect(() => {
		const id = saleId
		void attempt
		const stop = new AbortController()
		const aborted = () => stop.signal.aborted
		const sleep = (ms: number) =>
			new Promise<void>((resolve) => {
				const timer = setTimeout(resolve, ms)
				stop.signal.addEventListener('abort', () => (clearTimeout(timer), resolve()), { once: true })
			})

		async function run() {
			while (!aborted()) {
				phase = 'scanning'
				let link: string
				try {
					link = await readCard(stop.signal)
				} catch (e) {
					if (aborted()) return
					error = cardError(e)
					// NFC is off or blocked: tapping again cannot help.
					if (e instanceof CardRead && (e.reason === 'nfc-off' || e.reason === 'nfc-blocked')) {
						phase = 'stopped'
						return
					}
					continue
				}
				phase = 'paying'
				error = null
				try {
					await client().payWithCard(id, link)
				} catch (e) {
					if (aborted()) return
					// 409: the sale is no longer open; the long-poll shows why.
					if (e instanceof ApiError && e.status === 409) return
					error = cardError(e)
					continue
				}
				phase = 'sent'
				await sleep(SENT_GRACE_MS)
				if (aborted()) return
				error = t('card.noPayment')
			}
		}
		void run()
		return () => stop.abort()
	})

	function retry() {
		error = null
		attempt += 1
	}
</script>

<div
	class="mx-auto flex aspect-square w-full max-w-[290px] shrink-0 flex-col items-center justify-center gap-5 rounded-lg bg-surface px-6 text-center"
	role="status"
	aria-live="polite"
>
	{#if phase === 'paying' || phase === 'sent'}
		<Spinner size={48} />
		<p class="text-[0.9375rem] font-medium">{phase === 'paying' ? t('card.paying') : t('card.sent')}</p>
	{:else}
		<div class="relative grid h-28 w-28 place-items-center">
			{#if phase === 'scanning'}<span class="ring absolute inset-0 rounded-full bg-current opacity-15"></span>{/if}
			{#if phase === 'stopped'}
				<CircleAlert size={56} class="text-err" />
			{:else}
				<SmartphoneNfc size={56} />
			{/if}
		</div>
		<p class="text-[0.9375rem] font-medium">{phase === 'stopped' ? t('card.cannotRead') : t('card.hold')}</p>
	{/if}
	{#if error}
		<p role="alert" class="rounded-md bg-err-bg px-3 py-2 text-[0.875rem] leading-snug text-err">{error}</p>
	{/if}
	{#if phase === 'stopped'}
		<button type="button" class="h-11 rounded-md bg-muted px-4 font-medium active:opacity-70" onclick={retry}>
			{t('common.tryAgain')}
		</button>
	{/if}
</div>

<style>
	.ring {
		animation: ring 1.8s ease-out infinite;
	}
	@keyframes ring {
		from {
			transform: scale(0.6);
			opacity: 0.25;
		}
		to {
			transform: scale(1.15);
			opacity: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.ring {
			animation: none;
		}
	}
</style>
