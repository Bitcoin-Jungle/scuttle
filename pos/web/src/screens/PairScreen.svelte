<script lang="ts">
	// Pairing. The owner's wallet makes a single-use link; this screen
	// exchanges its code for a device token, after the staff install the app
	// and name the device. On iOS the installed app does not share Safari's
	// storage, so pairing waits for the app unless the staff choose to stay
	// in the browser.
	import { t } from '$lib/i18n/index.svelte'
	import { ApiError, PosClient } from '$lib/api'
	import { session } from '$lib/session.svelte'
	import { brand } from '$lib/brand.svelte'
	import BrandMark from '$lib/components/BrandMark.svelte'
	import Button from '$lib/components/Button.svelte'
	import InstallGuide from '$lib/components/InstallGuide.svelte'
	import { install, platform, standalone } from '$lib/install.svelte'

	let { code }: { code: string } = $props()

	let deviceName = $state('')
	let busy = $state(false)
	let errorMsg = $state<string | null>(null)

	let useBrowser = $state(false)
	// iOS: install first, unless the staff choose the browser.
	const installFirst = $derived(!standalone && platform === 'ios' && !useBrowser)
	// Android and desktop: installing is offered beside the form. Once it is
	// installed, the app opens at this link and pairs there.
	const offerInstall = $derived(!standalone && platform !== 'ios' && (install.canPrompt || install.installed || platform === 'android'))
	const showForm = $derived(!installFirst && !install.installed)

	async function activate() {
		const name = deviceName.trim()
		if (!name || busy) return
		busy = true
		errorMsg = null
		try {
			const res = await new PosClient().pair(code, name)
			session.signIn(res.token)
			// Leave the one-time link so a reload does not try the code again.
			history.replaceState(null, '', '/#/')
		} catch (e) {
			if (e instanceof ApiError && e.status === 404) {
				errorMsg = t('pair.linkUsed')
			} else if (e instanceof ApiError && e.status === 0) {
				errorMsg = t('pair.unreachable')
			} else {
				errorMsg = t('common.genericError')
			}
		} finally {
			busy = false
		}
	}
</script>

<main class="mx-auto flex min-h-[100dvh] max-w-md flex-col px-4 pt-6 pb-[max(1rem,env(safe-area-inset-bottom))] text-ink">
	<form
		class="flex flex-1 flex-col gap-6"
		onsubmit={(e) => {
			e.preventDefault()
			void activate()
		}}
	>
		<div class="flex flex-col gap-3.5">
			<BrandMark size={48} label={brand.info.name} />
			<h1 class="font-display text-title brand-case tracking-display">{t('pair.title')}</h1>
		</div>

		{#if installFirst || offerInstall}
			<InstallGuide />
		{/if}
		{#if useBrowser}
			<p class="rounded-md bg-muted p-3.5 text-[0.9375rem] leading-snug" role="note">{t('install.browserWarning')}</p>
		{/if}

		{#if showForm}
			<div class="flex flex-col gap-2.5">
				<label for="device-name" class="text-[1.0625rem] font-semibold">{t('pair.deviceName')}</label>
				<input
					id="device-name"
					class="h-[52px] w-full rounded-md border-[1.5px] border-line bg-surface px-3.5 text-lg text-ink outline-none focus:border-accent"
					bind:value={deviceName}
					placeholder={t('pair.deviceNamePlaceholder')}
					maxlength="64"
					autocomplete="off"
				/>
				<p class="text-sm text-ink-soft">{t('pair.deviceNameHint')}</p>
			</div>
		{/if}

		<div class="flex-1"></div>
		{#if errorMsg}
			<p role="alert" class="text-center text-[0.9375rem] leading-snug text-err">{errorMsg}</p>
		{/if}
		{#if showForm}
			<div class="flex flex-col gap-2.5">
				<Button type="submit" disabled={!deviceName.trim() || busy}>{busy ? t('pair.activating') : t('pair.activate')}</Button>
				<p class="text-center text-[0.8125rem] text-ink-soft">{t('pair.linkOnce')}</p>
			</div>
		{:else if installFirst}
			<Button variant="ghost" size="md" onclick={() => (useBrowser = true)}>{t('install.useBrowser')}</Button>
		{/if}
	</form>
</main>
