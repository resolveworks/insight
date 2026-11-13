<script lang="ts">
	import type { DocumentSearchResults } from '#lib/types/documents.js';
	import Alert from './Alert.svelte';
	import Button from './Button.svelte';
	import Icon from './Icon.svelte';
	import Input from './Input.svelte';

	interface Props {
		query: string;
		results: DocumentSearchResults | null;
		activeHit: number;
		searching: boolean;
		error: string | null;
		disabled: boolean;
		onquerychange: (query: string) => void;
		onprevious: () => void;
		onnext: () => void;
		onclear: () => void;
	}

	let {
		query = $bindable(''),
		results,
		activeHit,
		searching,
		error,
		disabled,
		onquerychange,
		onprevious,
		onnext,
		onclear,
	}: Props = $props();

	const matchCount = $derived(results?.matches.length ?? 0);
	const canNavigate = $derived(matchCount > 0);
	const readout = $derived(
		searching
			? 'Searching…'
			: matchCount > 0
				? `${activeHit + 1} of ${matchCount}`
				: results !== null && query.trim() !== ''
					? 'No matches'
					: '',
	);
</script>

<div class="flex flex-col gap-2">
	<div class="flex items-end gap-2">
		<div class="min-w-0 flex-1">
			<Input
				id="document-search"
				label="Search document"
				placeholder={disabled
					? 'Search available once indexing finishes'
					: 'Search this document'}
				{disabled}
				bind:value={query}
				oninput={onquerychange}
			/>
		</div>
		<div class="flex items-center gap-1">
			<Button
				variant="outline"
				size="icon"
				label="Previous match"
				disabled={!canNavigate}
				onclick={onprevious}
			>
				<Icon name="chevron-left" size="sm" />
			</Button>
			<Button
				variant="outline"
				size="icon"
				label="Next match"
				disabled={!canNavigate}
				onclick={onnext}
			>
				<Icon name="chevron-right" size="sm" />
			</Button>
			{#if query !== ''}
				<Button
					variant="ghost"
					size="icon"
					label="Clear search"
					onclick={onclear}
				>
					<Icon name="close" size="sm" />
				</Button>
			{/if}
		</div>
	</div>
	{#if readout}
		<p class="text-sm text-neutral-600" role="status" aria-live="polite">
			{readout}
		</p>
	{/if}
	{#if error}
		<Alert>{error}</Alert>
	{/if}
</div>
