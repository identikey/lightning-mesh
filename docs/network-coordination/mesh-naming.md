# `.mesh` Naming — the service-mesh name layer

**Status 2026-07-03:** DESIGN (bead `e21`, item 2). Decided in the e21 architecture
pass: daemon-embedded DNS responder behind dnsmasq, flat namespace, first stone =
services + well-known names. Nothing below is shipped yet; the CRDT schemas
(`crdt/dns.rs`, `crdt/service.rs`) and gossip wire format already carry these lanes
but the apply loop does not.

**Read first:** `gossip-and-crdt.md` (how facts converge), `network-architecture.md`
(routed /24s + babel + `mjolnir0`). Identity interactions: `user-identity.md` (rp9).

---

## TL;DR

Every name in the system ends in `.mesh`. A phone asks its node "who is
`wiki.mesh`?" and gets an answer that works whether the service is one radio hop
away or across the internet — because the answer is a mesh IP and **babel + the
iroh overlay already make every mesh IP reachable**. DNS resolves; the L3 overlay
routes; neither layer knows about the other. The name table is a CRDT lane, so
there is no DNS server to capture, no zone file authority, no registrar — the
namespace is a shared fact the nodes gossip, exactly like subnet claims.

---

## Resolution plumbing

Each node's `mjolnir-meshd` embeds a small authoritative DNS responder for the
`.mesh` zone, listening on `127.0.0.1:5335` (5353 is mDNS — do not squat it).
Stock dnsmasq keeps serving clients; we add one UCI line:

```
uci add_list dhcp.@dnsmasq[0].server='/mesh/127.0.0.1#5335'
```

dnsmasq forwards every `*.mesh` query to the daemon and handles everything else
normally. The same UCI reconcile also sets **DHCP option 114** (RFC 8910
captive-portal API, used non-blockingly per rp9) to `http://hello.mesh` so
client OSes surface the front-desk affordance — the contract
`hello-mesh-service.md` §5 asks of this track. This preserves the existing discipline (**the daemon never edits dnsmasq
files and never SIGHUPs it** — UCI + init.d restart only, same as
`reconcile_client_uci`), avoids file-render/reload races, and gives us SRV/TXT
records, which a hosts file cannot express. The responder is a **pure projection
of the in-memory CRDT** — a gossip merge is visible on the next query, no reload
step.

The same reconcile **must also whitelist `.mesh` from dnsmasq's rebind
protection** (`uci add_list dhcp.@dnsmasq[0].rebind_domain='/mesh/'`). OpenWrt
ships `stop-dns-rebind` on by default, which drops any upstream answer carrying
an RFC1918 address — and *every* `.mesh` answer is RFC1918, so without the
whitelist dnsmasq forwards the query, gets the right `10.42.x.1` back, and
silently discards it (`possible DNS-rebind attack detected`). This was found the
hard way on the first fleet rollout (bead `e21.8`): the forward line, DoH canary,
and option 114 were all correct and `hello.mesh` still resolved to an empty
answer. No unit or multi-node test caught it — it only manifests against real
dnsmasq. (Aside: dnsmasq's own cache at `:53` can briefly serve a just-unpublished
name as empty NODATA up to the 30s TTL even after the responder returns NXDOMAIN
— normal cache behavior, not a bug.)

Answer discipline:

- `A` for every name class below; **TTL 30s** everywhere (the substrate is
  eventually consistent and `hello.mesh` must re-resolve after a roam).
- Non-`A` queries on an existing name → **NOERROR with empty answer** (NODATA),
  never NXDOMAIN — an NXDOMAIN on AAAA would poison the A lookup in some stubs.
- Unknown `.mesh` name → NXDOMAIN.
- `SRV`/`TXT` served for services (`_<proto>` labels per the ServiceEntry record).

## Namespace: flat, one arbitration rule

`wiki.mesh`, `printer.mesh`, `laptop.mesh` — flat names, one namespace, no typed
zones. Prettiest UX and truest to the founding vision ("services broadcast on the
local mesh and discoverable via `.mesh`"). Three name classes share it, resolved
in this order:

| Class | Source | Answer | Status |
|---|---|---|---|
| **Well-known node-local** (`hello.mesh`, `id.mesh`) | compiled-in reserved list — **never in the CRDT, unclaimable** | this node's own client gateway IP (`10.42.x.1`) | first stone |
| **Services** (`wiki.mesh`) | `/services/{name}` CRDT lane, gossiped | the published `ip` (+ SRV port, TXT) | first stone |
| **Devices** (`laptop.mesh`) | `/dns/{hostname}` derived from `/devices/{mac}` leases | the device's lease IP | later (needs lease lane) |

Well-known names are **anycast by convention**: every node answers `hello.mesh`
with itself. That is deliberate and load-bearing for rp9 — `hello.mesh` is the
single stable browser origin (same-origin policy locks rung-1 browser keys to one
name), and "the front desk is always the node you're standing next to" is the
product behavior. The reserved list is a compiled constant; the merge layer
rejects CRDT claims on reserved names outright.

Everything else is **owner-bound** (decided in the PRD pass, 2026-07-03 — the
TOFU/addr-book pattern applied to names): the first claim binds a name to its
`owner_node_id`; updates are accepted only from that key (newer HLC); a
*different* key claiming the name is a `Conflict`, resolved first-writer-wins on
the **first-claim** HLC — the original owner keeps the name, deterministic
node-id tiebreak for true partition races. The loser's daemon stops answering
immediately, `status` shows the lost name with the winner's node-id, and a
re-publish attempt errors naming the winner. Provenance is visible everywhere:
`status` and the hello.mesh directory show the owner's key next to every name.
No registrar, no vote. Squatting is still possible (first claim is cheap) — the
durable answer is web-of-trust identity arbitration, punted to its own bead
with the discovered needs recorded. Full requirements:
`../prd-mesh-naming-first-stone.md`.

## Local *and* internet access: free, by construction

A service record's A-answer is a mesh IP (typically the publishing node's client
subnet or its `10.254.x` backhaul address). Reachability is not DNS's problem:

