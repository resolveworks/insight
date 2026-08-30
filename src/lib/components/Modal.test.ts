import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';

const text = (t: string) => createRawSnippet(() => ({ render: () => t }));
import Modal from './Modal.svelte';
import ConfirmDialog from './ConfirmDialog.svelte';

describe('Modal', () => {
	it('renders title and content', () => {
		render(Modal, {
			props: { title: 'Test dialog', children: text('Body content') },
		});

		expect(screen.getByText('Test dialog')).toBeInTheDocument();
		expect(screen.getByText('Body content')).toBeInTheDocument();
		expect(screen.getByRole('dialog')).toHaveAttribute('aria-modal', 'true');
	});

	it('closes on Escape without calling handlers', async () => {
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

	it('closes via cancel button, backdrop, and Escape without confirming', () => {
		const onConfirm = vi.fn();
		const onCancel = vi.fn();

		// Cancel button
		const { unmount } = render(ConfirmDialog, {
			props: { title: 'T', message: 'M', onConfirm, onCancel },
		});
		screen.getByRole('button', { name: 'Cancel' }).click();
		expect(onCancel).toHaveBeenCalledTimes(1);
		unmount();

		// Backdrop click
		const { container, unmount: unmount2 } = render(ConfirmDialog, {
			props: { title: 'T', message: 'M', onConfirm, onCancel },
		});
		container.firstElementChild!.dispatchEvent(
			new MouseEvent('click', { bubbles: true }),
		);
		expect(onCancel).toHaveBeenCalledTimes(2);
		unmount2();

		// Escape
		render(ConfirmDialog, {
			props: { title: 'T', message: 'M', onConfirm, onCancel },
		});
		window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
		expect(onCancel).toHaveBeenCalledTimes(3);
		expect(onConfirm).not.toHaveBeenCalled();
	});
});
