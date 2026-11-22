<script lang="ts">
	// The terminal's buttons: full width, 52px and up, flat. A disabled
	// button turns grey rather than faint, so it reads as "not yet".
	import type { Snippet } from 'svelte'

	let {
		variant = 'primary',
		size = 'lg',
		type = 'button',
		disabled = false,
		onclick,
		children,
	}: {
		variant?: 'primary' | 'secondary' | 'ghost' | 'danger' | 'danger-soft'
		/** lg: the screen's main action; md: the ones beside or under it. */
		size?: 'lg' | 'md'
		type?: 'button' | 'submit'
		disabled?: boolean
		onclick?: () => void
		children?: Snippet
	} = $props()

	const classes = $derived(
		[
			'inline-flex w-full items-center justify-center gap-2 rounded-md px-4 font-display brand-case tracking-button tabular-nums transition-opacity active:opacity-80 disabled:cursor-not-allowed disabled:bg-muted disabled:text-ink-soft disabled:active:opacity-100',
			size === 'lg' ? 'min-h-[58px] text-button' : 'min-h-[52px] text-button-sm',
			variant === 'primary' && 'bg-accent text-on-accent',
			variant === 'secondary' && 'bg-muted text-ink',
			variant === 'ghost' && 'bg-transparent text-ink active:bg-muted',
			variant === 'danger' && 'bg-danger text-on-accent',
			variant === 'danger-soft' && 'bg-surface text-err',
		]
			.filter(Boolean)
			.join(' '),
	)
</script>

<button class={classes} {type} {disabled} {onclick}>{@render children?.()}</button>
