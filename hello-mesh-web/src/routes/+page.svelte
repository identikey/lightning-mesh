<!-- Identity affordances (S6) live here. The neighbor/service directory (S5)
     is a separate story — it slots in alongside this section, not inside it.
     No external hosts — must run fully offline. -->
<script lang="ts">
	import IdentityPanel from '$lib/components/IdentityPanel.svelte';
	import DirectoryPanel from '$lib/components/DirectoryPanel.svelte';
</script>

<main class="mx-auto flex max-w-2xl flex-col gap-6 p-6 text-slate-100">
	<header>
		<h1 class="text-2xl font-semibold">hello.mesh</h1>
		<p class="text-slate-400">Lightning Mesh front desk.</p>
	</header>

	<IdentityPanel />

	<DirectoryPanel />
</main>
