<script lang="ts">
	// The amount keys. `precision` gates the bottom-left key: '.' for a
	// currency with cents, '00' without.
	import { Delete } from 'lucide-svelte'
	import { t } from '$lib/i18n/index.svelte'

	let { precision = 2, onInput }: { precision?: number; onInput: (value: string) => void } = $props()

	const keys = $derived(['1', '2', '3', '4', '5', '6', '7', '8', '9', precision > 0 ? '.' : '00', '0', 'back'])

	function ariaLabel(key: string): string {
		if (key === 'back') return t('keypad.deleteKey')
		if (key === '.') return t('keypad.centsKey')
		if (key === '00') return t('keypad.zerosKey')
		return t('keypad.addKey', { key })
	}
</script>

<div class="grid shrink-0 touch-manipulation select-none grid-cols-3 gap-2">
	{#each keys as key, i (i)}
		<button
			type="button"
			class="grid h-[clamp(3rem,8dvh,3.875rem)] touch-manipulation select-none place-items-center rounded-md bg-surface text-[1.6875rem] font-medium tabular-nums text-ink active:bg-muted"
			aria-label={ariaLabel(key)}
			onclick={() => onInput(key)}
		>
			{#if key === 'back'}
				<Delete size={28} />
			{:else}
				{key}
			{/if}
		</button>
	{/each}
</div>
