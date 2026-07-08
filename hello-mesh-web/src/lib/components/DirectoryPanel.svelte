<script lang="ts">
	import { directoryStore } from '$lib/directory/store.svelte';

	const directory = $derived(directoryStore.directory);
	const reconnecting = $derived(directoryStore.reconnecting);

	let nodeIdShort = $derived(directory ? shortId(directory.node.node_id) : '');
	let subnetLabel = $derived(directory?.node.subnet ?? 'no subnet assigned');

	function shortId(id: string): string {
		return id.length > 12 ? `${id.slice(0, 12)}…` : id;
	}
</script>

<section
	class="flex flex-col gap-4 rounded-lg border border-border bg-card p-4 text-card-foreground"
>
	<header class="flex flex-col gap-1">
		<h2 class="text-lg font-semibold">Mesh directory</h2>
		{#if directory}
			<p class="text-sm text-muted-foreground">
				You are here: <code class="rounded bg-muted px-1 py-0.5 text-xs">{nodeIdShort}</code>
				· {subnetLabel}
			</p>
		{:else}
			<p class="text-sm text-muted-foreground">Loading directory…</p>
		{/if}
		{#if reconnecting}
			<p class="text-xs text-warning">Reconnecting to this node…</p>
		{/if}
	</header>

	{#if directory}
		<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
			<div class="flex flex-col gap-2">
				<h3 class="text-sm font-medium">Neighbors</h3>
				{#if directory.neighbors.length === 0}
					<p class="text-sm text-muted-foreground">No neighbors yet</p>
				{:else}
					<ul class="flex flex-col gap-1">
						{#each directory.neighbors as neighbor (neighbor.node_id)}
							<li class="text-sm">
								<code class="rounded bg-muted px-1 py-0.5 text-xs">{shortId(neighbor.node_id)}</code
								>
								{#if neighbor.subnet}
									<span class="text-muted-foreground">· {neighbor.subnet}</span>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</div>

			<div class="flex flex-col gap-2">
				<h3 class="text-sm font-medium">Identities</h3>
				{#if directory.identities.length === 0}
					<p class="text-sm text-muted-foreground">No identities yet</p>
				{:else}
					<ul class="flex flex-col gap-1">
						{#each directory.identities as identity (identity.username)}
							<li class="text-sm">
								{identity.display_name}
								<span class="text-muted-foreground">@{identity.username}</span>
							</li>
						{/each}
					</ul>
				{/if}
			</div>
		</div>
	{/if}
</section>
