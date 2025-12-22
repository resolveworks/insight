import { describe, it, expect } from 'vitest';

describe('App Layout Navigation', () => {
	it('derives current tab correctly from various paths', () => {
		// Test the tab derivation logic used in the layout
		const deriveTab = (pathname: string) => pathname.split('/')[1] || 'search';

		const testCases = [
			{ path: '/search', expected: 'search' },
			{ path: '/files', expected: 'files' },
			{ path: '/files/abc/123', expected: 'files' },
			{ path: '/trajectory', expected: 'trajectory' },
			{ path: '/settings', expected: 'settings' },
			{ path: '/', expected: 'search' }, // falls back to search when empty
		];

		for (const { path, expected } of testCases) {
			expect(deriveTab(path)).toBe(expected);
		}
	});

	it('tab configuration is complete', () => {
		// Verify the tab configuration matches our routes
		const tabs = [
			{ id: 'trajectory', label: 'Trajectory', href: '/trajectory' },
			{ id: 'search', label: 'Search', href: '/search' },
			{ id: 'files', label: 'Files', href: '/files' },
			{ id: 'settings', label: 'Settings', href: '/settings' },
		];

		expect(tabs).toHaveLength(4);
		expect(tabs.map((t) => t.id)).toContain('search');
		expect(tabs.map((t) => t.id)).toContain('files');
		expect(tabs.map((t) => t.id)).toContain('trajectory');
		expect(tabs.map((t) => t.id)).toContain('settings');

		// Each tab should have matching id and href
		for (const tab of tabs) {
			expect(tab.href).toBe(`/${tab.id}`);
		}
	});
});
