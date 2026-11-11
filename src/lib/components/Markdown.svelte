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

<style>
	.markdown {
		overflow-wrap: anywhere;
	}

	.markdown > :global(:first-child) {
		margin-top: 0;
	}
	.markdown > :global(:last-child) {
		margin-bottom: 0;
	}

	.markdown :global(p) {
		margin: 0.75em 0;
	}

	.markdown :global(strong) {
		font-weight: 600;
	}
	.markdown :global(em) {
		font-style: italic;
	}

	/* Prose headings: a model-emitted document heading should not
	   overpower the answer it belongs to. */
	.markdown :global(h1),
	.markdown :global(h2),
	.markdown :global(h3),
	.markdown :global(h4),
	.markdown :global(h5),
	.markdown :global(h6) {
		font-weight: 600;
		line-height: 1.3;
		margin: 1.25em 0 0.5em;
	}
	.markdown :global(h1),
	.markdown :global(h2),
	.markdown :global(h3) {
		font-size: 1rem;
	}
	.markdown :global(h4),
	.markdown :global(h5),
	.markdown :global(h6) {
		font-size: 0.875rem;
	}

	/* Tailwind's preflight strips list markers; restore them. */
	.markdown :global(ul),
	.markdown :global(ol) {
		margin: 0.75em 0;
		padding-left: 1.375em;
	}
	.markdown :global(ul) {
		list-style: disc;
	}
	.markdown :global(ol) {
		list-style: decimal;
	}
	.markdown :global(li) {
		margin: 0.25em 0;
	}
	.markdown :global(li > p) {
		margin: 0;
	}
	.markdown :global(li > ul),
	.markdown :global(li > ol) {
		margin: 0.25em 0;
	}
	.markdown :global(li::marker) {
		color: var(--color-neutral-500);
	}

	.markdown :global(code) {
		background: var(--color-neutral-100);
		border: 1px solid var(--color-neutral-200);
		border-radius: var(--radius-sm);
		color: var(--color-neutral-700);
		font-family:
			ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas,
			'Liberation Mono', monospace;
		font-size: 0.75rem;
		padding: 0.1em 0.35em;
	}
	.markdown :global(pre) {
		background: var(--color-neutral-100);
		border: 1px solid var(--color-neutral-200);
		border-radius: var(--radius-md);
		color: var(--color-neutral-700);
		font-size: 0.75rem;
		line-height: 1.625;
		margin: 0.75em 0;
		overflow-x: auto;
		padding: 0.75rem;
	}
	.markdown :global(pre code) {
		background: none;
		border: 0;
		font-size: inherit;
		padding: 0;
	}

	.markdown :global(blockquote) {
		border-left: 2px solid var(--color-neutral-300);
		color: var(--color-neutral-500);
		margin: 0.75em 0;
		padding-left: 0.875em;
	}

	.markdown :global(a) {
		text-decoration: underline;
		text-underline-offset: 2px;
	}

	.markdown :global(hr) {
		border: 0;
		border-top: 1px solid var(--color-neutral-200);
		margin: 1.25em 0;
	}

	.markdown :global(table) {
		border-collapse: collapse;
		font-size: 0.875em;
		margin: 0.75em 0;
		width: 100%;
	}
	.markdown :global(th),
	.markdown :global(td) {
		border: 1px solid var(--color-neutral-200);
		padding: 0.375rem 0.5rem;
		text-align: left;
		vertical-align: top;
	}
	.markdown :global(th) {
		background: var(--color-neutral-100);
		font-weight: 600;
	}

	.markdown :global(img) {
		border-radius: var(--radius-md);
		max-width: 100%;
	}
</style>
