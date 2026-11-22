<script lang="ts">
	// A bottom sheet over a scrim. Tapping the scrim or Escape closes it.
	import type { Snippet } from 'svelte'
	import { t } from '$lib/i18n/index.svelte'

	let { label, onclose, children }: { label: string; onclose: () => void; children: Snippet } = $props()

	function keydown(e: KeyboardEvent) {
		if (e.key === 'Escape') onclose()
	}
</script>

<svelte:window onkeydown={keydown} />

<div class="no-print fixed inset-0 z-10 flex flex-col justify-end bg-scrim">
	<button type="button" class="flex-1 cursor-default" aria-label={t('common.close')} tabindex="-1" onclick={onclose}></button>
	<div
		class="mx-auto flex w-full max-w-md flex-col gap-3.5 rounded-t-[20px] bg-page px-4 pt-5 pb-[max(1rem,env(safe-area-inset-bottom))]"
		role="dialog"
		aria-modal="true"
		aria-label={label}
	>
		{@render children()}
	</div>
</div>
