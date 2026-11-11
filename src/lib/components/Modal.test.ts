import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { createRawSnippet, tick } from 'svelte';

const text = (t: string) => createRawSnippet(() => ({ render: () => t }));
const controls = () =>
	createRawSnippet(() => ({
		render: () => '<div><button>First</button><button>Second</button></div>',
	}));

import Modal from './Modal.svelte';
import ConfirmDialog from './ConfirmDialog.svelte';

function dialogRect(): DOMRect {
	return {
		left: 10,
		top: 10,
		right: 110,
		bottom: 110,
		width: 100,
		height: 100,
		x: 10,
		y: 10,
		toJSON: () => ({}),
	};
}

describe('Modal', () => {
	it('opens as a modal dialog', async () => {
		render(Modal, {
			props: { onClose: vi.fn(), children: text('x') },
		});
		await tick();

		expect(screen.getByRole('dialog')).toHaveAttribute('open');
	});

	it('labels the dialog with its title', async () => {
		render(Modal, {
			props: { title: 'Dialog', onClose: vi.fn(), children: text('x') },
		});
		await tick();

		expect(screen.getByRole('dialog', { name: 'Dialog' })).toBeInTheDocument();
	});

	it('closes when the dialog requests to be cancelled', async () => {
		const onClose = vi.fn();
		render(Modal, {
			props: { onClose, children: text('x') },
		});
		await tick();

		const event = new Event('cancel', { cancelable: true });
		screen.getByRole('dialog').dispatchEvent(event);

		expect(event.defaultPrevented).toBe(true);
		expect(onClose).toHaveBeenCalledTimes(1);
	});

	it('closes on a click outside the dialog', async () => {
		const onClose = vi.fn();
		render(Modal, {
			props: { onClose, children: text('x') },
		});
		await tick();

		const dialog = screen.getByRole('dialog');
		dialog.getBoundingClientRect = () => dialogRect();
		await fireEvent.click(dialog, { clientX: 5, clientY: 5 });

		expect(onClose).toHaveBeenCalledTimes(1);
	});

	it('stays open on a click inside the dialog', async () => {
		const onClose = vi.fn();
		render(Modal, {
			props: { onClose, children: text('x') },
		});
		await tick();

		const dialog = screen.getByRole('dialog');
		dialog.getBoundingClientRect = () => dialogRect();
		await fireEvent.click(dialog, { clientX: 20, clientY: 20 });

		expect(onClose).not.toHaveBeenCalled();
	});

	it('closes from the close button', async () => {
		const onClose = vi.fn();
		render(Modal, {
			props: { title: 'Dialog', onClose, children: text('x') },
		});
		await tick();

		await fireEvent.click(screen.getByRole('button', { name: 'Close dialog' }));

		expect(onClose).toHaveBeenCalledTimes(1);
	});

	it('renders child controls and footer content', async () => {
		const footer = createRawSnippet(() => ({
			render: () => '<button>Save</button>',
		}));
		render(Modal, {
			props: { onClose: vi.fn(), children: controls(), footer },
		});
		await tick();

		expect(screen.getByRole('button', { name: 'First' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Second' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
	});
});

describe('ConfirmDialog', () => {
	it('renders title and message', async () => {
		render(ConfirmDialog, {
			props: {
				title: 'Delete collection',
				message: 'Delete "Research"? This cannot be undone.',
				onConfirm: vi.fn(),
				onCancel: vi.fn(),
			},
		});
		await tick();

		expect(
			screen.getByRole('heading', { name: 'Delete collection' }),
		).toBeInTheDocument();
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
		await tick();

		await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

		expect(onConfirm).toHaveBeenCalledTimes(1);
		expect(onCancel).not.toHaveBeenCalled();
	});

	it('cancels without confirming', async () => {
		const onConfirm = vi.fn();
		const onCancel = vi.fn();
		render(ConfirmDialog, {
			props: { title: 'T', message: 'M', onConfirm, onCancel },
		});
		await tick();

		await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

		expect(onCancel).toHaveBeenCalledTimes(1);
		expect(onConfirm).not.toHaveBeenCalled();
	});

	it('marks Cancel as the autofocus target', async () => {
		render(ConfirmDialog, {
			props: {
				title: 'T',
				message: 'M',
				onConfirm: vi.fn(),
				onCancel: vi.fn(),
			},
		});
		await tick();

		expect(screen.getByRole('button', { name: 'Cancel' })).toHaveAttribute(
			'autofocus',
		);
	});
});
