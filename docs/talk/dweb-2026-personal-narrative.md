# The Long Way Around — DWeb Talk (personal narrative)

**Status:** Talk source / personal-story arc | **Audience:** DWeb (a project of the
Internet Archive) | **Date started:** 2026-07-04

This is the *story* cut of the Lightning Mesh talk — the road I actually walked to
get here, and the reasons that keep me from putting it down. Its sibling,
[the technical-arc source](dweb-2026-technical-arc.md), makes the same points as
systems design (Cerf & Kahn, the flat-L2 wall, projection-of-keys). This one is
the human throughline: how a key-based identity project turned into a network, why
the network had to be un-ownable, and what emerged when I took the authority away.
We'll braid the two together later; keeping them apart makes each easier to think
about.

> **Naming:** the public name is **Lightning Mesh**; crates and binaries keep the
> `mjolnir-` prefix and the overlay interface is `mjolnir0`. **IdentiKey** is the
> key-based identity/auth/authz layer. **Mjolnir** is the microVM/process-calculus
> compute fabric. All three are the same idea seen from three angles.

Each beat below is told twice: **the story** (what I felt and believed), then
**how we faced it** (what we actually had to build for the belief to hold). The
gap between those two is the whole talk.

---

## 0. The thesis, in one breath

I set out to build **key-based identity** — censorship-resistant, permissionless —
and discovered I couldn't build it honestly on top of an internet that assumes an
authority at every layer. So I had to build the ground first: a network with **no
authority to capture**. The identity project became a network project became a
compute project, and the through-line the whole way down was a single refusal —
*nothing here should be ownable.* Locked open. Our freedom depends on it.

---

## 1. IdentiKey, and the OAuth server that answered the wrong question

**The story.** I was designing IdentiKey: identity, authentication, authorization,
all rooted in keys instead of accounts. Censorship-resistant, permissionless — the
right to *be someone* online without asking a gatekeeper. To solve the immediate
problem in front of me, I stood up an OAuth server. It worked. And then it wasn't
obvious what *else* was really needed — which is exactly the trap. An OAuth server
answers "how do I log in," but it does it by *being an authority*: an issuer, a
place that says yes or no, a thing you phone home to. I didn't want to be a crypto
wallet either — that's the other ditch, where identity collapses into custody of a
token and a market. I wanted something quieter and older: **you are your key, and
the network just verifies signatures.** The hard part was that so many centralizing
assumptions are baked into the modern internet that they're hard to even *see*,
let alone tease apart. You reach for "just verify identity" and your hand closes
around a certificate authority, a DNS registrar, a relying-party server, a
platform vendor. The plumbing is centralized all the way down.

**How we faced it.** The governing principle we landed on is stark: **the protocol
verifies only signatures.** No component of the mesh ever verifies a *server*,
consults an issuer, or phones home; every trust decision is a stateless, offline
check of a signature chain
([user-identity.md §1](../network-coordination/user-identity.md)). A user identity
is an Ed25519 keypair; a device holds its own key; the identity key signs an
*attestation* over the device key, and anyone can verify that link with no issuer
contact. We evaluated WebAuthn/passkeys and **rejected** them for precisely the
centralizing reason — a passkey is bound to a DNS-domain Relying-Party ID and
platform-vendor infrastructure. And the OAuth instinct wasn't wrong, it was
*misplaced*: it survives as **rung 3** on a custody spectrum — a custodian you
*consciously choose*, which itself runs as an ordinary mesh service, never a
protocol dependency. Its outage degrades *its* users' signing, not the mesh. The
"am I a wallet?" fear is answered by the same spectrum: custody is a gradient the
user picks, from *no key at all* (anonymous is a right, not a fallback) up to a
hardware-held key. Services see one uniform thing — a key and valid signatures —
regardless of which rung you stand on. Identity stopped being a server and became
a *shape of verification*.

## 2. Mjolnir: computing as processes, not functions

