# Lightning Mesh — DWeb Talk Source Material

**Status:** Talk source / public-facing narrative | **Audience:** DWeb (a project of
the Internet Archive) | **Date started:** 2026-07-02

This is the source material for the Lightning Mesh talk, written for a
decentralization-native audience. It is deliberately public-facing: no internal
codenames, no bead references in the narrative itself. Over time this document is
expected to become the basis for the public website documentation.

> **Naming:** the project's public name is **Lightning Mesh**. The repository,
> crates, and binaries retain the `mjolnir-` prefix (`mjolnir-meshd`, etc.), and
> the overlay interface is still `mjolnir0`. Package names are plumbing; the name
> people meet is Lightning Mesh.

---

## The arc of the talk

1. We are in the lineage of Cerf & Kahn — this is the Internet's own architecture,
   recapitulated at mesh scale.
2. The mesh industry took a twenty-year detour through Layer-2 emulation. Here's
   why that detour was tempting, and here's the wall at the end of it.
3. Every large mesh that survived converged on our starting point. We're not
   guessing; we're starting where they arrived.
4. What this buys philosophically: structural sovereignty, hardware mortality
   without network mortality, discovery as a product instead of an accident.
5. The network becomes a projection of a set of keys — which is why we don't
   need IPv6, and which changes who gets to publish services.
6. Where this goes next: user identity, service mesh, and a network you own the
   way you own your keys.

---

## 1. The lineage: Cerf & Kahn, again

In 1974, Vint Cerf and Bob Kahn faced a problem that looks exactly like ours: a
collection of networks built on radically different link technologies — ARPANET's
leased lines, packet radio vans driving around the Bay Area, satellite links over
the Atlantic — that needed to behave as one network. Their answer was the
*catenet*: don't unify the links, unify the layer above them. IP became the narrow
waist of the hourglass — the one thing everything agrees on — and every link
technology below it became interchangeable plumbing.

That answer is the only network design that has ever scaled across five orders of
magnitude. It is the reason the Internet outlived every physical technology it
launched on.

Lightning Mesh is that architecture, recapitulated at mesh scale: **heterogeneous
link islands stitched together by L3 routing.** An 802.11s radio island, an
ethernet run, a QUIC tunnel across the open internet — each is just a link with a
routing metric. The network is the layer above: identity, routing, and shared
state. The radio is plumbing.

For this audience the point lands sharply: this isn't a novel bet on an exotic
architecture. It's a refusal to bet against the one architecture with a
fifty-year track record.

## 2. The twenty-year detour

Almost every mesh system of the last two decades made the opposite bet: **make
many radios look like one Ethernet.** Glue every node into a single flat Layer-2
cloud, one broadcast domain, so that every device appears to be on the same wire.

The appeal is obvious, and it's worth being honest about: on a flat L2, everything
"just works" *because the physical topology is disguised*. mDNS discovers your
printer, AirPlay finds your speakers, DHCP hands out addresses, devices roam
without renumbering — all for free, because the protocols designed for one wire
can't tell they're not on one wire.

But the loan comes due. Flat L2 has a structural scaling law: **global state and
broadcast traffic grow with the number of client devices, not the number of
routers.** Every phone's ARP chatter floods every link. Every client MAC is
mesh-wide state. Double the network and you double the background noise for
everyone — until broadcast load, or a single forwarding loop turned storm, becomes
the ceiling. It's fine for a house. It dies at a venue.

## 3. Everyone who survived converged here

This is the empirical heart of the talk, and it needs no speculation:

- **Freifunk** — the largest federation of community meshes in the world, built on
  batman-adv's flat L2 — hit the broadcast wall in the hundreds of nodes. Their
  fix, in their own documentation, was to segment into smaller L2 domains
  connected by L3 tunnels. That is our architecture, arrived at under duress.
- **Guifi.net** (tens of thousands of nodes) and **NYC Mesh** run BGP between
  heterogeneous zones: link islands stitched by L3 routing.
- **CeroWrt**, the bufferbloat community's canonical OpenWrt mesh, paired Babel
  routing with per-router subnets — the same split we ship.

