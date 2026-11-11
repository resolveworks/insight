<script lang="ts">
	import Markdown from './Markdown.svelte';

	type Props = {
		from: 'user' | 'assistant';
		text: string;
		streaming?: boolean;
	};

	let { from, text, streaming = false }: Props = $props();

	// A message is a container, so it takes the container radius while the
	// user surface reuses the solid button token to mean "authored by the user".
	const surface = $derived(
		from === 'user'
			? 'ml-auto w-fit max-w-[80%] rounded-lg bg-neutral-900 px-4 py-3 text-base text-white'
			: 'text-base text-neutral-800',
	);
</script>

<div class={surface}>
	<Markdown content={text} />
	{#if streaming}
		<span
			class="ml-0.5 inline-block h-4 w-0.5 animate-pulse bg-neutral-800 align-text-bottom"
		></span>
	{/if}
</div>
