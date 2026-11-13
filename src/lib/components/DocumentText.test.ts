import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import DocumentText from './DocumentText.svelte';
import type { DocumentSearchMatch } from '#lib/types/documents.js';

function match(
	offset: number,
	nextOffset: number,
	content: string,
): DocumentSearchMatch {
	return {
		content,
		offset,
		next_offset: nextOffset,
		source_range: null,
	};
}

function props(
	content: string,
	matches: DocumentSearchMatch[],
	activeHit = 0,
	visible = true,
) {
	return {
		content,
		error: null,
		loading: false,
		matches,
		activeHit,
		visible,
	};
}

beforeEach(() => {
	Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
	cleanup();
	vi.restoreAllMocks();
});

describe('DocumentText search highlights', () => {
	it('marks each match and distinguishes the active one', () => {
		const content = 'alpha beta alpha gamma';
		const { container } = render(DocumentText, {
			props: props(content, [match(0, 5, 'alpha'), match(11, 16, 'alpha')], 1),
		});

		const marks = container.querySelectorAll('mark');
		expect(marks).toHaveLength(2);
		expect(marks[0]).not.toHaveAttribute('data-active');
		expect(marks[1]).toHaveAttribute('data-active', 'true');
		expect(container.querySelector('pre')?.textContent).toBe(content);
	});

	it('converts Unicode character offsets, including astral characters', () => {
		const content = '😀 one β 😀';
		const { container } = render(DocumentText, {
			props: props(
				content,
				[match(0, 1, '😀'), match(6, 7, 'β'), match(8, 9, '😀')],
				1,
			),
		});

		const marks = container.querySelectorAll('mark');
		expect([...marks].map((mark) => mark.textContent)).toEqual([
			'😀',
			'β',
			'😀',
		]);
		expect(marks[1]).toHaveAttribute('data-active', 'true');
		expect(container.querySelector('pre')?.textContent).toBe(content);
	});

	it('scrolls the active match into view only while visible', async () => {
		const scrollIntoView = vi.spyOn(Element.prototype, 'scrollIntoView');
		const content = 'alpha beta alpha';
		const matches = [match(0, 5, 'alpha'), match(11, 16, 'alpha')];

		const { rerender } = render(DocumentText, {
			props: props(content, matches, 0, false),
		});
		expect(scrollIntoView).not.toHaveBeenCalled();

		await rerender(props(content, matches, 1, true));

		expect(scrollIntoView).toHaveBeenCalledWith({ block: 'center' });
	});

	it('throws on unsorted match data', () => {
		expect(() =>
			render(DocumentText, {
				props: props('abcdef', [match(4, 5, 'e'), match(1, 2, 'b')]),
			}),
		).toThrow(/unsorted or overlapping/);
	});

	it('throws when a match lies beyond the content', () => {
		expect(() =>
			render(DocumentText, {
				props: props('abc', [match(3, 4, 'd')]),
			}),
		).toThrow(/beyond the document text/);
	});
});