Every large mesh that survived converged on this shape *after* hitting the flat-L2
wall. Lightning Mesh starts where they arrived. The twenty-year detour is over;
we just declined to take it.

And the trade is honest: within an island, clients still get flat-L2 semantics —
local mDNS, cheap local traffic, seamless local roaming. L3 stitches only where
L2 physically can't reach. Islands stay small and internally simple; routing does
the scaling. Client churn — a conference attendee's phone associating, roaming,
leaving — is a local event, never a mesh event.

## 4. Contrast: the big-tech mesh

eero is Amazon. Google Wifi is Google. These systems are flat L2 plus a
proprietary controller plus a mandatory cloud account. Three properties follow:

- **The controller is an authority.** Your network has a boss, and the boss phones
  home.
- **The network is the vendor's silicon.** An eero mesh *is* eeros talking to
  eeros over eero's radios. Pull out the radio and there is no network. When the
  vendor sunsets the product line — and Amazon has sunset product lines — the
  network dies with it.
- **The scaling ceiling is a house.** Flat L2 was chosen because the design target
  was a living room, and it shows.

Lightning Mesh inverts all three, and the third point deserves its own section.

## 5. The network outlives the hardware

Because the invariant is the L3 overlay and the radio is "whatever each node can
do," the mesh can outlive any radio generation. 802.11s today; 60GHz
point-to-point links, ethernet runs, LTE backhaul, QUIC over any internet egress
tomorrow — each is just another link with a metric. This is what *future-proof*
means when it isn't a marketing adjective: the abstraction that defines the
network sits above everything that ages.

We have already lived this once. An entire hardware segment (closed-driver
MikroTik boards running an AP/STA topology) was retired from the fleet — and the
mesh architecture didn't notice, because the L3 layer never cared what the links
were. That retirement is the thesis working in production.

It also makes the design genuinely interoperable: a $40 OpenWrt box, a Raspberry
Pi with a USB dongle, a laptop, a VM in a datacenter — all join the same mesh as
equals, because they all speak the layer that matters. Meshes built independently
can merge by linking at a single node and letting routing stitch the address
spaces together.

## 6. Sovereignty is structural

Each node owns its own routed /24. That single design decision means:

**No node is in a position to be an authority over another node's segment.**

Not as policy — as topology. Blast-radius containment isn't a firewall rule
someone maintains; it's the shape of the network. A misbehaving island can't
broadcast-storm its neighbors. A compromised node's L2 tricks — ARP spoofing,
rogue DHCP — stop at its own segment, because there is no shared segment to
poison. In the flat-L2 world, containment is something you configure and
misconfigure. Here, it's something you'd have to *build infrastructure to
violate*.

This is what "symmetric and non-authoritative" means when it's load-bearing:
nobody is in charge, and — more importantly — **nobody is in a position to become
in charge**, even if they wanted to be.

And it's what makes the system genuinely plug-and-play: because no node is an
authority, **there is no authority to configure**. Nobody designates the
controller, elects the leader, or blesses the DHCP server. Plug in a router: it
derives its own address from its own identity, claims its own /24 through the
shared state layer, and starts routing. Unplug it: the mesh adapts. "Zero
configuration" isn't a convenience feature layered on top — it's what falls out
when there is no authority whose existence would need configuring.

## 7. Discovery as a product, not an accident (the sleeper)

Flat-L2 systems get discovery "for free" from mDNS — which means discovery is an
*emergent property of physical adjacency*. It evaporates the moment topology gets
interesting: multi-hop, cross-site, or partitioned, and your printer vanishes.

Being forced off flat L2 forced us to build discovery as a first-class system: a
gossip-replicated, conflict-free (CRDT) address book, synchronized mesh-wide with
no coordinator. That felt like paying a cost the flat-L2 folks dodged. It is
actually the better asset, and it's the sleeper of the whole design:

- It works **multi-hop and cross-site** — over the open internet, not just the
  local wire.