**The story.** In parallel I was building Mjolnir: fast micro-VMs with real
inter-process communication — an attempt to instantiate **process calculus**
(the π-calculus) in running silicon. Almost all the software we write lives inside
the **λ-calculus** model: functions in, values out, a single machine's memory as
the world. That model is magnificent and it is also a cage — it assumes one
address space, one clock, one owner of state. The π-calculus starts somewhere else:
the primitive isn't the function, it's the **communicating process** and the
**channel** between them. Concurrency, mobility, and independent parties are native,
not bolted on. It has a firm theoretical grounding and almost no real-life
instantiation, and I wanted to change that — to have software we could model the
way networks actually are: many parties, no shared memory, messages over channels,
no central scheduler.

**How we faced it.** Mjolnir and the mesh turned out to be the same bet at two
scales. A Mjolnir VM already *is* a process with a channel: an iroh endpoint
(an Ed25519 identity) and a QUIC connection are a π-calculus name and the channel
it names. A VM boots in ~125ms, gets an identity, and joins the mesh; it's
discoverable by hostname because the host writes it into the shared state, and any
agent or person can reach it over an encrypted channel with **no out-of-band ticket
sharing** ([mjolnir-integration.md](../vision/mjolnir-integration.md)). The mesh's
own coordination layer is π-shaped too: independent nodes, no shared memory,
communicating by message-passing, converging with no central scheduler (§7 below).
So "graduate from λ to π" isn't a slogan bolted onto the network — the network is
what π-calculus *looks like* when you build it out of routers and VMs instead of
chalkboard symbols. The compute fabric (Mjolnir) and the network fabric (the mesh)
share one transport, iroh, precisely because they are the same abstraction.

## 3. DWeb, and the two ideas I was missing

**The story.** DWeb invited me to their symposium, and it changed the trajectory.
I walked in with a pile of half-built pieces and walked out with a rich set of
peers to bounce ideas off — and two ideas I'd been missing. First: **conflict-free
methods** for maintaining data when no one is in charge — how independent replicas
can change the same state, with no authority to adjudicate, and still converge.
Second: **Iroh** — a genuinely brilliant sidestep around the corporate internet's
corner-painting. Instead of begging the addressing plan for reachability, Iroh just
makes a **key-based Layer 3**: your public key *is* your address, connections are
end-to-end encrypted between identities, and it carries HTTP/3, routing, NAT
traversal — the whole thing — without a coordinator you rent. It was the missing
floor under IdentiKey. If identity is a key, and the *network address* is also a
key, then identity and reachability stop being two problems.

**How we faced it.** Both ideas are load-bearing now. Iroh is the transport for the
entire stack — routers, VMs, gossip all speak it, and a node's overlay address is
literally derived from its key: `10.254.<blake3(node_id)>/16`
([decentralized-systems-design.md §4](../vision/decentralized-systems-design.md)).
The conflict-free idea became the mesh's spine: **strong eventual consistency
without consensus** — CRDTs merged over gossip, with a Hybrid Logical Clock giving
a total order that every node computes identically, so there's no "whose version
wins" that needs a human or a leader (§4, §7 below). The symposium's real gift
wasn't a library, it was permission to stop trying to preserve the *centralized*
guarantees and instead engineer the *decentralized* ones to be exactly as strong
as they can be with no throat to choke.

## 4. Why a mesh at all? (the accident that became the point)

**The story.** So why a *mesh* — wasn't I doing identity and compute? It started as
plumbing. I had islands of VM servers running Mjolnir and I wanted to unify them
into one reachable fabric — that's literally why the code still says `mjolnir-mesh`
everywhere. But once the islands were stitched, I saw it wasn't a private
convenience at all. It was **broadly applicable to every network**: we can run
services without paying corporate rent for datacenter colo and cloud egress; we can
have a real peer-to-peer internet where *anyone can run a node*. Publishing
something to reach your neighbors shouldn't require becoming a network wizard or
renting reachability back from whoever enclosed it. The mesh was the moment the
identity project stopped being about *me proving who I am* and became about *us
being able to reach each other at all* — without a landlord.

