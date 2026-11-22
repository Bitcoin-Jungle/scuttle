import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import tailwindcss from '@tailwindcss/vite'
import { VitePWA } from 'vite-plugin-pwa'
import { resolve } from 'node:path'

// posd embeds dist/ and serves it at the site root. The SPA handles both
// "/" and "/pair/<code>", so assets use root-absolute URLs.
//
// posd serves the manifests itself: "/manifest.webmanifest" for a paired
// terminal and "/pair/<code>/manifest.webmanifest" for an install link, so
// the plugin must not emit one.
//
// posd writes the brand into each page it serves, so the service worker
// does not precache the page: it fetches it, and serves the copy it last
// fetched only offline. Brand files come from posd the same way. The
// bundled assets are cached as they are fetched, under their hashed names,
// and kept after an upgrade, so a page cached before it still finds the
// scripts it names.
export default defineConfig({
	base: '/',
	plugins: [
		svelte(),
		tailwindcss(),
		VitePWA({
			manifest: false,
			injectRegister: null,
			registerType: 'autoUpdate',
			workbox: {
				globPatterns: [],
				navigateFallback: null,
				runtimeCaching: [
					{
						urlPattern: ({ request, url }) =>
							request.mode === 'navigate' &&
							!url.pathname.startsWith('/api/') &&
							!url.pathname.startsWith('/.well-known/'),
						handler: 'NetworkFirst',
						options: {
							cacheName: 'pages',
							networkTimeoutSeconds: 4,
							expiration: { maxEntries: 8 },
						},
					},
					{
						urlPattern: ({ url }) => url.pathname.startsWith('/assets/'),
						handler: 'CacheFirst',
						options: { cacheName: 'assets', expiration: { maxEntries: 60 } },
					},
					{
						urlPattern: ({ url }) => url.pathname.startsWith('/brands/'),
						handler: 'StaleWhileRevalidate',
						options: { cacheName: 'brands' },
					},
				],
			},
		}),
	],
	resolve: {
		alias: { $lib: resolve(__dirname, 'src/lib') },
	},
	build: {
		outDir: 'dist',
		assetsDir: 'assets',
	},
	server: {
		proxy: {
			'/api': 'http://127.0.0.1:3080',
			'/pair': {
				target: 'http://127.0.0.1:3080',
				bypass: (req) => (req.url?.endsWith('.webmanifest') ? undefined : '/index.html'),
			},
			'/manifest.webmanifest': 'http://127.0.0.1:3080',
			'/brands': 'http://127.0.0.1:3080',
		},
	},
})
