<script lang="ts">
	// Terminal shell. "/pair/<code>" activates the device; once it has a
	// token, the app is hash-routed.
	import { ApiError } from '$lib/api'
	import { t } from '$lib/i18n/index.svelte'
	import { router } from '$lib/router.svelte'
	import { client, pairingCodeFromPath, session } from '$lib/session.svelte'
	import { CircleAlert } from 'lucide-svelte'
	import Button from '$lib/components/Button.svelte'
	import Spinner from '$lib/components/Spinner.svelte'
	import { applyBrand } from '$lib/brand.svelte'
	import HistoryScreen from './screens/HistoryScreen.svelte'
	import KeypadScreen from './screens/KeypadScreen.svelte'
	import PairScreen from './screens/PairScreen.svelte'
	import PayScreen from './screens/PayScreen.svelte'
	import ReceiptScreen from './screens/ReceiptScreen.svelte'
	import SettingsScreen from './screens/SettingsScreen.svelte'
	import UnpairedScreen from './screens/UnpairedScreen.svelte'

	const pairingCode = pairingCodeFromPath(location.pathname)

	// An installed app always opens at its start_url, which carries the
	// pairing code. A device that is already paired ignores the code.
	if (session.token && pairingCode) history.replaceState(null, '', `/${location.hash || '#/'}`)

	let loadError = $state<'unreachable' | 'failed' | null>(null)

	$effect(() => {
		const token = session.token
		if (!token) return
		let current = true
		loadError = null
		client()
			.terminal()
			.then((t) => {
				if (!current) return
				session.terminal = t
				applyBrand(t.store.brand)
			})
			.catch((e) => {
				if (current && !(e instanceof ApiError && e.isUnauthorized)) {
					loadError = e instanceof ApiError && e.status === 0 ? 'unreachable' : 'failed'
				}
			})
		return () => {
			current = false
		}
	})

	const payId = $derived(router.match('/pay/:id')?.id)
	const receiptId = $derived(router.match('/receipt/:id')?.id)
</script>

{#if !session.token}
	{#if pairingCode}
		<PairScreen code={pairingCode} />
	{:else}
		<UnpairedScreen />
	{/if}
{:else if !session.terminal}
	{#if loadError}
		<main class="mx-auto flex min-h-[100dvh] max-w-md flex-col px-4 pt-4 pb-[max(1rem,env(safe-area-inset-bottom))] text-ink">
			<div class="flex flex-1 flex-col items-center justify-center gap-3 text-center">
				<div class="grid h-[72px] w-[72px] place-items-center rounded-full bg-muted text-ink-soft">
					<CircleAlert size={32} />
				</div>
				<h1 class="mt-1 font-display text-title brand-case tracking-display">
					{loadError === 'unreachable' ? t('app.unreachableTitle') : t('app.loadFailedTitle')}
				</h1>
				<p class="max-w-[280px] leading-normal text-ink-soft">
					{loadError === 'unreachable' ? t('app.unreachable') : t('app.loadFailed')}
				</p>
			</div>
			<Button onclick={() => location.reload()}>{t('common.tryAgain')}</Button>
		</main>
	{:else}
		<main class="grid min-h-[100dvh] place-items-center text-ink">
			<Spinner size={72} />
		</main>
	{/if}
{:else if payId}
	{#key payId}
		<PayScreen id={payId} />
	{/key}
{:else if receiptId}
	{#key receiptId}
		<ReceiptScreen id={receiptId} />
	{/key}
{:else if router.path === '/history'}
	<HistoryScreen />
{:else if router.path === '/settings'}
	<SettingsScreen />
{:else}
	<KeypadScreen />
{/if}
