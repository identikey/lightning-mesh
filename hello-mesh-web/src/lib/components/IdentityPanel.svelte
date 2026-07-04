<script lang="ts">
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { fetchChallenge, submitIdentity } from '$lib/identity/api';
	import { generateKeyPair, publicKeyHex, signChallengeHex } from '$lib/identity/keys';
	import { loadIdentity, saveIdentity, type StoredIdentity } from '$lib/identity/storage';
	import CustodyNotice from './CustodyNotice.svelte';

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

<section class="flex flex-col gap-4 rounded-lg border border-slate-700 bg-slate-900/40 p-4">
	<CustodyNotice />

	{#if !dismissedAnonymous && !identity}
		<div class="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
			<p class="text-sm text-slate-300">The net is open — you don't need an identity to use it.</p>
			<button
				type="button"
				class="rounded-md border border-slate-600 px-3 py-1.5 text-sm text-slate-200 hover:bg-slate-800"
				onclick={() => (dismissedAnonymous = true)}
			>
				Just browse
			</button>
		</div>
	{/if}

	{#if identityLoaded && !identity}
		<div class="flex flex-col gap-2">
			<p class="text-sm text-slate-300">
				Create a browser-held identity to be named and reachable in the directory.
			</p>
			<button
				type="button"
				class="w-fit rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-indigo-500"
				onclick={createIdentity}
			>
				Create an identity
			</button>
		</div>
	{/if}

	{#if identity}
		<div class="flex flex-col gap-2">
			<p class="text-sm text-slate-300">
				Your identity: <code class="rounded bg-slate-800 px-1 py-0.5 text-xs">{pubkeyDisplay}</code>
			</p>

			{#if announceState === 'announcing'}
				<p class="text-sm text-slate-400">Announcing to this node…</p>
			{:else if announceState === 'success'}
				<p class="text-sm text-emerald-400">Announced — you're visible in this node's directory.</p>
			{:else if announceState === 'error'}
				<div class="flex flex-col gap-1">
					<p class="text-sm text-rose-400">Couldn't announce: {announceError}</p>
					<button
						type="button"
						class="w-fit rounded-md border border-slate-600 px-3 py-1.5 text-sm text-slate-200 hover:bg-slate-800"
						onclick={reannounce}
					>
						Retry
					</button>
				</div>
			{:else}
				<button
					type="button"
					class="w-fit rounded-md border border-slate-600 px-3 py-1.5 text-sm text-slate-200 hover:bg-slate-800"
					onclick={reannounce}
				>
					Announce to this node
				</button>
			{/if}
		</div>
	{/if}
</section>
