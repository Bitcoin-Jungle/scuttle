<script lang="ts">
	// A sale's state as a small colored chip.
	import type { SaleState } from '$lib/api'
	import { stateLabel, stateShortLabel, stateTone } from '$lib/sale'

	let { state, short = false }: { state: SaleState; short?: boolean } = $props()

	const tone = $derived(stateTone(state))
</script>

<span
	class={`receipt-chip inline-flex h-[26px] shrink-0 items-center rounded-full px-2.5 text-[0.8125rem] font-semibold ${
		tone === 'ok' ? 'bg-ok-bg text-ok' : tone === 'warn' ? 'bg-warn-bg text-warn' : 'bg-muted text-ink-soft'
	}`}
>
	{short ? stateShortLabel(state) : stateLabel(state)}
</span>
