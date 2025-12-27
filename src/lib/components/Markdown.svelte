<script lang="ts">
	import { marked } from 'marked';

	type Props = {
		content: string;
	};

	let { content }: Props = $props();

	// Configure marked for safe rendering
	marked.setOptions({
		breaks: true, // Convert \n to <br>
		gfm: true, // GitHub Flavored Markdown
	});

	const html = $derived(marked.parse(content, { async: false }) as string);
</script>

<div class="markdown">
	<!-- eslint-disable-next-line svelte/no-at-html-tags -- LLM output, not user input -->
	{@html html}
</div>

<style lang="postcss">
	.markdown :global(p) {
		margin: 0.5em 0;
	}
	.markdown :global(p:first-child) {
		margin-top: 0;
	}
	.markdown :global(p:last-child) {
		margin-bottom: 0;
	}
	.markdown :global(strong) {
		font-weight: 600;
	}
	.markdown :global(em) {
		font-style: italic;
	}
	.markdown :global(code) {
		background: rgba(0, 0, 0, 0.3);
		padding: 0.1em 0.3em;
		border-radius: 0.25em;
		font-size: 0.9em;
	}
	.markdown :global(pre) {
		background: rgba(0, 0, 0, 0.3);
		padding: 0.75em 1em;
		border-radius: 0.375em;
		overflow-x: auto;
		margin: 0.5em 0;
	}
	.markdown :global(pre code) {
		background: none;
		padding: 0;
	}
	.markdown :global(ul),
	.markdown :global(ol) {
		margin: 0.5em 0;
		padding-left: 1.5em;
	}
	.markdown :global(li) {
		margin: 0.25em 0;
	}
	.markdown :global(blockquote) {
		border-left: 3px solid currentColor;
		opacity: 0.8;
		padding-left: 1em;
		margin: 0.5em 0;
	}
	.markdown :global(a) {
		color: inherit;
		text-decoration: underline;
	}
	.markdown :global(h1),
	.markdown :global(h2),
	.markdown :global(h3),
	.markdown :global(h4) {
		font-weight: 600;
		margin: 0.75em 0 0.25em;
	}
	.markdown :global(h1:first-child),
	.markdown :global(h2:first-child),
	.markdown :global(h3:first-child),
	.markdown :global(h4:first-child) {
		margin-top: 0;
	}
</style>
