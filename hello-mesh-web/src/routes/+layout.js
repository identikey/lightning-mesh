// Prerender everything: this is a pure static bundle, no server-side
// rendering at request time. `ssr = false` keeps the client bundle the
// single source of truth (directory + identity are client-fetched from
// /api/* once mjolnir-hello serves them), avoiding any hydration mismatch
// between a prerendered snapshot and live mesh state.
export const prerender = true;
export const ssr = false;
