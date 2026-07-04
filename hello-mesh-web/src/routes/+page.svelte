<script>
	// SCAFFOLD ONLY — this page is the S1 placeholder landing page.
	// S5 (mjolnir-mesh-8aw) wires in the real directory list.
	// S6 (mjolnir-mesh-c3f) wires in the identity affordances.
	//
	// Stub of the S5 directory fetch, left commented so the shape of the
	// contract is visible here without building the real UI yet:
	//
	// import { onMount } from 'svelte';
	// let directory = $state(null);
	// onMount(async () => {
	// 	const res = await fetch('/api/directory');
	// 	directory = await res.json();
	// 	// => { node: {...self}, neighbors: [...], services: [...] }
	// });
</script>

<main>
	<h1>hello.mesh</h1>
	<p class="tagline">the front desk for this mesh node</p>

	<section class="placeholder" aria-label="directory placeholder">
		<h2>Directory</h2>
		<p>
			Neighbors, identities, and services on this mesh will appear here.
			<br />
			<em>(scaffold: real directory UI lands in S5)</em>
		</p>
	</section>

	<section class="placeholder" aria-label="identity placeholder">
		<h2>Identity</h2>
		<p>
			"Just browse" or create a soft rung-1 identity will live here.
			<br />
			<em>(scaffold: real identity affordances land in S6)</em>
		</p>
	</section>
</main>

<style>
	:global(html),
	:global(body) {
		margin: 0;
		padding: 0;
		background: #0b0d10;
		color: #e6e8eb;
		font-family:
			system-ui,
			-apple-system,
			'Segoe UI',
			sans-serif;
	}

	main {
		max-width: 40rem;
		margin: 0 auto;
		padding: 3rem 1.5rem;
	}

	h1 {
		font-size: 2.25rem;
		margin-bottom: 0.25rem;
	}

	.tagline {
		color: #9aa4b2;
		margin-top: 0;
	}

	.placeholder {
		border: 1px dashed #3a4048;
		border-radius: 0.5rem;
		padding: 1.25rem;
		margin-top: 1.5rem;
	}

	.placeholder h2 {
		margin-top: 0;
		font-size: 1.1rem;
	}

	.placeholder em {
		color: #7d8894;
	}
</style>