- **Same island:** babel routes it over the 802.11s L2 backhaul.
- **Cross-site / internet:** babel routes it into `mjolnir0`, the iroh QUIC
  overlay carries it, encrypted end-to-end between daemons.

The resolver never branches on "local vs remote" — that is the whole thesis (the
L3 overlay is the product). The record additionally carries the **publishing
node's iroh node-id** for provenance, future identity binding (rp9 web-of-trust),
and future off-mesh access by dial-by-node-id; it is *not* used for resolution.

## Record changes: `ServiceEntry` v2

The current `ServiceEntry` (crdt/service.rs) is device-lease-coupled (`host_mac`,
expiry tied to lease). Services in the first stone are **node-hosted** (the wiki
on the router, the front-desk directory), so the record needs:

- `owner_node_id` — the publishing node (provenance + merge ownership + FWW arm);
- `published_at: Hlc` — conflict arbitration, same as `SubnetClaim.claimed_at`;
- `host_mac` becomes `Option` — populated only for device-published services once
  the lease lane exists.

Merge rule (`merge.rs`, new `merge_service`): same-owner + newer HLC → `Updated`
(owners freely re-publish; the anti-entropy tick re-announces, same pattern as the
0yb address book); different owner → `Conflict`, FWW on HLC. Old nodes decode-skip
unknown gossip variants, so this is mixed-fleet safe like `PeerAddrUpdate` was.

Expiry/TTL of stale services is **deferred to the same mechanism as address-book
expiry (bead `e21.9`)** — one staleness design for all self-announced lanes, not
per-lane ad-hockery.

## Publishing

`mjolnir-meshd` gains a publish surface (same control path as `status`):

```
meshd publish wiki --port 8080            # → /services/wiki, ip = node gateway, SRV port 8080
meshd unpublish wiki                      # tombstone
```

The hello.mesh front desk (rp9) later renders the live `/services/` map as the
neighbor/service directory — the directory is a *projection* of this lane plus the
0yb address book, not a separate registry.

## Sharp edges (documented, not solved here)

- **DoH bypass:** a browser with DNS-over-HTTPS enabled never asks the node
  resolver, so `.mesh` fails. Mitigation shipped with the first stone: dnsmasq
  NXDOMAINs the `use-application-dns.net` canary
  (`server='/use-application-dns.net/'`), which disables Firefox DoH on the
  network. Chrome's same-provider auto-upgrade follows the OS resolver and is
  unaffected. Hard-coded DoH (user forced it) stays broken — a support-note
  reality, not a design flaw.
- **`.mesh` is not an IANA special-use domain** (unlike `.local`, `home.arpa`,
  `.internal`). Leaked queries go to the roots and NXDOMAIN (fine); a future ICANN
  delegation of `.mesh` as a gTLD would shadow-collide (accepted risk; revisit if
  it ever leaves "applied-for" status). DNSSEC-validating stubs that insist on
  proving the root's NXDOMAIN will reject our answers — same boat as every
  private-TLD deployment; document, don't fight.
- **Not a secure context:** `http://hello.mesh` gets no WebCrypto etc. Already
  handled by rp9's custody-rung design (soft rung-1 in-page keys; hard custody
  requires extension/app). The naming layer just needs to not pretend otherwise.
- **mDNS interop is a later stone:** reflecting `/services/` into per-/24 mDNS so
  AirPlay/Bonjour discover mesh services natively. Valuable, separable, deferred.

## Why not …

- **Hosts-file rendering (`--hostsdir`):** A-records only, no SRV/TXT, and it
  re-opens daemon-managed dnsmasq state. Rejected.
- **Typed zones (`laptop.d.mesh`):** solves device/service collisions we don't
  have yet, at permanent UX cost. Revisit with the lease lane if collisions bite.
- **A real registrar / hierarchy:** an authority, rejected on ethos. Name
  disputes resolve by HLC now and by web-of-trust identity later — the same arc
  as node membership (`membership-enrollment.md`).
