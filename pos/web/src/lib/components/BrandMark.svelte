<script lang="ts">
	// The brand's mark. A brand's CSS can shape it through .brand-mark.
	import { brand } from '$lib/brand.svelte'

	let { size = 32, label = '' }: { size?: number; label?: string } = $props()
</script>

<img
	src={brand.info.mark}
	alt={label}
	width={size}
	height={size}
	class="brand-mark block shrink-0 object-contain"
	style={`width:${size}px;height:${size}px`}
	aria-hidden={label ? undefined : 'true'}
/>
