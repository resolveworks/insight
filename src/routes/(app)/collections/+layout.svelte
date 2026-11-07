<script lang="ts">
	let { children } = $props();
</script>

<div class="h-full overflow-hidden bg-surface">
	{@render children()}
</div>
