// IndexedDB persistence for the rung-1 browser-held identity key, keyed by
// the stable `hello.mesh` origin so it survives reloads and roams with the
// user between nodes (docs/network-coordination/user-identity.md §4.2).
// Browser-only: callers must guard with `browser` from $app/environment.

const DB_NAME = 'hello-mesh-identity';
const DB_VERSION = 1;
const STORE_NAME = 'rung1-keys';
const RECORD_KEY = 'primary';

export interface StoredIdentity {
	publicKey: Uint8Array;
	secretKey: Uint8Array;
	createdAt: number;
}

function openDb(): Promise<IDBDatabase> {
	return new Promise((resolve, reject) => {
		const request = indexedDB.open(DB_NAME, DB_VERSION);
		request.onupgradeneeded = () => {
			if (!request.result.objectStoreNames.contains(STORE_NAME)) {
				request.result.createObjectStore(STORE_NAME);
			}
		};
		request.onsuccess = () => resolve(request.result);
		request.onerror = () => reject(request.error ?? new Error('failed to open IndexedDB'));
	});
}

export async function loadIdentity(): Promise<StoredIdentity | undefined> {
	const db = await openDb();
	try {
		return await new Promise((resolve, reject) => {
			const tx = db.transaction(STORE_NAME, 'readonly');
			const request = tx.objectStore(STORE_NAME).get(RECORD_KEY);
			request.onsuccess = () => resolve(request.result as StoredIdentity | undefined);
			request.onerror = () => reject(request.error ?? new Error('failed to read identity'));
		});
	} finally {
		db.close();
	}
}

export async function saveIdentity(identity: StoredIdentity): Promise<void> {
	const db = await openDb();
	try {
		await new Promise<void>((resolve, reject) => {
			const tx = db.transaction(STORE_NAME, 'readwrite');
			tx.objectStore(STORE_NAME).put(identity, RECORD_KEY);
			tx.oncomplete = () => resolve();
			tx.onerror = () => reject(tx.error ?? new Error('failed to save identity'));
		});
	} finally {
		db.close();
	}
}
