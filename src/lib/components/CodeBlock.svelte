<script lang="ts">
	interface Props {
		text: string;
	}

	let { text }: Props = $props();
</script>

<pre
	class="max-h-64 overflow-auto rounded-md border border-neutral-200 bg-neutral-100 p-3 font-mono text-xs leading-relaxed break-words whitespace-pre-wrap text-neutral-700">{text}</pre>