**How we faced it.** The line we hold is that **joining is reachability**: a
Raspberry Pi joins the mesh and `wiki.mesh` resolves from every node — locally with
no internet, globally over the encrypted overlay with it — no port forwarding, no
rented tunnel, no coordination server that isn't yours
([philosophical-outcomes.md §7](../vision/philosophical-outcomes.md)). Rent-seeking
on *reachability* is one of the quieter forms of extraction — the infrastructure to
reach each other already exists, it's been enclosed, and it's sold back by
subscription — and it hurts most exactly the services whose point is *being of
service*: the mutual-aid page, the community wiki that will never have a business
model and shouldn't need one. Structurally, "anyone can run a node" is real because
every node runs identical software with **no special roles**: it derives its
address from its own key, claims its own routed `/24` through the shared state
layer, and routes. There is nothing to be granted, because there is no authority to
grant it.

## 5. The realization: I needed an adversarial ground to build on

**The story.** After some years of designing, scrapping, and redesigning IdentiKey,
the real lesson finally landed: **I needed an adversarial environment to build it
in.** One where I couldn't lean on *any* authoritative networked system being
present — no CA reachable, no DNS I trust, no cloud to hold the canonical copy, no
assumption that the other half of the network is even *there*. Every time I'd let
one of those assumptions back in, IdentiKey quietly stopped being censorship-
resistant, because the censor just moves to the assumption. The only way to build
something that survives the loss of authority is to develop it in a world where the
authority is already gone. So the mesh became the crucible: the harshest possible
substrate — flaky radios, nodes that vanish, partitions, no coordinator — chosen on
purpose, because anything that works *there* is actually free.

**How we faced it.** We committed to the hard side of CAP and never looked back:
when the network partitions — and a radio mesh partitions constantly — we pick
**Availability, every time**, because there's no coordinator to refuse writes on
everyone's behalf ([decentralized-systems-design.md §1](../vision/decentralized-systems-design.md)).
A node cut off from the rest must keep resolving names, keep routing, keep serving
the phones in the room, on its own, with whatever it last knew. That single
non-negotiable — *function with no authority present* — is what forces every good
property downstream: no leader election (that would reintroduce the single point of
capture), no quorum, deterministic merge instead. Building on the adversarial
ground didn't make the system fragile; it's what made it honest.

## 6. The dragons — and the fruit

**The story.** And the things that emerged. If I'd known the dragons down this road
— the total-ordering problem with no trustworthy clock, Sybil attacks in a
permissionless namespace, liveness that no CRDT can express, a browser that locks
you out of its own crypto the moment you don't have a certificate authority — I'm
not sure I'd have set out at all. Each one is a genuinely hard distributed-systems
problem that centralized systems don't *solve* so much as **define away** with an
admin, a database, a login. Take the authority out and every one of them comes back
as first-class engineering. But it's bearing fruit. Every dragon, faced honestly,
left behind a mechanism that's *better* than the centralized shortcut it replaced —
because it can't be captured. What follows (§7) is the bestiary and how each was
answered.

**How we faced it.** See the next section — it's the map of dragons and the specific
answer to each. The meta-point: I stopped treating the weaker-looking guarantee as
a defeat. Strong *eventual* consistency, total order by mathematics instead of by a
privileged node, identity by keypair instead of by KYC — every one is a *different
choice*, and each buys the same thing: **there is no authority to capture, subpoena,
or switch off.**

## 7. The bestiary — the hard parts of having no boss

**The story.** Four dragons, named plainly, because a decentralized system that
pretends it has slain them is lying:

- **Total ordering** — agreeing on *the* sequence of events with no coordinator to
  declare it and no clock you can trust.
- **Sybil resistance** — stopping one actor from fabricating a thousand identities
  and seizing everything first-come-first-served.
- **Liveness** — knowing whether a node is *still here*, which is the one thing a
  CRDT fundamentally cannot tell you.
- **Evolving with no flag day** — changing the wire format across a fleet that will
  never all upgrade at once.

**How we faced it** (each grounded in
[decentralized-systems-design.md](../vision/decentralized-systems-design.md)):

