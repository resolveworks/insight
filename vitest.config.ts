import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import path from 'path';

export default defineConfig({
	plugins: [svelte({ hot: false })],
	resolve: {
		conditions: ['browser'],
		alias: {
			$lib: path.resolve('./src/lib'),
			'$app/paths': path.resolve('./src/tests/mocks/app-paths.ts'),
			'$app/environment': path.resolve('./src/tests/mocks/app-environment.ts'),
		},
	},
	test: {
		environment: 'jsdom',
		include: ['src/**/*.{test,spec}.{js,ts}'],
		setupFiles: ['src/tests/setup.ts'],
		globals: true,
	},
});
