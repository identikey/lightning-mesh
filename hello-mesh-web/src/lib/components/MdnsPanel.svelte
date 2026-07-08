<!--
	Advertised-services (mDNS) panel. Shows every service name announced across
	the mesh — the CRDT service directory is the mesh's mDNS analog — as one
	expandable section. Each record shows "any information" the advertiser
	carried: instance hostname, address, protocol, device MAC, and TXT records.

	Data comes from the parent's /api/directory poll (DirectoryService[]), so
	this component is presentational — no fetch of its own.
-->
<script lang="ts">
	import type { DirectoryService } from '$lib/directory/api';
	import * as Collapsible from '$lib/components/ui/collapsible/index.js';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { ChevronRight, Radio } from '@lucide/svelte';

	let { services, loaded = true }: { services: DirectoryService[]; loaded?: boolean } = $props();

	let open = $state(false);
	const count = $derived(services.length);

	function txtEntries(svc: DirectoryService): [string, string][] {
		return svc.txt ? Object.entries(svc.txt) : [];
	}
</script>

<Collapsible.Root bind:open class="rounded-lg border border-border bg-card text-card-foreground">
	<Collapsible.Trigger
		class="focus-visible:ring-ring flex w-full items-center gap-3 rounded-lg px-4 py-3 text-left transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:outline-none"
	>
		<Radio class="size-4 text-muted-foreground" aria-hidden="true" />
		<span class="flex-1">
			<span class="font-semibold">Advertised services</span>
			<span class="ml-1 text-sm text-muted-foreground">mDNS</span>
		</span>
		<Badge variant={count > 0 ? 'default' : 'secondary'}>{count}</Badge>
		<ChevronRight
			class="size-4 text-muted-foreground transition-transform duration-200 {open
				? 'rotate-90'
				: ''}"
			aria-hidden="true"
		/>
	</Collapsible.Trigger>

	<Collapsible.Content>
		<div class="border-t border-border px-4 py-3">
			{#if !loaded}
				<p class="text-sm text-muted-foreground">Loading services…</p>
			{:else if count === 0}
				<p class="text-sm text-muted-foreground">
					No services have been advertised on the mesh yet.
				</p>
			{:else}
				<ul class="flex flex-col gap-3">
					{#each services as svc (svc.name)}
						{@const txt = txtEntries(svc)}
						<li class="rounded-md border border-border bg-background/40 p-3">
							<div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
								<code class="text-sm font-medium break-all text-foreground">{svc.name}</code>
								<Badge variant="outline" class="font-mono">{svc.protocol}</Badge>
							</div>

							<dl class="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
								{#if svc.hostname}
									<dt class="text-muted-foreground">Host</dt>
									<dd class="break-all">{svc.hostname}</dd>
								{/if}
								<dt class="text-muted-foreground">Address</dt>
								<dd class="break-all">
									<code class="rounded bg-muted px-1 py-0.5 text-xs">{svc.ip}:{svc.port}</code>
								</dd>
								{#if svc.host_mac}
									<dt class="text-muted-foreground">MAC</dt>
									<dd>
										<code class="rounded bg-muted px-1 py-0.5 text-xs">{svc.host_mac}</code>
									</dd>
								{/if}
							</dl>

							{#if txt.length > 0}
								<div class="mt-2">
									<p class="mb-1 text-xs font-medium text-muted-foreground">TXT records</p>
									<div class="flex flex-wrap gap-1">
										{#each txt as [key, value] (key)}
											<Badge variant="secondary" class="font-mono">
												{key}={value}
											</Badge>
										{/each}
									</div>
								</div>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</div>
	</Collapsible.Content>
</Collapsible.Root>
