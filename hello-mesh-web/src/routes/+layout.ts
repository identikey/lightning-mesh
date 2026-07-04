// Prerender the whole app to a static bundle (adapter-static, SSG). Pages that
// need live mesh state (directory, identity) fetch /api/* client-side at
// runtime, so the prerendered shell is all we need at build time.
export const prerender = true;