- It **survives partitions** — split the mesh, both halves keep working, rejoin
  and the state merges. No leader election, no quorum, just merge.
- It's bound to **cryptographic identity**, not to being on the same wire.

Routing is a commodity — Babel is excellent and replaceable. A
partition-tolerant, identity-keyed, mesh-wide directory is not a commodity. As it
grows to carry names, services, and membership, it becomes the thing the network
*is*. (This section of the talk should grow as the address book and service
layers ship over the next few sprints.)

## 8. The network is a projection of a set of keys

Every node's mesh address is *derived from its cryptographic identity* — a hash
of its public key. That sounds like an implementation detail. It's the deepest
move in the stack:

**The network becomes a projection of a set of keys.** The physical substrate —
which island, which radio, which continent — becomes routing detail. As it should
be.

Our kin here are cjdns and Yggdrasil, which pioneered crypto-derived addressing
and deserve the credit for it. They dissolved the island concept entirely and
route through the overlay alone, paying for it in routing stretch and in losing
local L2 semantics. Lightning Mesh keeps their deepest idea — addresses are
identities — and marries it to the catenet architecture: real link islands,
locally fast, stitched by ordinary routing.

What follows from identity-addressing:

- **Ownership by key, not by physical access.** A node's owner is whoever holds
  the key — authorized cryptographically, not by being in the room. Manage your
  node from anywhere; the mesh manages itself over itself.
- Every connection is end-to-end encrypted between identities. Forwarders — a
  neighbor's router, a café's uplink — move ciphertext they cannot read. You get
  a trusted network on an untrusted substrate.
- Next: this extends from nodes to **people**. User identity by key
  (IdentiKey), so that "who can see this service" and "who can join this
  network" are answered by keys and webs of trust, not by MAC filters and
  shared passwords.

### The IPv6 question (and why the answer is keys)

There's a fair question this audience will ask: *why is a next-generation mesh
running on IPv4?* We asked it ourselves, seriously — sat down and designed the
IPv6 migration, then red-teamed it. The answer surprised us.

IPv6 made three promises: address space so vast that collisions stop being a
coordination problem, a stable end-to-end address for every device and
service, and the end of NAT's middlebox authority. Look at that list again —
**the identity layer already keeps all three.** A node's public key is a
collision-free identifier from a space of 2²⁵⁶ that no registry allocates and
no one can squat. A service reached by key is reachable from anywhere, through
any NAT, because reachability is negotiated by the identity layer instead of
begged from the addressing plan. And a key can't be reassigned by whoever runs
the middlebox, because it isn't a number someone agreed to route — it *is* the
identity.

So the requirement that seemed to demand IPv6 — a stable address for
everything — had simply been filed under the wrong layer. Once service
identity lives in keys, IP stops being the network's identity layer at all
and demotes to what it's genuinely good at: access plumbing. A familiar,
debuggable, universally supported way for an ordinary phone to hand packets
to the nearest node. And IPv4's famous scarcity stops mattering when nothing
scarce is being asked of it.

This is the narrow waist moving up, one layer at a time: Cerf and Kahn
unified heterogeneous *links* under IP; we unify heterogeneous *networks* —
radio islands, NATed households, cloud VMs — under cryptographic identity,
and IP itself drops below the waist to join the plumbing. What IPv6 tried to
achieve by widening the number, the identity layer achieves by replacing the
number with a key. (And nothing is foreclosed: if a single physical island
ever truly outgrows IPv4, v6 can still be adopted *as plumbing* — nothing
above the waist would notice.)

## 9. Publishing a service without paying rent

Here is where the architecture becomes political, in a way this audience will
recognize.

Today, publishing a service — a wiki, a game server, a community archive — means
one of two things: become a network wizard (port forwarding, dynamic DNS, TLS
certificates, NAT traversal), or rent the capability back from the people who
enclosed it (a cloud provider, a tunnel service, Tailscale's coordination
server). The knowledge barrier and the rent are the same wall viewed from
different sides — and rent-seeking on *reachability* is one of the quieter forms
of extractive capitalism. The infrastructure to reach each other exists; it's
been enclosed, and access is sold back by subscription.

