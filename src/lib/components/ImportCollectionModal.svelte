<script lang="ts">
	import { onMount } from 'svelte';
	import Alert from './Alert.svelte';
	import Button from './Button.svelte';
	import Modal from './Modal.svelte';
	import Textarea from './Textarea.svelte';
	import { importCollection } from '#lib/api/collection.js';
	import { describeError } from '#lib/api/provider.js';
	import type { Collection } from '#lib/types/documents.js';

	interface Props {
		ticket?: string;
		onClose: () => void;
		onImported: (collection: Collection) => void;
	}

	let { ticket = $bindable(''), onClose, onImported }: Props = $props();

	let importing = $state(false);
	let error = $state<string | null>(null);
	// Ignore an import that finishes after this modal has closed.
	let cancelled = false;

	onMount(() => {
		return () => {
			cancelled = true;
		};
	});

	async function submit() {
		if (!ticket.trim()) return;

		importing = true;
		error = null;

		try {
			const collection = await importCollection(ticket);
			if (cancelled) return;
			ticket = '';
			onImported(collection);
		} catch (e) {
			if (cancelled) return;
			console.error('Failed to import collection:', e);
			error = `Failed to import collection: ${describeError(e)}`;
		} finally {
			if (!cancelled) importing = false;
		}
	}
</script>

<Modal title="Import shared collection" {onClose}>
	<Textarea placeholder="Paste ticket here..." bind:value={ticket} rows={3} />
	{#if error}
		<Alert>{error}</Alert>
	{/if}
	{#snippet footer()}
		<Button variant="outline" onclick={onClose}>Cancel</Button>
		<Button
			variant="solid"
			onclick={submit}
			disabled={importing || !ticket.trim()}
		>
			{importing ? 'Importing...' : 'Import'}
		</Button>
	{/snippet}
</Modal>
