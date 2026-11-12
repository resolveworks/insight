import { describe, it, expect } from 'vitest';
import { buildRendererIndex, type Renderer } from './registry';

function stub(mimes: readonly string[]): Renderer {
	return {
		mimes,
		load: async () => {
			throw new Error('load is not called');
		},
	};
}

describe('buildRendererIndex', () => {
	it('resolves each MIME of a registered renderer to it', () => {
		const pdf = stub(['application/pdf', 'application/x-pdf']);
		const index = buildRendererIndex([pdf]);

		expect(index.get('application/pdf')).toBe(pdf);
		expect(index.get('application/x-pdf')).toBe(pdf);
	});

	it('returns undefined for an unknown MIME', () => {
		const index = buildRendererIndex([stub(['application/pdf'])]);

		expect(index.get('text/plain')).toBeUndefined();
	});

	it('throws naming the MIME when two renderers claim it', () => {
		const index = () =>
			buildRendererIndex([
				stub(['application/pdf']),
				stub(['application/pdf']),
			]);

		expect(index).toThrow(/application\/pdf/);
	});
});
