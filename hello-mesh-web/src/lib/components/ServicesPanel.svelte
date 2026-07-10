<!--
	Services: the .mesh directory of things you can open on this network. This is
	the mesh's own service directory (the "mDNS analog"), but the word mDNS never
	appears in user-facing copy — residents just see openable services. Web
	services (http/https) render as real, clickable links; everything else shows
	its address. Presentational: fed by the parent's /api/directory poll.
-->
<script lang="ts">
	import type { DirectoryService } from '$lib/directory/api';
	import * as Collapsible from '$lib/components/ui/collapsible/index.js';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { ChevronRight, Compass, ExternalLink } from '@lucide/svelte';

	let { services, loaded = true }: { services: DirectoryService[]; loaded?: boolean } = $props();

	// Open by default: Services is how walk-up users find apps on this mesh, so it
	// leads the page and shows its contents without a tap (mjolnir-mesh-kgq).
	let open = $state(true);
	const count = $derived(services.length);

	function txtEntries(svc: DirectoryService): [string, string][] {
		return svc.txt ? Object.entries(svc.txt) : [];
	}

	/** A real URL for web services; undefined for non-web protocols. Prefer the
	 *  `.mesh` name over the raw IP: it's what people should see and share, and it
	 *  survives the host's IP changing (e.g. roaming between node subnets). A
	 *  dotted mDNS-style name (`printer._ipp._tcp`) isn't a resolvable host, so
	 *  those fall back to the IP. The port is dropped when it's the scheme default
	 *  (or unspecified), so a name served on 443 links as bare `https://name.mesh`. */
	function webUrl(svc: DirectoryService): string | undefined {
		const p = svc.protocol.toLowerCase();
		if (p !== 'http' && p !== 'https') return undefined;
		const isMdns = /\._[a-z]+\._[a-z]+$/i.test(svc.name);
		const host = isMdns ? svc.ip : `${svc.name}.mesh`;
		const defaultPort = p === 'https' ? 443 : 80;
		const port = svc.port && svc.port !== defaultPort ? `:${svc.port}` : '';
		return `${p}://${host}${port}`;
	}

	/** Friendly service name — strip the trailing _proto._transport if present. */
	function niceName(name: string): string {
		return name.replace(/\._[a-z]+\._[a-z]+$/i, '');
	}
</script>

<Collapsible.Root bind:open class="rounded-lg border border-border bg-card text-card-foreground">
	<Collapsible.Trigger
		class="flex w-full items-center gap-3 rounded-lg px-4 py-3 text-left transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
	>
		<Compass class="size-4 text-muted-foreground" aria-hidden="true" />
		<span class="flex-1">
			<span class="font-semibold">Services</span>
			<span class="ml-1 text-sm text-muted-foreground">.mesh directory</span>
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
			<p class="mb-3 text-sm text-muted-foreground">Things you can open on this mesh.</p>
			{#if !loaded}
				<p class="text-sm text-muted-foreground">Loading services…</p>
			{:else if count === 0}
				<p class="text-sm text-muted-foreground">Nothing has been shared on the mesh yet.</p>
			{:else}
				<ul class="flex flex-col gap-3">
					{#each services as svc (svc.name)}
						{@const txt = txtEntries(svc)}
						{@const url = webUrl(svc)}
						<li class="rounded-md border border-border bg-background/40 p-3">
							<div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
								{#if url}
									<!-- External service on another mesh host, not an app route. -->
									<!-- eslint-disable svelte/no-navigation-without-resolve -->
									<a
										href={url}
										class="flex items-center gap-1.5 text-sm font-medium break-all text-primary hover:underline"
									>
										{niceName(svc.name)}
										<ExternalLink class="size-3.5 shrink-0" aria-hidden="true" />
									</a>
									<!-- eslint-enable svelte/no-navigation-without-resolve -->
								{:else}
									<span class="text-sm font-medium break-all text-foreground"
										>{niceName(svc.name)}</span
									>
								{/if}
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
									<dt class="text-muted-foreground">Device</dt>
									<dd>
										<code class="rounded bg-muted px-1 py-0.5 text-xs">{svc.host_mac}</code>
									</dd>
								{/if}
							</dl>

							{#if txt.length > 0}
								<div class="mt-2 flex flex-wrap gap-1">
									{#each txt as [key, value] (key)}
										<Badge variant="secondary" class="font-mono">{key}={value}</Badge>
									{/each}
								</div>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</div>
	</Collapsible.Content>
</Collapsible.Root>