- **Total order → Hybrid Logical Clock.** A stamp is `(wall_clock_ms, counter,
  node_id)`, ordered lexicographically. The wall clock keeps it legible, the counter
  breaks same-millisecond ties, and the **node_id — a public key — is the final
  tiebreak that's guaranteed unique**, so no two events ever compare equal. The
  order is *total* and every node computes the same winner independently, with zero
  coordination. That's what turns "first-writer-wins" from an argument into a
  function (§3 there).
- **Sybil → the network is a projection of keys.** You can't forge a node-id (it's
  an Ed25519 key from a space of 2²⁵⁶), and you can't choose your address — it's
  `blake3(node_id)`. Making a thousand identities is easy; making a thousand that
  *each already own the thing you want* is not. We're honest about the frontier:
  service *names* are still trust-on-first-use today, and the upgrade path is
  web-of-trust name arbitration — identity promoting first-writer-wins from "first
  to speak" to "first *legitimate* claimant" (§4 there).
- **Liveness → a separate ephemeral plane.** This is the piece of genuinely new
  engineering. A CRDT stores *monotonic truth* — facts that stay true. "Is X still
  here?" is the opposite: true, then silently not, with no event marking the death.
  You can't merge your way to an absence. So we **split the planes**: liveness rides
  a tiny beacon that is never merged, never persisted, never relayed; receivers
  judge staleness by their *own* local clock, never a remote timestamp — trusting
  remote clocks to *order writes* but never to *measure liveness*. The durable CRDT
  goes back to storing only truth and only writing to flash when a real fact
  changes (§7 there). Small idea, general shape.
- **No flag day → decode-and-skip.** New message types are *appended* to the wire
  enum so tags never shift; an old node that can't decode a new message logs and
  skips instead of crashing. A five-year-old box and a nightly build share one mesh
  with no negotiation (§6 there). Version tolerance as a property of the decoder,
  not a compatibility matrix someone maintains.

## 8. Standing on giants — resilience is the one I can't put down

**The story.** None of this is from nowhere. I'm standing on the shoulders of
people who fought this fight for decades: **Freifunk**, **LibreMesh**, **NYC Mesh**,
**Guifi.net**, and many others who kept the internet collaboratively owned and
actually resilient. The word that won't let me go is *resilient*. The ARPANET was
designed so that it **couldn't be taken down or taken over by attacking a few
central nodes** — that was the *point*, the founding requirement. And we've spent
forty years engineering that property back *out*, recentralizing onto a handful of
clouds and CAs and platforms until a few well-placed failures — technical, or
commercial, or political — can take down or take over enormous swaths of it. I keep
coming back to the resilience because I think we quietly turned necessary
infrastructure into rented infrastructure, and forgot it was supposed to survive
the loss of its own center.

**How we faced it.** We start where the survivors *arrived*, not where they started
([the technical-arc source §3](dweb-2026-technical-arc.md), and
[philosophical-outcomes.md §1](../vision/philosophical-outcomes.md)): Freifunk hit
the flat-L2 broadcast wall in the hundreds of nodes and segmented into L2 islands
stitched by L3; Guifi.net and NYC Mesh run routing between heterogeneous zones for
the same reason. That *is* our architecture — heterogeneous link islands stitched
by L3 routing — which is also, not coincidentally, Cerf & Kahn's catenet, the one
design that survived every technology it launched on. Resilience is structural
here: no center that's been *hidden*, but no center that *could* exist. Every node
runs identical software; ordering is decided by mathematics every node computes;
identity is a keypair anyone can generate. The ARPANET requirement — survive the
loss of any few nodes — is the requirement we refused to compromise.

## 9. No certificate authority — so the browser locked me out of crypto

**The story.** Here's the dragon that made it visceral. Without an authority, you
have no **certificate authority** — nobody to issue the certs that the whole
encrypted web takes for granted. So we had to do our own key exchange, prove
identity from keys directly. Fine, that's the *point*. But then I ran into a wall I
didn't expect: **the browser shuts off its own crypto when you don't have a CA.**
`crypto.subtle` — WebCrypto, the good non-extractable-key API, the thing you'd
actually want to hold an identity safely — is restricted to "secure contexts,"
which in practice means HTTPS with a CA-valid certificate. Serve a page over plain
`http://hello.mesh` from a router that has no CA, and the browser simply refuses to
hand you real cryptography. The most security-conscious thing in the browser is
unavailable at exactly the moment you're being *most* principled about not trusting
an authority. It felt like the platform punishing you for refusing the landlord.
And it's not academic: for real resilience, when things go wrong, we **need** our
tools for communicating and coordinating to keep working without a CA in reach.

