<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    title: string;
    children: Snippet;
  };

  let { title, children }: Props = $props();
</script>

<aside class="w-56 overflow-y-auto border-r border-slate-700 bg-slate-800 p-4">
  <h2 class="mb-3 text-xs font-medium uppercase tracking-wide text-slate-400">{title}</h2>
  {@render children()}
</aside>
