<script lang="ts">
	// The sale is paid: the whole screen turns green so staff see it from
	// across the counter, with a burst of confetti, a buzz and a chime.
	import confetti from 'canvas-confetti'
	import { Check } from 'lucide-svelte'
	import { t } from '$lib/i18n/index.svelte'

	let {
		amountLabel,
		meta,
		note,
		onnewsale,
		onreceipt,
	}: {
		amountLabel: string
		/** How it was paid, for example "Lightning · 20,000 sats". */
		meta: string
		/** Shown when the payer sent more than the sale. */
		note?: string
		onnewsale: () => void
		onreceipt: () => void
	} = $props()

	// The theme's confetti colors, a comma-separated list in --pos-confetti.
	function themeConfetti(): string[] {
		return getComputedStyle(document.documentElement)
			.getPropertyValue('--pos-confetti')
			.split(',')
			.map((c) => c.trim())
			.filter(Boolean)
	}

	$effect(() => {
		const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
		if (!reduceMotion) {
			// The library's default instance is created with useWorker: true; its
			// blob: worker violates the page CSP (script-src 'self' is the
			// worker-src fallback), silently killing the burst. Create a
			// no-worker cannon instead — one-shot main-thread rendering is fine.
			const fire = confetti.create(null as unknown as HTMLCanvasElement, {
				resize: true,
				useWorker: false,
			})
			fire({
				particleCount: 120,
				spread: 70,
				origin: { y: 0.6 },
				colors: themeConfetti(),
			})
		}

		if ('vibrate' in navigator) {
			navigator.vibrate([200])
		}

		try {
			const ctx = new AudioContext()
			const osc = ctx.createOscillator()
			const gain = ctx.createGain()
			osc.type = 'sine'
			osc.frequency.value = 880
			gain.gain.setValueAtTime(0.15, ctx.currentTime)
			gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.4)
			osc.connect(gain)
			gain.connect(ctx.destination)
			osc.start()
			osc.stop(ctx.currentTime + 0.4)
			osc.onended = () => void ctx.close()
		} catch {
			/* audio unavailable: skip chime */
		}
	})
</script>

<main class="min-h-[100dvh] bg-paid text-white">
	<div class="mx-auto flex min-h-[100dvh] max-w-md flex-col px-4 pt-4 pb-[max(1rem,env(safe-area-inset-bottom))]">
		<div class="flex flex-1 flex-col items-center justify-center gap-2.5 text-center">
			<div class="mb-2.5 grid h-[104px] w-[104px] place-items-center rounded-full bg-white text-paid">
				<Check size={56} strokeWidth={3} />
			</div>
			<h1 class="font-display text-hero leading-none brand-case tracking-display">{t('pay.paid')}</h1>
			<p class="font-display text-paid-amount tabular-nums tracking-display">{amountLabel}</p>
			<p class="tabular-nums">{meta}</p>
			{#if note}
				<p class="mt-3.5 max-w-[300px] rounded-md bg-black/20 px-4 py-3 text-[0.9375rem] leading-snug">{note}</p>
			{/if}
		</div>
		<div class="flex flex-col gap-2.5">
			<button
				type="button"
				class="min-h-[58px] w-full rounded-md bg-white font-display text-button text-paid-ink brand-case tracking-button active:opacity-85"
				onclick={onnewsale}
			>
				{t('pay.newSale')}
			</button>
			<button
				type="button"
				class="min-h-[52px] w-full rounded-md bg-black/20 font-display text-button-sm text-white brand-case tracking-button active:opacity-80"
				onclick={onreceipt}
			>
				{t('pay.receipt')}
			</button>
		</div>
	</div>
</main>