**How we faced it.** First, the honest correction to my own frustration: the browser
isn't malicious, it's a *secure-context gate* — but the effect on a CA-less mesh is
exactly the lockout it feels like. So we met it on two fronts
([user-identity.md §3, §4.6](../network-coordination/user-identity.md)):

- **On the wire, node-to-node, it's already solved, CA-free.** An iroh QUIC
  connection uses the node's Ed25519 key *as* its TLS identity; the handshake proves
  possession. The "ceremony" that replaces a CA is just learning a node's
  `EndpointId` out of band — a ticket, an on-screen QR, an NFC tap — after which the
  channel is cryptographically authenticated with no authority involved. This is our
  own key exchange, and for mesh-native clients it's done.
- **In the bare browser, we tell the truth about custody.** Over plain HTTP there's
  no secure context, so we use a small audited **pure-JS Ed25519** library
  (`crypto.getRandomValues` *is* available even in insecure contexts) with the key
  in IndexedDB — a *real* key, but **soft** custody, because the page and the key
  share a trust domain. We never dress that up as equivalent to hard custody.
  Anyone who wants the key out of the page's reach climbs one rung: a **browser
  extension** page *is* a secure context (real non-extractable WebCrypto, and it
  doubles as a cross-origin keystore), or the **app** on `localhost`, or
  WebTransport's `serverCertificateHashes` to pin a self-signed cert by hash — no CA
  anywhere. The frontier is named, not hidden: the bare-browser tier is deliberately
  disposable, and the reputation layer (client→node trust, "never sign blind") keeps
  it safe *for what it's for*.

The browser lockout stopped being a wall and became a *map of exactly where the
centralizing assumption still bites* — and a ladder of rungs out of it.

## 10. Split-brain is the whole test

**The story.** This is the requirement I care about most, the one that makes me
unable to let go. If our networks bifurcate — split-brain, a partition, a region
cut off, a building's concrete eating a signal, or something far worse — we
**need** the services we use to communicate, collaborate, and coordinate to *still
work*. Both halves. Immediately. On what they last knew. These are not toys anymore;
group chat, directories, shared documents, the way a neighborhood or a movement
*talks to itself* — this is **necessary infrastructure**, and we cannot leave
necessary infrastructure in the hands of Amazon, Google, Meta, or any entity that
is structurally incentivized to *own* the network, because an owner's first move in
a crisis is to decide who gets to keep using it. A network that only works while
it's whole, and only works while the landlord is willing, is not resilient. It just
hasn't been tested yet.

**How we faced it.** Split-brain isn't an error case we handle; it's the **normal
operation running after a gap**
([decentralized-systems-design.md §5](../vision/decentralized-systems-design.md)).
When a partition heals, the two sides simply gossip their states and merge — there's
no "primary" to resync against, no split-brain to reconcile by hand, because the
merge is deterministic and symmetric (that's what the HLC total order and the CRDT
merge rules *buy* us). Two meshes that were never joined fuse the same way, by
linking at a single node. **Anti-entropy** makes it a standing property rather than
a procedure: every node periodically re-broadcasts its full (small) maps, so a late
joiner, a node that missed packets during the partition, or a box that just rebooted
all catch up on the next tick — no "please re-send what I missed" handshake. We've
watched it in the field: a router slept through an entire fleet update, came back,
and converged within seconds with nobody reconfiguring anything
([technical-arc §7](dweb-2026-technical-arc.md)). The test isn't hypothetical, and
the system is built to pass it by construction.

## 11. The why behind the why — connectivity is a human right

