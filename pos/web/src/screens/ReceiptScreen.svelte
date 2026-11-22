<script lang="ts">
	// A sale's receipt, with Print once it is paid. Receipts print on 58 mm
	// thermal paper.
	import { untrack } from 'svelte'
	import { Printer } from 'lucide-svelte'
	import { t } from '$lib/i18n/index.svelte'
	import { ApiError, type Sale } from '$lib/api'
	import { router } from '$lib/router.svelte'
	import { cachedSale, rememberSale } from '$lib/sale-cache'
	import { client, session } from '$lib/session.svelte'
	import Button from '$lib/components/Button.svelte'
	import ReceiptView from '$lib/components/ReceiptView.svelte'
	import ScreenHeader from '$lib/components/ScreenHeader.svelte'

	let { id }: { id: string } = $props()

	let sale = $state<Sale | null>(untrack(() => cachedSale(id) ?? null))
	let notFound = $state(false)
	let loadError = $state(false)
	let attempt = $state(0)

	$effect(() => {
		void attempt
		let current = true
		loadError = false
		client()
			.sale(id)
			.then((s) => {
				if (!current) return
				rememberSale(s)
				sale = s
			})
			.catch((e) => {
				if (!current) return
				if (e instanceof ApiError && e.status === 404) notFound = true
				else loadError = true
			})
		return () => {
			current = false
		}
	})

	/** Back where the receipt was opened from, or to Sales on a fresh start. */
	function back() {
		if (history.length > 1) history.back()
		else router.go('/history')
	}

	// Size the printed page to the receipt, for 58 mm roll printers. The
	// receipt is measured in its print layout (body.printing in app.css),
	// so the printer feeds the receipt and no blank paper after it.
	$effect(() => {
		const page = document.createElement('style')
		document.head.appendChild(page)
		function before() {
			document.body.classList.add('printing')
			const receipt = document.querySelector('.receipt-paper')
			const mm = receipt ? Math.ceil((receipt.getBoundingClientRect().height * 25.4) / 96) + 1 : 100
			page.textContent = `@page { size: 58mm ${mm}mm; margin: 0; }`
		}
		function after() {
			document.body.classList.remove('printing')
		}
		window.addEventListener('beforeprint', before)
		window.addEventListener('afterprint', after)
		return () => {
			window.removeEventListener('beforeprint', before)
			window.removeEventListener('afterprint', after)
			after()
			page.remove()
		}
	})
</script>

<main
	class="receipt-screen mx-auto flex h-[100dvh] max-w-md flex-col gap-3.5 px-4 pt-3 pb-[max(1rem,env(safe-area-inset-bottom))] text-ink"
>
	<ScreenHeader title={t('receipt.title')} onback={back} />

	<div class="receipt-scroll min-h-0 flex-1 overflow-y-auto">
		{#if notFound}
			<p class="py-20 text-center text-ink-soft">{t('receipt.notFound')}</p>
		{:else if sale}
			<ReceiptView {sale} storeName={session.terminal?.store.name ?? ''} />
		{:else if loadError}
			<div class="flex flex-col items-center gap-3 py-20 text-center">
				<p class="text-ink-soft">{t('receipt.loadFailed')}</p>
				<Button variant="secondary" size="md" onclick={() => (attempt += 1)}>{t('common.tryAgain')}</Button>
			</div>
		{/if}
	</div>

	{#if sale?.state === 'paid'}
		<div class="no-print">
			<Button variant="secondary" size="md" onclick={() => window.print()}><Printer size={20} />{t('receipt.print')}</Button>
		</div>
	{:else if sale}
		<div class="no-print">
			<Button onclick={() => router.go('/')}>{t('receipt.newSale')}</Button>
		</div>
	{/if}
</main>
