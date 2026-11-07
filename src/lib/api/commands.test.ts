import { describe, it, expect } from 'vitest';
import { mockIPC } from '@tauri-apps/api/mocks';
import { invalidate } from '$app/navigation';
import {
	createCollection,
	deleteCollection,
	importCollection,
	listCollections,
	shareCollection,
} from './collection';
import {
	deleteDocument,
	getPipelineProgress,
	listDocuments,
	startImport,
} from './document';
import {
	deleteConversation,
	listConversations,
	loadConversation,
	startChat,
} from './conversation';

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

const document = {
	id: 'doc1',
	name: 'report.pdf',
	file_type: 'application/pdf',
	page_count: 12,
	tags: [],
	created_at: '2024-01-15T10:30:00Z',
};

function commandArgs(args: unknown): Record<string, unknown> {
	return (args ?? {}) as Record<string, unknown>;
}

describe('Collection commands', () => {
	it('lists collections without re-reading anything', async () => {
		mockIPC((cmd) => {
			if (cmd === 'get_collections') return [collection];
		});

		expect(await listCollections()).toEqual([collection]);
		expect(invalidate).not.toHaveBeenCalled();
	});

	it('creates a collection and re-reads the collection list', async () => {
		mockIPC((cmd, args) => {
			if (
				cmd === 'create_collection' &&
				commandArgs(args).name === 'Climate Research'
			)
				return collection;
		});

		expect(await createCollection('Climate Research')).toEqual(collection);
		expect(invalidate).toHaveBeenCalledWith('app:collections');
	});

	it('deletes a collection and re-reads the collection list', async () => {
		mockIPC((cmd, args) => {
			if (
				cmd === 'delete_collection' &&
				commandArgs(args).collectionId === 'col1'
			)
				return null;
		});

		await deleteCollection('col1');
		expect(invalidate).toHaveBeenCalledWith('app:collections');
	});

	it('creates a read-only share ticket', async () => {
		mockIPC((cmd, args) => {
			if (
				cmd === 'share_collection' &&
				commandArgs(args).collectionId === 'col1' &&
				commandArgs(args).writable === false
			)
				return 'ticket-123';
		});

		expect(await shareCollection('col1')).toBe('ticket-123');
		expect(invalidate).toHaveBeenCalledWith('app:collections');
	});

	it('imports a trimmed ticket and re-reads the collection list', async () => {
		mockIPC((cmd, args) => {
			if (
				cmd === 'import_collection' &&
				commandArgs(args).ticket === 'ticket-123'
			)
				return collection;
		});

		expect(await importCollection('  ticket-123  ')).toEqual(collection);
		expect(invalidate).toHaveBeenCalledWith('app:collections');
	});

	it('propagates a failed create without re-reading', async () => {
		mockIPC((cmd) => {
			if (cmd === 'create_collection') throw new Error('disk full');
		});

		await expect(createCollection('Climate Research')).rejects.toThrow(
			'disk full',
		);
		expect(invalidate).not.toHaveBeenCalled();
	});
});

describe('Document commands', () => {
	it('lists a collection’s documents', async () => {
		mockIPC((cmd, args) => {
			if (cmd === 'get_documents' && commandArgs(args).collectionId === 'col1')
				return [document];
		});

		expect(await listDocuments('col1')).toEqual([document]);
	});

	it('re-reads the collection and its documents after a delete', async () => {
		mockIPC((cmd, args) => {
			if (
				cmd === 'delete_document' &&
				commandArgs(args).collectionId === 'col1' &&
				commandArgs(args).documentId === 'doc1'
			)
				return null;
		});

		await deleteDocument('col1', 'doc1');
		expect(invalidate).toHaveBeenCalledWith('app:collections');
		expect(invalidate).toHaveBeenCalledWith('app:collection:col1');
	});

	it('re-reads the collection and its documents after an import', async () => {
		const outcome = { documents: [document], failures: [] };
		mockIPC((cmd, args) => {
			if (
				cmd === 'start_import' &&
				commandArgs(args).collectionId === 'col1' &&
				Array.isArray(commandArgs(args).paths)
			)
				return outcome;
		});

		expect(await startImport('col1', ['/tmp/report.pdf'])).toEqual(outcome);
		expect(invalidate).toHaveBeenCalledWith('app:collections');
		expect(invalidate).toHaveBeenCalledWith('app:collection:col1');
	});

	it('reads active pipeline progress without re-reading page data', async () => {
		const progress = [
			{
				collection_id: 'col1',
				store: { pending: 1, active: 0, completed: 0, failed: 0 },
				extract: { pending: 0, active: 0, completed: 0, failed: 0 },
				index: { pending: 0, active: 0, completed: 0, failed: 0 },
			},
		];
		mockIPC((cmd) => {
			if (cmd === 'get_pipeline_progress') return progress;
		});

		expect(await getPipelineProgress()).toEqual(progress);
		expect(invalidate).not.toHaveBeenCalled();
	});
});

describe('Conversation commands', () => {
	const summary = { id: 'c1', title: 'Climate research' };
	const view = {
		id: 'c1',
		collection_id: 'col1',
		title: 'Climate research',
		messages: [],
	};

	it('lists and loads conversations without re-reading the list', async () => {
		mockIPC((cmd, args) => {
			if (cmd === 'list_conversations') return [summary];
			if (
				cmd === 'load_conversation' &&
				commandArgs(args).conversationId === 'c1'
			)
				return view;
		});

		expect(await listConversations('col1')).toEqual([summary]);
		expect(await loadConversation('col1', 'c1')).toEqual(view);
		expect(invalidate).not.toHaveBeenCalled();
	});

	it('re-reads the conversation list after starting a chat', async () => {
		mockIPC((cmd, args) => {
			if (cmd === 'start_chat' && commandArgs(args).collectionId === 'col1')
				return view;
		});

		expect(await startChat('col1')).toEqual(view);
		expect(invalidate).toHaveBeenCalledWith('app:conversations:col1');
	});

	it('re-reads the conversation list after deleting a conversation', async () => {
		mockIPC((cmd, args) => {
			if (
				cmd === 'delete_conversation' &&
				commandArgs(args).conversationId === 'c1'
			)
				return null;
		});

		await deleteConversation('col1', 'c1');
		expect(invalidate).toHaveBeenCalledWith('app:conversations:col1');
	});
});