That wall matters most for exactly the services whose point is **being of
service** rather than extracting money: the neighborhood file share, the mutual
aid coordination page, the community wiki that will never have a business model
and shouldn't need one.

On Lightning Mesh, a Raspberry Pi joins the mesh and `wiki.mesh` resolves from
every node — locally when there's no internet, globally over the encrypted
overlay when there is. No port forwarding, no rented tunnel, no coordination
server that isn't yours. The mesh is a **decentralized application platform**
where reachability is a property of joining, not a product you subscribe to.

DWeb runs on the irony that a movement about decentralization convenes on
centralized infrastructure. A venue mesh where organizers plug in ten routers
and attendees' services are discoverable by name — offline-capable,
nobody's cloud in the loop — is the demo that dissolves the irony.

## 10. Honest ceilings and future work

Credibility with this audience comes from naming the limits plainly:

- **Routing trust.** Babel believes its neighbors; today one malicious node
  could announce routes it doesn't own. We're unusually well-positioned to fix
  this because we already have cryptographic node identity and a replicated
  state layer: route-origin validation — "only the identity that hashes to that
  block may announce it" — is a natural extension, planned alongside the
  IdentiKey and service-mesh work. Most meshes can't build this because they
  have no identity layer at all.
- **Addressing.** Per-node /24s from a private /16 is a finite pool — 256
  sites per mesh at defaults, expandable to tens of thousands. We evaluated
  an IPv6 migration and declined it deliberately (see section 8: identity
  lives in keys, so IP is plumbing); collisions in derived addresses are
  resolved through the same CRDT machinery that arbitrates subnet claims.
  If a single physical island ever outgrows IPv4, v6 remains available as
  plumbing without touching anything above it.
- **Directory scaling.** Full replication of the address book is comfortable to
  thousands of routers. Beyond that: sharding or DHT-style lookup — a genuinely
  fun future problem, and one the flat-L2 systems never get to have because
  they die at dozens.
- **Cross-island roaming.** Walking between islands means re-addressing; names,
  not sticky IPs, are the continuity layer (the Internet's own lesson). Fast
  in-island handoff (802.11r) is standard; whether better key management could
  make cross-island transitions smoother than the protocol's reputation
  suggests is an open investigation.

## Talk beats / one-liners (for slides)

- "Heterogeneous link islands stitched together by L3 routing — you've heard
  this design before. It's called the Internet."
- "The mesh industry took a twenty-year detour through Layer-2 emulation.
  Everyone who survived it converged on the catenet. We just started there."
- "Flat L2 buys convenience by taking a loan against physics. The loan is
  called in at a few dozen nodes."
- "eero is Amazon. Your network has a boss, and the boss phones home."
- "The radio is mortal. The network doesn't have to be."
- "Sovereignty is structural: no node is in a position to be an authority over
  another node's segment. Not as policy — as topology."
- "No authority means no authority to configure. That's why it's plug-and-play."
- "Discovery that comes free from the wire evaporates with the wire. Ours is a
  product, not an accident."
- "The network is a projection of a set of keys. Everything physical is routing
  detail — as it should be."
- "We don't need IPv6. We have keys."
- "IPv6 widened the number. We replaced the number with a key."
- "Everything IPv6 promised — no scarcity, end-to-end addresses, no NAT
  middlemen — the identity layer already delivers. IP is just plumbing now."
- "Ownership by key, not by physical access."
- "Reachability shouldn't be a subscription."
- "This is the shape you never have to migrate away from."

## Related documents

- [Philosophical outcomes](../vision/philosophical-outcomes.md) — the durable
  statement of these ideas, independent of the talk.
- [Why decentralized mesh networking](../vision/why-decentralized-mesh.md) —
  motivation and comparisons.
- [Prior art](../network-coordination/prior-art.md) — the auditable
  state-of-the-art comparison (CeroWrt, AHCP, OpenWISP).
- [Network architecture](../network-coordination/network-architecture.md) — what
  is actually shipped and field-validated.
