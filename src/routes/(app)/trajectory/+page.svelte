<script lang="ts">
	import Chat from '$lib/components/Chat.svelte';
	import ConversationSidebar from '$lib/components/ConversationSidebar.svelte';

	let chatComponent = $state<Chat | null>(null);
	let conversationSidebar = $state<ConversationSidebar | null>(null);
	let activeConversationId = $state<string | null>(null);
</script>

<div class="flex h-full">
	<ConversationSidebar
		bind:this={conversationSidebar}
		{activeConversationId}
		onSelect={async (id) => {
			activeConversationId = id;
			await chatComponent?.loadConversation(id);
		}}
		onNew={async () => {
			activeConversationId = null;
			await chatComponent?.newConversation();
		}}
	/>
	<div class="flex-1">
		<Chat
			bind:this={chatComponent}
			onConversationStart={(id) => {
				activeConversationId = id;
				conversationSidebar?.refresh();
			}}
		/>
	</div>
</div>
