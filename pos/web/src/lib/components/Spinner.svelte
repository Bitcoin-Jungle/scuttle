<script lang="ts">
	// The brand's loading animation, or a ring turning around its mark.
	import { brand } from '$lib/brand.svelte'
	import { t } from '$lib/i18n/index.svelte'

	let { size = 48 }: { size?: number } = $props()
</script>

{#if brand.info.spinner}
	<img
		src={brand.info.spinner}
		alt={t('common.loading')}
		class="pointer-events-none block select-none object-contain"
		style={`width:${size}px;height:${size}px`}
	/>
{:else}
	<div class="relative" style={`width:${size}px;height:${size}px`} role="img" aria-label={t('common.loading')}>
		<svg viewBox="0 0 48 48" class="turning absolute inset-0 h-full w-full" aria-hidden="true">
			<circle cx="24" cy="24" r="22" fill="none" stroke="var(--pos-line)" stroke-width="2.5" />
			<circle
				cx="24"
				cy="24"
				r="22"
				fill="none"
				stroke="var(--pos-accent)"
				stroke-width="2.5"
				stroke-linecap="round"
				stroke-dasharray="36 103"
			/>
		</svg>
		<img
			src={brand.info.mark}
			alt=""
			class="brand-mark absolute top-1/2 left-1/2 w-[64%] -translate-x-1/2 -translate-y-1/2"
		/>
	</div>
{/if}

<style>
	.turning {
		animation: turn 1.2s linear infinite;
	}
	@keyframes turn {
		to {
			transform: rotate(360deg);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.turning {
			animation-duration: 3s;
		}
	}
</style>