> **Handle with care.** This is the moral center of the talk, and it's about real
> people who are in danger *right now*. The stage-ready version is below; the fuller,
> rawer account and the things that must **not** be said in public live in the
> companion note [dweb-2026-myanmar-why-PRIVATE.md](dweb-2026-myanmar-why-PRIVATE.md).
> Read that note's opsec section before adapting any of this for an audience.

**The story.** Everything above is *why the architecture is right*. This is why it
matters. I was invited to a small, private gathering — people working on distributed
systems and identity, and several anonymous refugees from Myanmar. We spent days
listening.

One of the weapons being used against the ethnic peoples there is **disconnection
itself.** Cut the cell towers. Ban Starlink. Choke off the ISPs. Make a region go
dark. I heard what that does to a life: news reduced to what a person can hand-copy
and carry by motorbike across hundreds of miles — *which towns had burned, whether
the soldiers were coming toward you or turning somewhere else.* The not-knowing. No
way to reach anyone, not the outside world, just your village in the dark waiting to
find out.

Cutting people off from each other to take them apart piece by piece is *exactly*
the thing the internet was built to make impossible. The ARPANET's founding
requirement — survive the loss of your center — wasn't an abstraction; it was
*this*. And yet here we are, with connectivity centralized onto so few points that
it can be switched off region by region. That is where "communication is a human
right" stopped being a phrase to me.

So I'll say it plainly: **connectivity is a human right**, and a network worth
building is one that *cannot be taken away this easily.* This isn't a political talk,
exactly — but what, that has a fundamental effect on our capacity to reach one
another, *isn't*? The threat model for this whole system was never hypothetical to
me after that week. It has faces.

**How we're facing it.** Read from there, every "how we faced it" in this doc is a
countermeasure to disconnection as warfare. *No tower to cut* — the mesh rides
whatever link is present: radio, wire, a neighbor's uplink, one satellite hop shared
onward (§4). *No ISP to ban* — joining **is** reachability; there's no provider to
switch off (§4). *No coordinator or CA to seize* — the network keeps resolving names
and routing with no authority present (§9); the property that makes it
censorship-resistant in a conference hall is the same one that matters under a
junta. *Split-brain that keeps working* (§10) isn't a systems-design flourish; it's a
community that stays able to talk to itself when the outside line is cut. We built
for the adversarial ground on purpose (§5) — and the most adversarial ground there
is happens to be where people are made to disappear by being made unreachable.

I won't overclaim: a mesh node is not a shield against an army, and I'd never
pretend otherwise. But connectivity with no single throat to choke is *harder* to
take away — and that is the whole reason the hard engineering above is worth doing.
That is the why behind the why.

## 12. Locked open — the thing this is really about

**The story.** So, all of it — the identity that's just a key, the network that's a
projection of keys, the compute that's processes on channels, the refusal of every
authority the modern internet assumes — comes down to one design goal, and it's not
a technical one. **Make it fundamentally not ownable. Locked open.** Not "open" as
in "we chose to open it and could close it later," but open the way a mathematical
fact is open — with no center that could be bought, seized, subpoenaed, or
enshittified over time, because there is no center that *could* exist. That's the
whole point of doing the hard distributed-systems work instead of the easy
centralized shortcut: the shortcut always leaves a throat to choke, and someone
eventually chokes it. Our freedom to communicate, to coordinate, to be someone
online without permission — increasingly that freedom *depends on* infrastructure
that cannot be owned. So we build it to be un-ownable, on purpose, all the way
down.

**How we faced it.** "Locked open" is the sum of every mechanism in this doc, and
it's checkable, not decorative
([decentralized-systems-design.md §8](../vision/decentralized-systems-design.md)):
no transaction log to subpoena (HLC total order, computed per-node), no account
system to compel an admin to ban you from (identity is a keypair, verification is a
signature check), no coordinator to take offline to take the network offline
(gossip + deterministic merge, availability under partition), no single vendor whose
incentive gradient bends the system toward extraction (every node runs identical
software, no special roles), and no flag day a gatekeeper controls (decode-and-skip
version tolerance). The decentralized web isn't centralized services with the logo
filed off and a token bolted on — it's systems whose **correctness does not route
through anyone's authority.** That's the sentence I'd want the room to leave with.

