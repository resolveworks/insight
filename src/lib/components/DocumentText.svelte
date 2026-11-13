<script lang="ts">
	import Alert from './Alert.svelte';
	import Card from './Card.svelte';
	import LoadingSpinner from './LoadingSpinner.svelte';
	import type { DocumentSearchMatch } from '#lib/types/documents.js';

	interface Props {
		content: string | null;
		error: string | null;
		loading: boolean;
		matches: DocumentSearchMatch[];
		activeHit: number;
		visible: boolean;
	}

	interface Segment {
		text: string;
		match: boolean;
		active: boolean;
	}

	let { content, error, loading, matches, activeHit, visible }: Props =
		$props();

	let activeMark = $state<HTMLElement | null>(null);

	/**
	 * Split the content at search match offsets. Matches carry Unicode character
	 * offsets, so walk the content once by code point to convert them to UTF-16
	 * indexes.
	 */
	function buildSegments(
		text: string,
		found: DocumentSearchMatch[],
		active: number,
	): Segment[] {
		const segments: Segment[] = [];
		let scalar = 0;
		let utf16 = 0;
		let plainStart = 0;

		for (const [index, match] of found.entries()) {
			if (
				!Number.isInteger(match.offset) ||
				!Number.isInteger(match.next_offset) ||
				match.offset < 0 ||
				match.next_offset <= match.offset
			) {
				throw new Error(
					`Invalid document search match at index ${index}: offset ${match.offset}, next_offset ${match.next_offset}`,
				);
			}
			if (match.offset < scalar) {
				throw new Error(
					`Document search matches are unsorted or overlapping at index ${index}`,
				);
			}
			while (scalar < match.offset) {
				const codePoint = text.codePointAt(utf16);
				if (codePoint === undefined) {
					throw new Error(
						`Document search match at index ${index} starts beyond the document text`,
					);
				}
				utf16 += codePoint > 0xffff ? 2 : 1;
				scalar++;
			}
			const markStart = utf16;
			while (scalar < match.next_offset) {
				const codePoint = text.codePointAt(utf16);
				if (codePoint === undefined) {
					throw new Error(
						`Document search match at index ${index} ends beyond the document text`,
					);
				}
				utf16 += codePoint > 0xffff ? 2 : 1;
				scalar++;
			}
			if (plainStart < markStart) {
				segments.push({
					text: text.slice(plainStart, markStart),
					match: false,
					active: false,
				});
			}
			segments.push({
				text: text.slice(markStart, utf16),
				match: true,
				active: index === active,
			});
			plainStart = utf16;
		}

		if (plainStart < text.length) {
			segments.push({
				text: text.slice(plainStart),
				match: false,
				active: false,
			});
		}
		return segments;
	}

	const segments = $derived(
		content ? buildSegments(content, matches, activeHit) : [],
	);

	$effect(() => {
		const show = visible;
		const mark = activeMark;
		if (!show || !mark) return;
		mark.scrollIntoView({ block: 'center' });
	});
</script>

{#if loading}
	<LoadingSpinner size="sm" ariaLabel="Loading content" />
{:else if error}
	<Alert variant="warning">{error}</Alert>
{:else if content}
	<Card>
		<pre
			class="font-sans text-base leading-relaxed whitespace-pre-wrap text-neutral-800">{#each segments as segment, index (index)}{#if segment.match}{#if segment.active}<mark
							bind:this={activeMark}
							data-active="true"
							class="rounded-sm bg-yellow-300 px-0.5 text-neutral-900 ring-2 ring-neutral-800"
							>{segment.text}</mark
						>{:else}<mark
							class="rounded-sm bg-yellow-100 px-0.5 text-neutral-900"
							>{segment.text}</mark
						>{/if}{:else}{segment.text}{/if}{/each}</pre>
	</Card>
{:else}
	<p class="text-sm text-neutral-400 italic">No content available</p>
{/if}
