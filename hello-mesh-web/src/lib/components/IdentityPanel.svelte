<script lang="ts">
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { fetchChallenge, submitIdentity } from '$lib/identity/api';
	import { generateKeyPair, publicKeyHex, signChallengeHex } from '$lib/identity/keys';
	import { loadIdentity, saveIdentity, type StoredIdentity } from '$lib/identity/storage';
	import CustodyNotice from './CustodyNotice.svelte';
	import { Button } from '$lib/components/ui/button/index.js';

	type AnnounceState = 'idle' | 'announcing' | 'success' | 'error';

	let identity = $state<StoredIdentity | undefined>(undefined);
	let identityLoaded = $state(false);
	let announceState = $state<AnnounceState>('idle');
	let announceError = $state('');
	let dismissedAnonymous = $state(false);

	let pubkeyDisplay = $derived(identity ? publicKeyHex(identity.publicKey) : '');

	onMount(async () => {
		if (!browser) return;
		identity = await loadIdentity();
		identityLoaded = true;
	});

	async function announce(secretKey: Uint8Array, publicKey: Uint8Array) {
		announceState = 'announcing';
		announceError = '';
		try {
			const challenge = await fetchChallenge();
			const sig = await signChallengeHex(secretKey, challenge);
			await submitIdentity({ pubkey: publicKeyHex(publicKey), sig, challenge });
			announceState = 'success';
		} catch (err) {
			announceState = 'error';
			announceError = err instanceof Error ? err.message : String(err);
		}
	}

	async function createIdentity() {
		const keyPair = await generateKeyPair();
		const stored: StoredIdentity = {
			publicKey: keyPair.publicKey,
			secretKey: keyPair.secretKey,
			createdAt: Date.now()
		};
		await saveIdentity(stored);
		identity = stored;
		await announce(stored.secretKey, stored.publicKey);
	}

	async function reannounce() {
		if (!identity) return;
		await announce(identity.secretKey, identity.publicKey);
	}
</script>

<section
	class="flex flex-col gap-4 rounded-lg border border-border bg-card p-4 text-card-foreground"
>
	<CustodyNotice />

	{#if !dismissedAnonymous && !identity}
		<div class="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
			<p class="text-sm text-muted-foreground">
				The net is open — you don't need an identity to use it.
			</p>
			<Button variant="outline" size="sm" onclick={() => (dismissedAnonymous = true)}>
				Just browse
			</Button>
		</div>
	{/if}

	{#if identityLoaded && !identity}
		<div class="flex flex-col gap-2">
			<p class="text-sm text-muted-foreground">
				Create a browser-held identity to be named and reachable in the directory.
			</p>
			<Button class="w-fit" size="sm" onclick={createIdentity}>Create an identity</Button>
		</div>
	{/if}

	{#if identity}
		<div class="flex flex-col gap-2">
			<p class="text-sm">
				Your identity: <code class="rounded bg-muted px-1 py-0.5 text-xs">{pubkeyDisplay}</code>
			</p>

			{#if announceState === 'announcing'}
				<p class="text-sm text-muted-foreground">Announcing to this node…</p>
			{:else if announceState === 'success'}
				<p class="text-sm text-success">Announced — you're visible in this node's directory.</p>
			{:else if announceState === 'error'}
				<div class="flex flex-col gap-1">
					<p class="text-sm text-destructive">Couldn't announce: {announceError}</p>
					<Button variant="outline" size="sm" class="w-fit" onclick={reannounce}>Retry</Button>
				</div>
			{:else}
				<Button variant="outline" size="sm" class="w-fit" onclick={reannounce}>
					Announce to this node
				</Button>
			{/if}
		</div>
	{/if}
</section>
