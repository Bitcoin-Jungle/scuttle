<script lang="ts">
	// How to install the terminal as an app on this device: the browser's
	// own prompt where it has one, otherwise the steps for its menus, each
	// with the icon the staff will look for.
	import { Check, EllipsisVertical, Share, SquarePlus } from 'lucide-svelte'
	import { t } from '$lib/i18n/index.svelte'
	import { install, platform } from '$lib/install.svelte'
	import { isIosSafari } from '$lib/platform'
	import { brand } from '$lib/brand.svelte'
	import BrandMark from './BrandMark.svelte'
	import Button from './Button.svelte'

	const app = $derived(brand.info.app_name)
	const safari = platform === 'ios' && isIosSafari(navigator.userAgent)

	type Step = { icon: 'share' | 'add' | 'menu' | 'app'; title: string; hint: string }
	const steps = $derived.by((): Step[] => {
		const open: Step = { icon: 'app', title: t('install.open', { app }), hint: t('install.openHint') }
		if (platform === 'ios') {
			return [
				{ icon: 'share', title: t('install.share'), hint: safari ? t('install.shareHint') : t('install.shareHintOther') },
				{ icon: 'add', title: t('install.add'), hint: t('install.addHint') },
				open,
			]
		}
		return [
			{ icon: 'menu', title: t('install.menu'), hint: t('install.menuHint') },
			{ icon: 'add', title: t('install.addAndroid'), hint: '' },
			open,
		]
	})

	let prompting = $state(false)
	async function prompt() {
		prompting = true
		try {
			await install.prompt()
		} finally {
			prompting = false
		}
	}
</script>

<section class="flex flex-col gap-4 rounded-lg bg-surface p-4" aria-label={t('install.title')}>
	<!-- What the staff will find on the home screen. -->
	<div class="flex items-center gap-3.5">
		<div class="grid h-14 w-14 shrink-0 place-items-center rounded-[14px] bg-page shadow-[0_1px_3px_rgb(0_0_0/0.18)]">
			<BrandMark size={36} label={app} />
		</div>
		<div class="flex min-w-0 flex-col gap-0.5">
			<p class="text-[1.0625rem] font-semibold">{app}</p>
			<p class="text-[0.9375rem] leading-snug text-ink-soft">{t('install.why')}</p>
		</div>
	</div>

	{#if install.installed}
		<div class="flex items-start gap-3 rounded-md bg-muted p-3.5" role="status">
			<span class="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-paid text-white"><Check size={18} strokeWidth={2.6} /></span>
			<p class="pt-0.5 text-[0.9375rem] leading-snug">{t('install.done', { app })}</p>
		</div>
	{:else if install.canPrompt}
		<Button onclick={prompt} disabled={prompting}>{t('install.button')}</Button>
	{:else}
		<ol class="flex flex-col gap-3">
			{#each steps as step, i (i)}
				<li class="flex items-start gap-3">
					<span
						class="grid h-10 w-10 shrink-0 place-items-center rounded-[10px] border border-line bg-page text-accent"
						aria-hidden="true"
					>
						{#if step.icon === 'share'}
							<Share size={20} />
						{:else if step.icon === 'add'}
							<SquarePlus size={20} />
						{:else if step.icon === 'menu'}
							<EllipsisVertical size={20} />
						{:else}
							<BrandMark size={22} label="" />
						{/if}
					</span>
					<div class="flex min-w-0 flex-col gap-0.5 pt-0.5">
						<p class="font-semibold"><span class="text-ink-soft tabular-nums">{i + 1}.</span> {step.title}</p>
						{#if step.hint}
							<p class="text-sm leading-snug text-ink-soft">{step.hint}</p>
						{/if}
					</div>
				</li>
			{/each}
		</ol>
		{#if platform === 'ios' && !safari}
			<p class="text-sm leading-snug text-ink-soft">{t('install.otherBrowser')}</p>
		{/if}
	{/if}
</section>
