import { describe, it, expect, vi } from 'vitest';
import { load } from './+layout';

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

describe('Collection workspace load', () => {
	it('registers its dependency and finds the collection without loading its documents', async () => {
		const depends = vi.fn();
		const parent = vi.fn(async () => ({ collections: [collection] }));

		const data = (await load({
			params: { collectionId: 'col1' },
			depends,
			parent,
		} as unknown as Parameters<typeof load>[0])) as { collection: unknown };

		expect(depends).toHaveBeenCalledWith('app:collection:col1');
		expect(data.collection).toEqual(collection);
		expect(data).not.toHaveProperty('documents');
	});
});