---

## The emotional arc, for delivery

If the technical-arc talk is "here's why this is the right architecture," this one
is "here's why I couldn't stop." The shape to hit on stage:

1. **I wanted to build identity, and the internet wouldn't let me do it honestly.**
   (Every layer assumed an authority I was trying to remove.)
2. **So I had to build the ground first** — a network with no authority to capture —
   and to build *that* honestly I had to develop it somewhere the authority was
   already gone. The mesh is that adversarial ground, chosen on purpose.
3. **The dragons were real** (total order, Sybil, liveness, no CA, the browser
   locking me out), and facing each one honestly left a mechanism better than the
   centralized shortcut, because it can't be captured.
4. **The stakes are resilience.** The ARPANET was built to survive losing its
   center; we engineered that away; split-brain-that-keeps-working is us engineering
   it back.
5. **And resilience has faces.** In Myanmar, disconnection is a weapon — towers cut,
   Starlink banned, news carried by motorbike. Connectivity is a human right, and the
   hard engineering above is worth doing precisely because it's *harder to take
   away*. *(This is the emotional peak — land it, then don't linger; see the private
   note on pacing and opsec.)*
6. **The goal is freedom, and freedom now depends on un-ownable infrastructure.**
   Locked open. That's why I can't let go.

## One-liners (personal register, for slides)

- "I set out to build identity and discovered I had to build the ground it stands
  on first."
- "Centralization isn't a feature of the internet you can opt out of — it's an
  assumption baked into every layer, and you have to tease it out by hand."
- "I didn't want to be an authority *or* a wallet. I wanted you to just be your
  key."
- "λ-calculus assumes one machine, one owner, one clock. π-calculus assumes many
  parties and no boss. Guess which one the real world is."
- "I needed an adversarial environment to build in — one where I couldn't assume
  any authority was even *reachable*. The mesh is that crucible, on purpose."
- "If I'd known the dragons down this road, I might not have started. I'm glad I
  didn't know."
- "Take the authority away and total ordering, Sybil, and liveness all come back as
  real engineering. Centralized systems don't solve them — they define them away."
- "Without a CA, the browser locks you out of its own crypto. The platform punishes
  you for refusing the landlord."
- "Split-brain isn't the failure case. Split-brain-that-keeps-working is the *whole
  test.*"
- "The ARPANET was designed to survive losing its center. We spent forty years
  engineering that away."
- "Necessary infrastructure can't live in the hands of anyone whose incentive is to
  own it."
- "Cut the towers, ban Starlink, choke the ISPs, and a region goes dark. When
  disconnection is a weapon, un-ownable connectivity is the answer."
- "When the only news is what fits on a page carried by motorbike, 'harder to take
  away' stops being a spec line."
- "Connectivity is a human right. That's not the reason I started — it's the reason
  I can't stop."
- "Not open because we chose to open it. Open the way a fact is open — with no
  center that could ever be closed."
- "Our freedom depends on infrastructure that can't be owned. So we build it
  un-ownable, all the way down."

## Related documents

- [DWeb talk — technical-arc source](dweb-2026-technical-arc.md) — the same ideas
  as systems design; the sibling to braid this with.
- [dweb-2026-myanmar-why-PRIVATE.md](dweb-2026-myanmar-why-PRIVATE.md) — the fuller,
  rawer account behind §11, plus the opsec/ethics guardrails. **Not for
  publication**; read before adapting §11 for any audience.
- [The Hard Parts of Having No Authority](../vision/decentralized-systems-design.md)
  — the engineering behind every "how we faced it" in §7–§12.
- [Philosophical outcomes of the architecture](../vision/philosophical-outcomes.md)
  — what the mechanisms *mean* for ownership and reachability.
- [User Identity & the Front Desk](../network-coordination/user-identity.md) —
  IdentiKey, the custody spectrum, the no-CA / WebCrypto story (§1, §9 here).
- [Mjolnir integration](../vision/mjolnir-integration.md) — the compute fabric and
  the π-calculus angle (§2 here).
