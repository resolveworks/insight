<script lang="ts">
	import { resolve } from '$app/paths';
	import List from './List.svelte';
	import ListRow from './ListRow.svelte';

	interface Props {
		collectionId: string;
		resultText: string;
	}

	let { collectionId, resultText }: Props = $props();

	interface SourceRange {
		start: number;
		end: number;
	}

	interface Quote {
		content: string;
		offset: number;
		next_offset: number;
		source_range: SourceRange | null;
	}

	interface Excerpt {
		content: string;
	}

	interface SearchHit {
		document_name: string;
		document_id: string;
		quote: Quote | null;
		excerpt: Excerpt | null;
	}

	function isRecord(value: unknown): value is Record<string, unknown> {
		return value !== null && typeof value === 'object' && !Array.isArray(value);
	}

	function stringField(
		value: Record<string, unknown>,
		field: string,
		context: string,
	): string {
		const result = value[field];
		if (typeof result !== 'string') {
			throw new Error(`${context} has no string ${field}`);
		}
		return result;
	}

	function offsetField(
		value: Record<string, unknown>,
		field: string,
		context: string,
	): number {
		const result = value[field];
		if (!Number.isInteger(result) || (result as number) < 0) {
			throw new Error(`${context} has no non-negative integer ${field}`);
		}
		return result as number;
	}

	function sourceRange(value: unknown, context: string): SourceRange | null {
		if (value === null) return null;
		if (!isRecord(value)) {
			throw new Error(`${context} has an invalid source_range`);
		}
		const start = offsetField(value, 'start', `${context} source_range`);
		const end = offsetField(value, 'end', `${context} source_range`);
		return { start, end };
	}

	function parseQuote(value: unknown, context: string): Quote | null {
		if (value === null) return null;
		if (!isRecord(value)) {
			throw new Error(`${context} has an invalid quote`);
		}
		return {
			content: stringField(value, 'content', `${context} quote`),
			offset: offsetField(value, 'offset', `${context} quote`),
			next_offset: offsetField(value, 'next_offset', `${context} quote`),
			source_range: sourceRange(value.source_range, `${context} quote`),
		};
	}

	function parseExcerpt(value: unknown, context: string): Excerpt | null {
		if (value === null) return null;
		if (!isRecord(value)) {
			throw new Error(`${context} has an invalid excerpt`);
		}
		return { content: stringField(value, 'content', `${context} excerpt`) };
	}

	function parseSearchOutput(text: string): SearchHit[] {
		let parsed: unknown;
		try {
			parsed = JSON.parse(text);
		} catch {
			throw new Error('Search result is not valid JSON');
		}
		if (!isRecord(parsed) || !Array.isArray(parsed.hits)) {
			throw new Error('Search result has no hits list');
		}

		return parsed.hits.map((value, index) => {
			const context = `Search hit ${index + 1}`;
			if (!isRecord(value)) {
				throw new Error(`${context} is not an object`);
			}
			return {
				document_name: stringField(value, 'document_name', context),
				document_id: stringField(value, 'document_id', context),
				quote: parseQuote(value.quote, context),
				excerpt: parseExcerpt(value.excerpt, context),
			};
		});
	}

	const hits = $derived(parseSearchOutput(resultText));

	function hrefFor(hit: SearchHit): string {
		const path = resolve(
			`collections/${collectionId}/documents/${hit.document_id}`,
		);
		if (hit.quote === null) return path;

		const params = new URLSearchParams({ q: hit.quote.content });
		return `${path}?${params.toString()}`;
	}

	function pageLabel(range: SourceRange): string {
		return range.start === range.end
			? `Page ${range.start}`
			: `Pages ${range.start}–${range.end}`;
	}
</script>

{#if hits.length === 0}
	<p class="text-sm text-neutral-500">No documents found.</p>
{:else}
	<List>
		{#each hits as hit (hit.document_id)}
			<ListRow href={hrefFor(hit)}>
				<span class="text-sm text-neutral-800">{hit.document_name}</span>
				{#if hit.quote?.source_range}
					<p class="mt-1 text-xs text-neutral-500">
						{pageLabel(hit.quote.source_range)}
					</p>
				{/if}
				{#if hit.excerpt}
					<p class="mt-1 line-clamp-2 text-xs text-neutral-500">
						{hit.excerpt.content}
					</p>
				{/if}
			</ListRow>
		{/each}
	</List>
{/if}
