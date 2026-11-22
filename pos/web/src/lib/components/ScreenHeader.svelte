<script lang="ts">
	// A back button and the screen's title, with room for an action.
	import type { Snippet } from 'svelte'
	import { ArrowLeft } from 'lucide-svelte'
	import { t } from '$lib/i18n/index.svelte'
	import IconButton from './IconButton.svelte'

	let {
		title,
		back,
		onback,
		action,
	}: { title: string; back?: string; onback?: () => void; action?: Snippet } = $props()
</script>

<header class="no-print flex min-h-12 shrink-0 items-center gap-2.5">
	<IconButton label={t('common.back')} href={back} onclick={onback}><ArrowLeft size={22} /></IconButton>
	<h1 class="min-w-0 flex-1 truncate font-display text-heading brand-case tracking-display">{title}</h1>
	{@render action?.()}
</header>
