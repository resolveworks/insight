<script lang="ts">
	import Disclosure from './Disclosure.svelte';
	import Icon from './Icon.svelte';
	import LoadingSpinner from './LoadingSpinner.svelte';
	import { formatResult, type ActivityStep } from '#lib/chat/activity.js';

	type Props = { step: ActivityStep };

	let { step }: Props = $props();

	const isError = $derived(step.kind === 'tool' && step.isError);
</script>

{#snippet preBlock(text: string)}
	<pre
		class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded bg-surface-dim p-2 font-mono text-xs leading-relaxed {isError
			? 'text-error'
			: 'text-neutral-700'}">{text}</pre>
{/snippet}

{#if step.kind === 'reasoning'}
	<Disclosure summaryClass="flex items-center gap-2 py-1 text-xs">
		{#snippet summary()}
			{#if step.streaming || step.text.trim() === ''}
				<LoadingSpinner size="xs" ariaLabel="Thinking" />
			{/if}
			<span class="shrink-0 font-medium text-neutral-600">Thought</span>
			{#if !step.streaming && step.text.trim() !== ''}
				<span class="min-w-0 truncate text-subtle">{step.preview}</span>
			{/if}
		{/snippet}
		<pre
			class="whitespace-pre-wrap break-words text-xs text-neutral-600">{step.text}</pre>
	</Disclosure>
{:else}
	<Disclosure
		open={step.isError}
		summaryClass="flex items-center gap-2 py-1 text-xs"
	>
		{#snippet summary()}
			{#if step.executing}
				<LoadingSpinner size="xs" ariaLabel="Running" />
			{:else if step.isError}
				<Icon name="warning" size="xs" class="text-error" label="Failed" />
			{:else}
				<Icon name="check" size="xs" class="text-success" label="Finished" />
			{/if}
			<span class="min-w-0 truncate">{step.label}</span>
			{#if step.hint}
				<span class="ml-auto shrink-0 pl-2 text-subtle">{step.hint}</span>
			{/if}
		{/snippet}

		<div class="space-y-2 pb-2 pr-2 pl-5 pt-1">
			{#if step.argsLines.length > 0}
				<dl class="space-y-1 font-mono text-xs leading-relaxed">
					{#each step.argsLines as line (line.key)}
						<div class="flex gap-2">
							<dt class="shrink-0 text-subtle">{line.key}:</dt>
							<dd
								class="min-w-0 whitespace-pre-wrap break-words text-neutral-600"
							>
								{line.value}
							</dd>
						</div>
					{/each}
				</dl>
				{#if step.argsTruncated}
					{@render preBlock(step.argsText)}
				{/if}
			{:else if !step.argsParseable && step.argsText.trim()}
				{@render preBlock(step.argsText)}
			{/if}

			{#if step.resultText !== null}
				{@render preBlock(formatResult(step.resultText))}
			{/if}
		</div>
	</Disclosure>
{/if}
