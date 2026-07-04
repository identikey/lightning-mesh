// Rung-1 "soft custody" identity keypair: pure-JS Ed25519 (@noble/ed25519 v3),
// generated and held in the browser. Plain HTTP `hello.mesh` is an insecure
// context (no WebCrypto), so this is the honest fallback described in
// docs/network-coordination/user-identity.md §3/§4.4 — extractable by the
// serving node, never equivalent to app/hardware (hard) custody.
//
// v3 is ESM; hashing is async. We use the async functions (getPublicKeyAsync/
// signAsync) rather than setting the sync sha512 hook.
import * as ed from '@noble/ed25519';
import { bytesToHex, hexToBytes } from './hex';

export interface KeyPair {
	publicKey: Uint8Array;
	secretKey: Uint8Array;
}

export async function generateKeyPair(): Promise<KeyPair> {
	const { secretKey, publicKey } = await ed.keygenAsync();
	return { secretKey, publicKey };
}

/** Sign a hex-encoded challenge, returning the hex-encoded signature. */
export async function signChallengeHex(
	secretKey: Uint8Array,
	challengeHex: string
): Promise<string> {
	const message = hexToBytes(challengeHex);
	const signature = await ed.signAsync(message, secretKey);
	return bytesToHex(signature);
}

export function publicKeyHex(publicKey: Uint8Array): string {
	return bytesToHex(publicKey);
}
