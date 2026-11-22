<script lang="ts">
	// A sale that is not waiting for its payment: an icon, what happened,
	// and what to do next.
	import type { Snippet } from 'svelte'
	import Button from './Button.svelte'

	let {
		tone = 'muted',
		title,
		amount,
		body,
		error,
		primary,
		onprimary,
		secondary,
		onsecondary,
		busy = false,
		icon,
	}: {
		tone?: 'muted' | 'warn'
		title: string
		amount?: string
		body: string
		error?: string | null
		primary: string
		onprimary: () => void
		secondary?: string
		onsecondary?: () => void
		busy?: boolean
		icon: Snippet
	} = $props()
</script>

<main class="mx-auto flex min-h-[100dvh] max-w-md flex-col px-4 pt-4 pb-[max(1rem,env(safe-area-inset-bottom))] text-ink">
	<div class="flex flex-1 flex-col items-center justify-center gap-2.5 text-center">
		<div
			class={`mb-2 grid h-20 w-20 place-items-center rounded-full ${tone === 'warn' ? 'bg-warn-bg text-warn' : 'bg-muted text-ink-soft'}`}
		>
			{@render icon()}
		</div>
		<h1 class="font-display text-title brand-case tracking-display">{title}</h1>
		{#if amount}
			<p class="text-lg font-semibold tabular-nums">{amount}</p>
		{/if}
		<p class="max-w-[300px] leading-normal text-ink-soft">{body}</p>
	</div>
	{#if error}
		<p role="alert" class="mb-3 text-center text-[0.9375rem] text-err">{error}</p>
	{/if}
	<div class="flex flex-col gap-2.5">
		<Button disabled={busy} onclick={onprimary}>{primary}</Button>
		{#if secondary && onsecondary}
			<Button variant="secondary" size="md" onclick={onsecondary}>{secondary}</Button>
		{/if}
	</div>
</main>
