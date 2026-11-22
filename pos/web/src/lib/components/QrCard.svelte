<script lang="ts">
	// A payment QR code on a white card, white in dark mode too so cameras
	// get full contrast. It is drawn as SVG at the card's full width with
	// square modules, so low-resolution screens do not blur it, at the
	// lowest error correction (the screen is not damaged print) and with the
	// standard four-module quiet zone.
	import type { Snippet } from 'svelte'
	import QRCode from 'qrcode'

	let { value, label, children }: { value: string | null; label: string; children?: Snippet } = $props()

	const path = $derived.by(() => {
		if (!value) return null
		const qr = QRCode.create(value, { errorCorrectionLevel: 'L' })
		const n = qr.modules.size
		let d = ''
		for (let y = 0; y < n; y++) {
			for (let x = 0; x < n; x++) if (qr.modules.get(x, y)) d += `M${x + 4} ${y + 4}h1v1h-1z`
		}
		return { d, size: n + 8 }
	})
</script>

<div
	class={`mx-auto grid aspect-square w-full max-w-[290px] shrink-0 place-items-center rounded-lg ${path ? 'bg-white' : 'bg-surface'}`}
>
	{#if path}
		<svg
			viewBox={`0 0 ${path.size} ${path.size}`}
			class="block h-full w-full"
			shape-rendering="crispEdges"
			role="img"
			aria-label={label}
		>
			<path d={path.d} fill="#000" />
		</svg>
	{:else}
		{@render children?.()}
	{/if}
</div>
