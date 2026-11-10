<script lang="ts">
	interface Props {
		text: string;
		tone?: 'default' | 'error';
	}

	let { text, tone = 'default' }: Props = $props();
</script>

<pre
	class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md border border-line bg-surface-dim p-3 font-mono text-xs leading-relaxed {tone ===
	'error'
		? 'text-error'
		: 'text-neutral-700'}">{text}</pre>
