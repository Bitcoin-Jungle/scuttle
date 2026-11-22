<script lang="ts">
	// Sales grouped by the day they were made, newest first, as posd lists
	// them: each row has the amount, when, how and the note, and the state.
	import { ChevronRight } from 'lucide-svelte'
	import type { Sale } from '$lib/api'
	import { i18n, t } from '$lib/i18n/index.svelte'
	import { formatFiat } from '$lib/money'
	import { isoDate, railLabel, saleAmountLabel } from '$lib/sale'
	import StateChip from './StateChip.svelte'

	let { rows }: { rows: Sale[] } = $props()

	function at(sale: Sale): Date {
		return new Date(sale.created_at)
	}

	function dayLabel(day: Date): string {
		const today = new Date()
		const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1)
		if (isoDate(day) === isoDate(today)) return t('history.today')
		if (isoDate(day) === isoDate(yesterday)) return t('history.yesterday')
		return day.toLocaleDateString(i18n.tag, { dateStyle: 'medium' })
	}

	const groups = $derived.by(() => {
		const out: { key: string; label: string; rows: Sale[] }[] = []
		for (const row of rows) {
			const day = at(row)
			const key = isoDate(day)
			let group = out[out.length - 1]
			if (group?.key !== key) {
				group = { key, label: dayLabel(day), rows: [] }
				out.push(group)
			}
			group.rows.push(row)
		}
		return out
	})

	function meta(sale: Sale): string {
		const time = at(sale).toLocaleTimeString(i18n.tag, { hour: 'numeric', minute: '2-digit' })
		return [time, sale.paid_rail && railLabel(sale.paid_rail), sale.memo].filter(Boolean).join(' · ')
	}
</script>

<div class="flex flex-col gap-2">
	{#each groups as group (group.key)}
		<h2 class="px-1 pt-1 text-sm font-semibold text-ink-soft">{group.label}</h2>
		<div class="overflow-hidden rounded-lg bg-surface">
			{#each group.rows as row, i (row.id)}
				<a
					class={`flex min-h-[68px] items-center gap-2.5 py-2.5 pr-3 pl-4 active:bg-muted ${i > 0 ? 'border-t border-line' : ''}`}
					href={`#/receipt/${row.id}`}
				>
					<span class="flex min-w-0 flex-1 flex-col gap-[3px]">
						<span class="text-[1.0625rem] font-semibold tabular-nums">{saleAmountLabel(row, (a, c) => formatFiat(a, c))}</span>
						<span class="truncate text-sm text-ink-soft">{meta(row)}</span>
					</span>
					<StateChip state={row.state} short />
					<ChevronRight size={18} class="shrink-0 text-ink-soft" />
				</a>
			{/each}
		</div>
	{/each}
</div>
