import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import ConversationSidebar from './ConversationSidebar.svelte';
import * as chat from '#lib/stores/conversations.svelte.js';

const summary = (id: string, title: string) => ({ id, title });

const view = (id: string) => ({
	id,
	collection_id: 'col1',
	title: `Chat ${id}`,
	messages: [],
});

describe('ConversationSidebar', () => {
	beforeEach(() => {
		clearMocks();
		localStorage.clear();
	});

	it('shows the conversation list without a chat provider configured', async () => {
		const listCalls: Array<Record<string, unknown>> = [];
		mockIPC((cmd, args) => {
			if (cmd === 'list_conversations') {
				listCalls.push(args as Record<string, unknown>);
				return [
					summary('c1', 'Climate research'),
					summary('c2', 'Budget analysis'),
				];
			}
			if (cmd === 'load_conversation') return view('c1');
		});

		await chat.initialize('col1');
		render(ConversationSidebar, { props: {} });

		await waitFor(() => {
			expect(screen.getByText('Climate research')).toBeInTheDocument();
		});
		expect(screen.getByText('Budget analysis')).toBeInTheDocument();
		expect(screen.queryByText('Loading...')).not.toBeInTheDocument();
		expect(listCalls).toContainEqual({ collectionId: 'col1' });
	});

	it('shows empty state when the list is empty', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_conversations') return [];
		});

		await chat.initialize('col1');
		render(ConversationSidebar, { props: {} });

		await waitFor(() => {
			expect(screen.getByText('No conversations yet')).toBeInTheDocument();
		});
	});

	it('deletes a conversation after confirmation', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'list_conversations') {
				return [summary('c1', 'Climate research')];
			}
			if (cmd === 'load_conversation') return view('c1');
			if (cmd === 'delete_conversation') return null;
		});

		await chat.initialize('col1');
		render(ConversationSidebar, { props: {} });

		await waitFor(() => {
			expect(screen.getByText('Climate research')).toBeInTheDocument();
		});

		// Opening the confirm dialog alone does not delete anything.
		screen.getByLabelText('Delete chat').click();
		await waitFor(() => {
			expect(screen.getByText('Delete chat')).toBeInTheDocument();
		});
		expect(
			screen.getByText(/This conversation's history will be removed\./),
		).toBeInTheDocument();
		expect(calls.find((c) => c.cmd === 'delete_conversation')).toBeUndefined();

		screen.getByRole('button', { name: 'Delete' }).click();

		await waitFor(() => {
			const deleteCall = calls.find((c) => c.cmd === 'delete_conversation');
			expect(deleteCall?.args).toEqual({
				collectionId: 'col1',
				conversationId: 'c1',
			});
		});
	});

	it('cancels deletion without invoking the backend', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'list_conversations') {
				return [summary('c1', 'Climate research')];
			}
			if (cmd === 'load_conversation') return view('c1');
			if (cmd === 'delete_conversation') return null;
		});

		await chat.initialize('col1');
		render(ConversationSidebar, { props: {} });

		await waitFor(() => {
			expect(screen.getByText('Climate research')).toBeInTheDocument();
		});

		screen.getByLabelText('Delete chat').click();
		await waitFor(() => {
			expect(
				screen.getByRole('button', { name: 'Cancel' }),
			).toBeInTheDocument();
		});
		screen.getByRole('button', { name: 'Cancel' }).click();

		await waitFor(() => {
			expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
		});
		expect(calls.find((c) => c.cmd === 'delete_conversation')).toBeUndefined();
	});
});
