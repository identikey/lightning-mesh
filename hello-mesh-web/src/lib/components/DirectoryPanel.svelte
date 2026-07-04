<script lang="ts">
	import { browser } from '$app/environment';
	import { fetchDirectory, type Directory } from '$lib/directory/api';

	const POLL_INTERVAL_MS = 5000;

	let directory = $state<Directory | undefined>(undefined);
	let reconnecting = $state(false);

	let nodeIdShort = $derived(directory ? shortId(directory.node.node_id) : '');
	let subnetLabel = $derived(directory?.node.subnet ?? 'no subnet assigned');

	function shortId(id: string): string {
		return id.length > 12 ? `${id.slice(0, 12)}…` : id;
	}

	async function poll() {
		try {
			const next = await fetchDirectory();
			directory = next;
			reconnecting = false;
		} catch {
			// Keep last-good directory; surface a subtle hint, never a hard error.
			reconnecting = true;
		}
	}

	$effect(() => {
		if (!browser) return;
		poll();
		const interval = setInterval(poll, POLL_INTERVAL_MS);
		return () => clearInterval(interval);
	});
</script>

<section class="flex flex-col gap-4 rounded-lg border border-slate-700 bg-slate-900/40 p-4">
	<header class="flex flex-col gap-1">
		<h2 class="text-lg font-semibold text-slate-100">Mesh directory</h2>
		{#if directory}
			<p class="text-sm text-slate-400">
				You are here: <code class="rounded bg-slate-800 px-1 py-0.5 text-xs">{nodeIdShort}</code>
				· {subnetLabel}
			</p>
		{:else}
			<p class="text-sm text-slate-400">Loading directory…</p>
		{/if}
		{#if reconnecting}
			<p class="text-xs text-amber-400">Reconnecting to this node…</p>
		{/if}
	</header>

	{#if directory}
		<div class="grid grid-cols-1 gap-4 sm:grid-cols-3">
			<div class="flex flex-col gap-2">
				<h3 class="text-sm font-medium text-slate-200">Neighbors</h3>
				{#if directory.neighbors.length === 0}
					<p class="text-sm text-slate-500">No neighbors yet</p>
				{:else}
					<ul class="flex flex-col gap-1">
						{#each directory.neighbors as neighbor (neighbor.node_id)}
							<li class="text-sm text-slate-300">
								<code class="rounded bg-slate-800 px-1 py-0.5 text-xs"
									>{shortId(neighbor.node_id)}</code
								>
								{#if neighbor.subnet}
									<span class="text-slate-500">· {neighbor.subnet}</span>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</div>

			<div class="flex flex-col gap-2">
				<h3 class="text-sm font-medium text-slate-200">Identities</h3>
				{#if directory.identities.length === 0}
					<p class="text-sm text-slate-500">No identities yet</p>
				{:else}
					<ul class="flex flex-col gap-1">
						{#each directory.identities as identity (identity.username)}
							<li class="text-sm text-slate-300">
								{identity.display_name}
								<span class="text-slate-500">@{identity.username}</span>
							</li>
						{/each}
					</ul>
				{/if}
			</div>

			<div class="flex flex-col gap-2">
				<h3 class="text-sm font-medium text-slate-200">Services</h3>
				{#if directory.services.length === 0}
					<p class="text-sm text-slate-500">No services yet</p>
				{:else}
					<ul class="flex flex-col gap-1">
						{#each directory.services as service (service.name + service.ip + service.port)}
							<li class="text-sm text-slate-300">
								{service.name}
								<span class="text-slate-500"
									>· {service.ip}:{service.port} ({service.protocol})</span
								>
							</li>
						{/each}
					</ul>
				{/if}
			</div>
		</div>
	{/if}
</section>
