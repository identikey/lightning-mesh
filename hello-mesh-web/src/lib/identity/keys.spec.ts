import { describe, expect, it } from 'vitest';
import * as ed from '@noble/ed25519';
import { generateKeyPair, publicKeyHex, signChallengeHex } from './keys';
import { hexToBytes } from './hex';

describe('rung-1 keypair generation + signing', () => {
	it('generates a keypair and produces a signature verifiable against the public key', async () => {
		const { publicKey, secretKey } = await generateKeyPair();
		expect(publicKey.byteLength).toBe(32);
		expect(secretKey.byteLength).toBe(32);

		const challengeHex = 'deadbeef'.repeat(4); // arbitrary 16-byte challenge
		const sigHex = await signChallengeHex(secretKey, challengeHex);

		const isValid = await ed.verifyAsync(hexToBytes(sigHex), hexToBytes(challengeHex), publicKey);
		expect(isValid).toBe(true);
	});

	it('hex-encodes the public key', async () => {
		const { publicKey } = await generateKeyPair();
		const hex = publicKeyHex(publicKey);
		expect(hex).toMatch(/^[0-9a-f]{64}$/);
	});
});
