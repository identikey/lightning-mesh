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

`wiki.mesh`, `printer.mesh` — flat names for the **curated global tier**
(well-known + services): few, human-published, so flat-global first-writer-wins
is safe and gives the prettiest UX, truest to the founding vision ("services
broadcast on the local mesh and discoverable via `.mesh`"). **Device names are
the exception** — auto-derived and many, they are local-first and, when global,
*scoped* (see "Device names" below), so they never contend in the flat tier.
The name classes, resolved in this order:

| Class | Source | Answer | Status |
|---|---|---|---|
| **Well-known node-local** (`hello.mesh`, `id.mesh`) | compiled-in reserved list — **never in the CRDT, unclaimable** | this node's own client gateway IP (`10.42.x.1`) | first stone |
| **Services** (`wiki.mesh`) | `/services/{name}` CRDT lane, gossiped | the published `ip` (+ SRV port, TXT) | first stone |
| **Devices, stationary** (`nas.n7x3.mesh`) | explicit opt-in publish, **scoped** (a device-published service) | the device's IP | later (e21.3) |
| **Devices, auto** (`laptop.duke.mesh`) | DHCP lease lane, **identity-scoped**, gossiped + location-tracked | current lease IP | deferred (`e21.5`) |

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

## Device names: identity-gated, stationary opt-in first (revised 2026-07-04)

Devices are unlike services: **auto-derived from DHCP and many, with hostnames
the device picks, not the mesh** (`laptop`, `iphone`, `android-a3f2`). Two
constraints — surfaced in the e21.3 design pass — together prove that a *good*
auto device name (unique + human + roaming-stable + unforgeable) cannot exist
without identity:

- **Forgery / shadowing.** A bare `<host>.mesh` device name shares the flat form
  with services and well-knowns, so a device whose DHCP hostname is `wiki` (or
  `hello`, `id`) collides with the real `wiki.mesh` — shadowing it for clients on
  that node, or being shadowed when a real service appears. A reserved-list +
  precedence patch is whack-a-mole. The fix is structural: **devices never occupy
  the bare flat form.** (Owner-scoped globals like `nas.n7x3.mesh` are two labels
  and structurally cannot collide with a one-label service — the hole is only in
  the bare form.)
- **Roaming, even within one house.** Each node owns a **routed /24** and we
  deliberately **do not bridge client L2 across nodes** (broadcast containment) —
  there is no single L2 island spanning the mesh. So a device roaming between two
  mesh nodes *in the same home* re-DHCPs onto a new /24: new IP, new owning node.
  Node-local or node-scoped names therefore aren't stable — they're a **de-dup
  handle, not a name**. Stability requires binding to the device's/owner's
  identity and tracking its location mesh-wide (a gossiped lane updating the
  A-record as it roams), which is exactly what identity (`e21.5`) provides.

So device naming is staged on identity:

1. **Stationary opt-in (e21.3, pre-identity).** The only safe *and* useful
   pre-identity form: an operator **explicitly publishes** a non-roaming device
   (NAS, print server, always-on box) under a scope, e.g. `nas.<node>.mesh`.
   Safe — scoped, so no bare-form forgery; explicit, so no auto-flood. Stable —
   it doesn't roam, so node-scope holds. Mechanically it is a *device-published
   service*: the `/services` v2 lane already carries `host_mac` for exactly this,
   and `e21.9` handles staleness. Auto names for phones/laptops are **not** shipped
   here.
2. **Auto, stable, identity-scoped (deferred, `e21.5`).** `<host>.<owner>.mesh`
   bound to IdentiKey identity, gossiped and location-tracked so it survives
   roaming across the owner's nodes — the real portable device name, and the home
   of the auto-from-DHCP lease lane. Built only once identity exists, because
   before then it can be neither stable nor unforgeable, and such a name is worse
   than none.

**Locked shape:** bare `<host>.mesh` is reserved for the **curated global tier**
(well-known + services) and is never a device; every device name is **scoped**
`<host>.<scope>.mesh`, and the scope segment is always key-derived (node id now,
identity later), so the hierarchy is authority-free and Sybil-bounded — you
cannot publish under a scope whose key you do not hold.

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
- **Typed zones (`laptop.d.mesh`):** solves *type* collisions (device vs service)
  we don't have — devices and services don't share a tier (devices are
  local-first / owner-scoped, see above), so a type discriminator buys nothing at
  permanent UX cost. Rejected. (Distinct from *owner*-scoping, which we do adopt
  for global device names — that scopes by identity, not by type.)
- **A real registrar / hierarchy:** a registrar is an authority, rejected on
  ethos. Note the owner-scoping above *is* a hierarchy but **not** a registrar:
  its segments are key-derived (a node id / identity you already hold), so no
  authority issues them — hierarchy without a registrar. Name disputes in the
  flat tier resolve by HLC now and web-of-trust identity later — the same arc as
  node membership (`membership-enrollment.md`).
