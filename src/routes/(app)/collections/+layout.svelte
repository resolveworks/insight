<script lang="ts">
	let { children } = $props();
</script>

<div class="h-full overflow-y-auto bg-surface">
	{@render children()}
</div>
