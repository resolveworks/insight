// Mock for $app/navigation
import { vi } from 'vitest';

export const goto = vi.fn();
export const invalidate = vi.fn(async () => {});
