import { vi, afterEach } from 'vitest';
import { clearMocks } from '@tauri-apps/api/mocks';
import '@testing-library/jest-dom/vitest';

// jsdom ships the element but not the modal API; the webviews we target do.
HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) {
	this.open = true;
};
HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) {
	this.open = false;
	this.dispatchEvent(new Event('close'));
};

// Clear Tauri mocks after each test to prevent state leakage
afterEach(() => {
	clearMocks();
	vi.clearAllMocks();
});
