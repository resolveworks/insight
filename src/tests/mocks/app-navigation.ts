// Mock for $app/navigation
import { vi } from 'vitest';
import { page } from './app-state';

export const goto = vi.fn();
export const invalidate = vi.fn(async () => {});
export const replaceState = vi.fn(async (url: string | URL) => {
	page.url = new URL(url);
});
