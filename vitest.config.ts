import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import path from 'path';

export default defineConfig({
	plugins: [svelte()],
	resolve: {
		conditions: ['browser'],
		alias: {
			'$app/navigation': path.resolve('./src/tests/mocks/app-navigation.ts'),
			'$app/paths': path.resolve('./src/tests/mocks/app-paths.ts'),
			'$app/environment': path.resolve('./src/tests/mocks/app-environment.ts'),
			'$app/state': path.resolve('./src/tests/mocks/app-state.ts'),
		},
	},
	test: {
		environment: 'jsdom',
		include: ['src/**/*.{test,spec}.{js,ts}'],
		setupFiles: ['src/tests/setup.ts'],
		globals: true,
	},
});
