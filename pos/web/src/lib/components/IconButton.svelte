<script lang="ts">
	// A 48px square button for the header: back, Sales, Settings.
	import type { Snippet } from 'svelte'

	let { label, href, onclick, children }: { label: string; href?: string; onclick?: () => void; children: Snippet } =
		$props()

	const classes = 'grid h-12 w-12 shrink-0 place-items-center rounded-md bg-muted text-ink active:opacity-70'
</script>

{#if href}
	<a class={classes} {href} aria-label={label}>{@render children()}</a>
{:else}
	<button type="button" class={classes} aria-label={label} {onclick}>{@render children()}</button>
{/if}
