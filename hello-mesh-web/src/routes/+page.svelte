<!-- Identity affordances (S6), the mesh directory (S5), and the advertised
     services / mDNS panel. One shared /api/directory poll (directoryStore)
     backs the directory grid and the services panel. No external hosts — must
     run fully offline. -->
<script lang="ts">
	import IdentityPanel from '$lib/components/IdentityPanel.svelte';
	import DirectoryPanel from '$lib/components/DirectoryPanel.svelte';
	import MdnsPanel from '$lib/components/MdnsPanel.svelte';
	import TopologyPanel from '$lib/components/TopologyPanel.svelte';
	import { directoryStore, startDirectoryPolling } from '$lib/directory/store.svelte';

	$effect(startDirectoryPolling);

	const services = $derived(directoryStore.directory?.services ?? []);
</script>

<main class="mx-auto flex max-w-2xl flex-col gap-6 p-6">
	<header>
		<h1 class="text-2xl font-semibold">hello.mesh</h1>
		<p class="text-muted-foreground">Lightning Mesh front desk.</p>
	</header>

	<IdentityPanel />

	<DirectoryPanel />

	<TopologyPanel />

	<MdnsPanel {services} loaded={directoryStore.loaded} />
</main>
