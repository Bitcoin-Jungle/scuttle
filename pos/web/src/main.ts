import { mount } from 'svelte'
import { registerSW } from 'virtual:pwa-register'
import '$lib/app.css'
import { syncDocumentLanguage } from '$lib/i18n/index.svelte'
// Listens for the browser's install prompt, which comes once and early.
import '$lib/install.svelte'
import App from './App.svelte'

const target = document.getElementById('app')
if (!target) throw new Error('missing #app root')

syncDocumentLanguage()
mount(App, { target })

// The service worker caches the app shell only; every API call goes to posd.
if (import.meta.env.PROD) registerSW({ immediate: true })
