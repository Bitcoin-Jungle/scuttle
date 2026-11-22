<script lang="ts">
	// A device without a token: how to pair it from the owner's wallet.
	import { t } from '$lib/i18n/index.svelte'
	import { brand } from '$lib/brand.svelte'
	import BrandMark from '$lib/components/BrandMark.svelte'
	import { platform, standalone } from '$lib/install.svelte'

	// The body names the wallet and puts the button to tap in bold.
	const body = $derived(t('unpaired.body', { wallet: brand.wallet }).split('{action}'))
</script>

<main class="mx-auto flex min-h-[100dvh] max-w-md flex-col items-center justify-center gap-3.5 p-6 text-center text-ink">
	<BrandMark size={64} label={brand.info.name} />
	<h1 class="mt-1.5 font-display text-title brand-case tracking-display">{t('unpaired.title')}</h1>
	<p class="max-w-[300px] leading-normal text-ink-soft">
		{body[0]}<b class="font-semibold text-ink">{t('unpaired.action')}</b>{body[1] ?? ''}
	</p>
	<!-- Safari clears a tab's storage after a week without use. -->
	{#if platform === 'ios' && !standalone}
		<p class="max-w-[300px] text-sm leading-snug text-ink-soft">{t('unpaired.cleared')}</p>
	{/if}
</main>
