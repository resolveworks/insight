<script lang="ts">
	import Modal from './Modal.svelte';
	import ShareTicket from './ShareTicket.svelte';
	import { shareCollection } from '#lib/api/collection.js';
	import { describeError } from '#lib/api/provider.js';

	interface Props {
		collectionId: string;
		onClose: () => void;
	}

	let { collectionId, onClose }: Props = $props();

	let ticket = $state<string | null>(null);
	let error = $state<string | null>(null);
	let copied = $state(false);
	let copyError = $state<string | null>(null);

	const shareState = $derived(
		error
			? { status: 'error' as const, message: error }
			: ticket
				? { status: 'ready' as const, ticket }
				: { status: 'loading' as const },
	);

	$effect(() => {
		// Closing the modal must not apply this request's result to a newer session.
		let cancelled = false;

		shareCollection(collectionId).then(
			(result) => {
				if (!cancelled) ticket = result;
			},
			(e) => {
				if (cancelled) return;
				console.error('Failed to share collection:', e);
				error = `Failed to generate share ticket: ${describeError(e)}`;
			},
		);

		return () => {
			cancelled = true;
		};
	});

	async function copyTicket() {
		if (!ticket) return;
		try {
			await navigator.clipboard.writeText(ticket);
			copyError = null;
			copied = true;
			setTimeout(() => (copied = false), 2000);
		} catch (e) {
			console.error('Failed to copy ticket:', e);
			copyError = "Couldn't copy the ticket. Select it and copy it manually.";
		}
	}
</script>

<Modal title="Share collection" {onClose}>
	<ShareTicket state={shareState} {copied} {copyError} onCopy={copyTicket} />
</Modal>
