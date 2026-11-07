import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { createRawSnippet, tick } from 'svelte';

const text = (t: string) => createRawSnippet(() => ({ render: () => t }));
const controls = (autofocusSecond = false) =>
	createRawSnippet(() => ({
		render: () =>
			`<div><button>First</button><button${autofocusSecond ? ' data-autofocus' : ''}>Second</button></div>`,
	}));
import Modal from './Modal.svelte';
import ConfirmDialog from './ConfirmDialog.svelte';

describe('Modal', () => {
	it('closes once on Escape inside the dialog', async () => {
		const onClose = vi.fn();
		render(Modal, {
			props: { onClose, children: controls() },
		});
		await tick();

		fireEvent.keyDown(screen.getByRole('button', { name: 'First' }), {
			key: 'Escape',
		});
		expect(onClose).toHaveBeenCalledTimes(1);
	});

	it('closes on Escape from the window', async () => {
		const onClose = vi.fn();
		render(Modal, {
			props: { title: 'Dialog', onClose, children: text('x') },
		});

		window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
		expect(onClose).toHaveBeenCalledTimes(1);
	});

	it('closes on backdrop click', async () => {
		const onClose = vi.fn();
		const { container } = render(Modal, {
			props: { onClose, children: text('x') },
		});

		container.firstElementChild!.dispatchEvent(
			new MouseEvent('click', { bubbles: true }),
		);
		expect(onClose).toHaveBeenCalledTimes(1);
	});

	it('does not close when clicking inside the panel', () => {
		const onClose = vi.fn();
		render(Modal, {
			props: { onClose, children: text('x') },
		});

		screen.getByRole('dialog').click();
		expect(onClose).not.toHaveBeenCalled();
	});

	it('labels the dialog with its title', () => {
		render(Modal, {
			props: { title: 'Dialog', onClose: vi.fn(), children: text('x') },
		});

		expect(screen.getByRole('dialog', { name: 'Dialog' })).toBeInTheDocument();
	});

	it('focuses the autofocus target on open', async () => {
		render(Modal, {
			props: { onClose: vi.fn(), children: controls(true) },
		});
		await tick();

		expect(screen.getByRole('button', { name: 'Second' })).toHaveFocus();
	});

	it('focuses the first focusable control when there is no autofocus target', async () => {
		render(Modal, {
			props: { onClose: vi.fn(), children: controls() },
		});
		await tick();

		expect(screen.getByRole('button', { name: 'First' })).toHaveFocus();
	});

	it('wraps Tab from the last control to the first', async () => {
		render(Modal, {
			props: { onClose: vi.fn(), children: controls() },
		});
		await tick();

		const first = screen.getByRole('button', { name: 'First' });
		const last = screen.getByRole('button', { name: 'Second' });
		last.focus();
		fireEvent.keyDown(last, { key: 'Tab' });

		expect(first).toHaveFocus();
	});

	it('wraps Shift+Tab from the first control to the last', async () => {
		render(Modal, {
			props: { onClose: vi.fn(), children: controls() },
		});
		await tick();

		const first = screen.getByRole('button', { name: 'First' });
		const last = screen.getByRole('button', { name: 'Second' });
		first.focus();
		fireEvent.keyDown(first, { key: 'Tab', shiftKey: true });

		expect(last).toHaveFocus();
	});

	it('makes the background inert and restores it on close', async () => {
		const outside = document.createElement('button');
		document.body.appendChild(outside);
		outside.focus();

		const { unmount } = render(Modal, {
			props: { onClose: vi.fn(), children: controls() },
		});
		await tick();
		expect(outside.inert).toBe(true);

		unmount();
		expect(outside.inert).toBeFalsy();
		outside.remove();
	});

	it('restores focus to the previously focused element on close', async () => {
		const outside = document.createElement('button');
		document.body.appendChild(outside);
		outside.focus();

		const { unmount } = render(Modal, {
			props: { onClose: vi.fn(), children: controls() },
		});
		await tick();
		expect(screen.getByRole('button', { name: 'First' })).toHaveFocus();

		unmount();
		expect(outside).toHaveFocus();
		outside.remove();
	});
});

describe('ConfirmDialog', () => {
	it('renders title and message', () => {
		render(ConfirmDialog, {
			props: {
				title: 'Delete collection',
				message: 'Delete "Research"? This cannot be undone.',
				onConfirm: vi.fn(),
				onCancel: vi.fn(),
			},
		});

		expect(screen.getByText('Delete collection')).toBeInTheDocument();
		expect(
			screen.getByText('Delete "Research"? This cannot be undone.'),
		).toBeInTheDocument();
	});

	it('fires confirm when the confirm button is clicked', async () => {
		const onConfirm = vi.fn();
		const onCancel = vi.fn();
		render(ConfirmDialog, {
			props: {
				title: 'Delete document',
				message: 'Delete "report.pdf"?',
				onConfirm,
				onCancel,
			},
		});

		screen.getByRole('button', { name: 'Delete' }).click();
		expect(onConfirm).toHaveBeenCalledTimes(1);
		expect(onCancel).not.toHaveBeenCalled();
	});

	it('autofocuses the Cancel button', async () => {
		render(ConfirmDialog, {
			props: {
				title: 'T',
				message: 'M',
				onConfirm: vi.fn(),
				onCancel: vi.fn(),
			},
		});

		await new Promise((r) => setTimeout(r, 0));
		expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
	});

	it('cancels without confirming', () => {
		const onConfirm = vi.fn();
		const onCancel = vi.fn();
		render(ConfirmDialog, {
			props: { title: 'T', message: 'M', onConfirm, onCancel },
		});

		screen.getByRole('button', { name: 'Cancel' }).click();
		expect(onCancel).toHaveBeenCalledTimes(1);
		expect(onConfirm).not.toHaveBeenCalled();
	});
});
