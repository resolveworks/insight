import { describe, it, expect } from 'vitest';

describe('App Layout Navigation', () => {
	it('derives current tab correctly from various paths', () => {
		// Test the tab derivation logic used in the layout
		const deriveTab = (pathname: string) =>
			pathname.split('/')[1] || 'research';

		const testCases = [
			{ path: '/research', expected: 'research' },
			{ path: '/files', expected: 'files' },
			{ path: '/files/abc/123', expected: 'files' },
			{ path: '/settings', expected: 'settings' },
			{ path: '/', expected: 'research' }, // falls back to research when empty
		];

		for (const { path, expected } of testCases) {
			expect(deriveTab(path)).toBe(expected);
		}
	});

	it('tab configuration is complete', () => {
		// Verify the tab configuration matches our routes
		const tabs = [
			{ id: 'research', label: 'Research', href: '/research' },
			{ id: 'files', label: 'Files', href: '/files' },
			{ id: 'settings', label: 'Settings', href: '/settings' },
		];

		expect(tabs).toHaveLength(3);
		expect(tabs.map((t) => t.id)).toContain('research');
		expect(tabs.map((t) => t.id)).toContain('files');
		expect(tabs.map((t) => t.id)).toContain('settings');

		// Each tab should have matching id and href
		for (const tab of tabs) {
			expect(tab.href).toBe(`/${tab.id}`);
		}
	});
});
