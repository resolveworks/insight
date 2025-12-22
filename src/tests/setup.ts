import { vi, afterEach } from 'vitest';
import { clearMocks } from '@tauri-apps/api/mocks';
import '@testing-library/jest-dom/vitest';

// Clear Tauri mocks after each test to prevent state leakage
afterEach(() => {
	clearMocks();
	vi.clearAllMocks();
});
