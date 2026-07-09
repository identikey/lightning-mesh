# Locked Open: Building Networks With No Authority to Capture — Talk Outline (v2)

> **Title:** "Locked Open: Building Networks With No Authority to Capture."
> (Submitted/program title was "Overcoming Implicit Authority Structures To
> Build a P2P Mesh" — if the program can't be updated, the old title stays in
> the listing and this one goes on the title slide.)

**Status:** Working outline — the talk skeleton | **Slot:** 1 hour (~45–50 min
talk + Q&A) | **Audience:** DWeb Camp — open-hearted technologists and
dreamers, people who want to build a more egalitarian world together, Freifunk
folks in the room, friends from Myanmar in the room | **Date:** 2026-07-07

> Braided from the three source cuts
> ([commons-frame](dweb-2026-commons-frame.md),
> [personal-narrative](dweb-2026-personal-narrative.md),
> [technical-arc](dweb-2026-technical-arc.md)) but written fresh — this doc is
> not bound to their tonality. Register: invitation, possibility, resolve.
> These people are kin; the talk is a gift to the room, not a defense of a
> product.

## The talk in one sentence

**Find the implicit authority hiding in a system, remove it with keys,
peering, and convergence — and what falls out is a configuration-free network
nobody can own.** Taught as a **portable method** that can be applied to any
centralized system, grounded in one real use case — so the room leaves
empowered to apply the move to whatever *they're* building.

## The organizing principle: the story of joining a commons

The triad isn't just a mnemonic — its arc **is the talk's story**, and every
beat is a stage of it:

- **Keys — what you hold.** You mint yourself. Nobody admits you, so nobody
  can expel you.
- **Peering — how you relate.** You connect as an equal. Nobody's customer,
  nobody's tenant.
- **Convergence — where you all arrive.** Shared truth with no referee. The
  commons agrees with itself.

Hold → relate → arrive: the story of joining a commons. And the crucial
sharpening: **the act of joining *creates* the commons.** There is no
pre-existing thing that admits you — a commons with a front door would have a
doorman, and a doorman is an authority. The network *is* its participants:
add a node and the commons grows; your capacity added is everyone's; two
commons that meet become one. Joining is constitutive, not granted — that's
what "permissionless" means when it's load-bearing.

The cold open is the audience literally living this (their joining *is* the
network the room watches come into being). The case studies are the three
stages done rigorously. The Myanmar beat is what it costs when a people's
commons is taken — and why one that exists *by* participation can't be
confiscated at the front desk. "Locked open" is the commons made permanent.
Every beat should know which stage of joining-that-creates it's telling.

## The principles (the educational spine)

A running device: every time the method removes an authority, the slide names
the **principle** gained, in the shared vocabulary of this movement. By the
end the room has assembled the full set, term by term:

| Principle | What it means here | Earned in beat |
|---|---|---|
| **Permissionless** | Participation requires no one's approval — keys are created by oneself, not granted | 5, 8 |
| **Censorship-resistant** | No control point where speech/reachability can be revoked | 7, 8 |
| **Self-sovereign** | You hold your identity and data; nothing is custodied for you by default | 8 |
| **Disintermediated** | No landlord in the path — reachability isn't rented back to you | 7 |
| **Symmetric / egalitarian** | Every node runs identical software; there is no role a boss could occupy | 5, 6 |
| **Resilient / partition-tolerant** | Works when split, degraded, offline; heals by merge. The human keystone. | 6, 7, 9, 10 |
| **Un-ownable ("locked open")** | No center could be bought, seized, or enshittified, because no center can exist | 11 |

**Resilience** is the emotional center of the set — it's the one that has
faces (beat 10) — and **locked open** is the summit the rest climb toward.

## Audience-sensitivity notes

- **Freifunk is in the room.** They are lineage and kin — the intrepid souls
  who kept collaboratively-owned networking alive, ran the flat-L2 experiment
  at scale, and re-derived the catenet under duress. We start where they
  arrived, with gratitude — and we're actively working on how our networks can
  be complementary. Any "detour" critique aims exclusively at big-tech mesh
  (eero/Google Wifi: flat L2 + proprietary controller + mandatory cloud).
- **Myanmar.** Cleared with the person concerned — nothing here is
  private/secret. The one rule that remains is pacing: don't linger on the
  trauma. Tell it with respect, land it, move to resolve. Their struggle is
  directly relevant to this community: people from Myanmar have been coming to
  DWeb for years, working on how to run government services without access to
  the main internet.

---

## Beat sheet (~48 min)

### 1. Cold open — the demo (5 min)

Any old computer, plugged into any mesh router — behind your regular home
router. No VPS, no Tailscale account, no port forwarding, no config. It's
reachable by name from every node on the mesh — and from anywhere on Earth.
**Now cut the internet.** Same box, same name, still reachable across the
radio hops, exactly the same way.

Live if at all possible: **`http://hello.mesh`** (say the `http://` — the
browser won't believe it's a website otherwise). The room watches the network
*socially*: clients coming online, people joining, keys appearing, liveness
status changing in real time. Seeing the network see itself is half the magic.
Demo app candidates by Thursday: text chat; browser-based walkie-talkie (p2p
voice from a phone's web browser); wiki or MySpace-clone. Whatever ships, the
*seeing-each-other* part is the vital piece — this is a social fabric, not a
router feature.

> "This talk is about why that works — and why it's structurally impossible on
> the internet you're using right now."

### 2. The promise — what we get to build (3 min)

The affirmative frame: a private internet of our own. Services for each
other, discoverable by name, on hardware we hold — private when we want,
spanning the public internet when we want reach. Everything web3 promised —
own your identity, own your data, permissionless, disintermediated — delivered
by **keys, peering, and convergence. No speculative asset in the path.** ("Peering" chosen
deliberately over "routing": it's the term of art for networks connecting as
equals, settlement-free — vs. *transit*, where you're a customer.
"Convergence" chosen over "merge": it's the actual guarantee — separate paths
arriving at the same place with no referee — explained via the `conflicted
copy (2)` hook. The triad doubles as the take-home checklist: who mints your
identity? peer or customer? who decides whose version wins?)

This is an invitation, not a fortress. The rest of the talk is the method,
and what it protects.

### 3. The problem — implicit authority at every layer (6 min)

The lens. The modern stack assumes an authority everywhere, so deeply it's
hard to even *see*:

- **DHCP**: an address is something a server *grants* you.
- **DNS**: a name is something a registrar *grants* you.
- **TLS**: trust is something a CA *grants* you.
- **Login**: being someone is something an issuer *grants* you.
- **The phone number**: the identifier you're *called by* is leased from a
  carrier — and it's a primary tracking and surveillance tool, for states and
  platforms alike.

Each grant point is a **locus of control**, and control gets used: rent
extraction, surveillance, enshittification, shutoff. The ARPANET's founding
requirement was survive-the-loss-of-your-center; forty years of
recentralization engineered it back out.

Seed the stakes in one sentence: "And in at least one country right now,
disconnection itself is being used as a weapon. We'll come back to that."

### 4. The road here — origin story (5 min)

Told plainly, in first person:

1. **IdentiKey**: I set out to build key-based identity — you are your key,
   the network just verifies signatures. To solve the immediate problem I
   stood up an OAuth server. It worked — and that was the trap. It answered
   "how do I log in" by *being an authority*: an issuer, a place that says yes
   or no, a thing you phone home to.
2. **Mjolnir** (brief): in parallel, decentralized compute — processes and
   channels rather than one machine, one owner, one clock. The mesh began as
   plumbing to unify islands of Mjolnir servers — and then turned out to be
   the actual point.
3. **The realization**: I couldn't build censorship-resistant identity on
   ground that assumes an authority at every layer — the censor just moves to
   the assumption. **I had to build the ground first**, and prove it in the
   most adversarial environment available: flaky radios, partitions, nodes
   vanishing, no coordinator anywhere. Anything that works there is actually
   free. (And this is why identity returns as the final case study.)

(Cut from v1: the "two missing ideas / symposium" beat. What mattered humanly
was finding people who *get it* to refine the ideas with — that sentiment can
surface as one warm line here or in the close: this community is where the
ideas got sharpened.)

### 5. The move — the portable method (5 min)

Taught once, explicitly, as a method you can take home:

1. **Symmetry**: every node runs identical software; no special roles. Not
   "we don't elect a leader" — *there is no role a leader could occupy*.
   → *principle: symmetric/egalitarian*
2. **Conflict-free coordination**: shared state as CRDTs merged over gossip; a
   Hybrid Logical Clock gives a total order every node computes identically —
   "first-writer-wins" becomes a *function*, not an argument. Availability
   under partition, always; deterministic merge instead of leader election.
   → *principle: resilient/partition-tolerant*
3. **Self-created keys**: identities are keypairs you generate yourself —
   nothing to be granted, so nothing to be revoked, for nodes and for people
   alike. → *principle: permissionless*

The corollary that makes it land: **no authority means no authority to
configure.** Zero-config isn't a convenience feature; it's what's left when
nothing exists whose existence would need configuring.

Then the lineage, told as gratitude: **Cerf & Kahn's catenet** — heterogeneous
link islands stitched by routing, the one architecture with a fifty-year track
record — and the intrepid souls of the mesh movement (**Freifunk, Guifi.net,
NYC Mesh, CeroWrt**) who kept collaboratively-owned networking alive and
re-derived that architecture under duress, at scale, in the field. We start
where they arrived. And we're working on how these networks interconnect.

### 6. Case study 1 — DHCP without the server (6 min)

- The implicit authority: the DHCP server is the network's memory and its
  adjudicator. And it's not abstract — **DHCP authority is one of the primary
  drivers of fiddly network configuration**: which box runs the server, which
  range it hands out, what happens when two of them disagree. It's the source
  of the weird failure modes everyone here has debugged when you **plug two
  networks together** — duplicate servers, colliding ranges, devices confidently
  wrong about where they are. The authority doesn't just centralize; it makes
  networks *brittle at their seams*.
- The move applied: each node claims its own routed `/24` in the shared CRDT
  state; collisions are resolved by the deterministic merge — every node
  computes the same winner, no arbiter. (We happen to derive the claim from
  the node's key hash, but that's a convenience, not the load-bearing part —
  random claims would disambiguate the same way. **The merge is the
  mechanism.** Where keys become genuinely powerful is naming — next beats.)
- What falls out: plug in a router, it addresses itself, claims its block,
  routes. Unplug it, the mesh adapts. Plug two *whole meshes* together and
  they merge the same way — the seam just heals. → *principles: symmetric,
  resilient*

### 7. Case study 2 — reachability & naming without the landlord (9 min)

The heart, and the payoff of the cold open. Two layers, presented as one
capability:

- **Iroh — reachability from identity.** This deserves its own moment,
  promulgated as a method in itself: your public key *is* your address.
  Dial a key and the connection finds its way — link-local, multi-hop across
  radios, or across the open internet, through NATs, end-to-end encrypted
  between identities the whole way. **The same dial works in a disaster zone
  with no internet and on a fiber connection with full internet** — which is
  what makes this a *universal* system rather than an offline-mode special
  case. We've concentrated our engineering on the internet-less local case,
  but the architecture is one continuous fabric. → *principles:
  disintermediated, censorship-resistant*
- **Naming — the gossip address book.** Names (`hello.mesh`) live in the same
  CRDT state as everything else: replicated mesh-wide, no registrar, no DNS
  server to run or seize, survives partition, merges on heal. First-writer-
  wins arbitration by the HLC — and forward-looking: because names are claimed
  by *keys*, the trust story upgrades from "first to speak" toward reputation
  and web-of-trust arbitration — **the same identity method for nodes as for
  people, keys created by oneself rather than given.** (mDNS interop at the
  local edge is a compatibility detail, one line, not the story.)
- Depth beat — the liveness dragon (keep; it's the best pure-engineering
  moment): a CRDT stores facts that stay true; "is X still here?" is
  true-then-silently-not. You can't merge your way to an absence. So liveness
  rides a separate ephemeral plane — never merged, never persisted; staleness
  judged by the receiver's own clock. This is what the demo's live
  presence view is showing.
- Status, honestly: address book shipped and field-validated on the router
  fleet (a router slept through a fleet update, came back, converged in
  seconds — nobody touched anything). Names/services in build; demoed at camp.
- **Close the loop with the cold open**: this is why the box was reachable by
  name from anywhere, and why cutting the internet changed nothing.
  Reachability is a property of *joining*, not a product you subscribe to.

### 8. Case study 3 — identity without the issuer (8 min)

IdentiKey — the project that started it all, standing on ground that can
finally hold it:

- The implicit authority: issuers, relying parties, platform vendors — and
  most intimately, **the phone number**: the strong identifier nearly every
  service demands, leased from a carrier, and a primary surveillance and
  tracking instrument. Being reachable *as yourself*, by an identifier you
  created and no one can revoke or subpoena the registry of — that's the
  quiet, radical thing. → *principles: permissionless, self-sovereign*
- The move applied: **the protocol verifies only signatures.** An identity is
  an Ed25519 keypair; devices hold their own keys; the identity key signs an
  attestation over the device key; anyone verifies the chain offline. No
  issuer contact, ever. (Passkeys evaluated and declined: they bind identity
  to a DNS domain and a platform vendor.)
- **Anonymity is a first-class rung, not a failure mode**: a temporary keypair
  created just for one interaction is a complete, valid identity. From there a
  custody spectrum the user picks — throwaway, soft, custodial-by-choice,
  hardware — and services see one uniform thing: a key and valid signatures.
- **Identity without a panopticon** (seeds beat 10): an identity system is
  also a *target* — a registry is something a regime can seize and read
  backwards. In Myanmar, carrying the wrong ID at a checkpoint is grounds for
  arrest. Design consequence: there is no registry. Verification is pairwise
  and offline; what doesn't exist can't be seized. → *principle:
  censorship-resistant*
- The visceral dragon (keep, compressed): refuse the CA and the browser shuts
  off its own crypto (`crypto.subtle` needs a "secure context"). The platform
  punishing you for refusing the landlord — and the ladder out of it.

### 9. What falls out — sovereignty as topology (3 min)

Rapid synthesis, one line per slide:

- Zero configuration — nothing to configure, because no authority to configure.
- Meshes **merge** — networks built independently fuse by linking one node.
- **Split-brain-that-keeps-working is the whole test** — both halves keep
  working on what they last knew; healing is a merge, not a resync.
- The network outlives the hardware — links are plumbing; we retired a whole
  radio generation from the fleet and the mesh didn't notice.
- Nobody is in charge — and nobody is in a *position* to become in charge.
  Not policy; topology. → *the principles table, now complete except one*

### 10. Why it matters — Myanmar (5 min)

(Pacing rule: tell it with respect, don't linger on the trauma, land it, and
turn to resolve.)

This community already knows: people from Myanmar have been coming to DWeb
for years, working on a problem most of us have never had to pose — **how do
you run government services with no access to the main internet?** I've sat
with them; we worked on identity systems for a provisional government, and the
first order of business turned out not to be identity at all. It was
**connectivity itself.**

One of the weapons used against the ethnic peoples there is **disconnection**:
cell towers cut, Starlink banned, ISPs shut off; a region made to go dark.
News reduced to what a person can hand-copy and carry by motorbike across
hundreds of miles — which towns had burned, whether the soldiers were coming
toward you. Cutting people off from each other to take them apart piece by
piece is exactly the thing the internet was built to make impossible.

And the second-order lesson, from the identity work: **the fix must not
become another panopticon.** People are already pulled over and arrested for
carrying the wrong ID. An identity registry is a weapon waiting to change
hands — which is why the design has no registry to seize (beat 8 was the
countermeasure all along).

Honest claim, no more: a mesh node is not a shield against an army. But a
network with **no head to cut off** is harder to take away — no tower to cut,
no ISP to ban, no coordinator to seize, split-brain that keeps working.
**Resilience** is the principle with faces on it. Connectivity is a human
right.

### 11. Locked open — close and call to action (4 min)

The turn from grief into resolve: make it **fundamentally not ownable.**
**Locked open** — not "open because we chose to open it, and could close it
later," but open the way a mathematical fact is open: no center that could be
bought, seized, subpoenaed, or enshittified over time, because no center *can
exist*. This is the most revolutionary freedom the method buys — the last
principle, and the one the others climb toward.

The decentralized web isn't centralized services with the logo filed off —
it's systems whose **correctness does not route through anyone's authority.**

The call to action is the method, handed over: find the implicit authority in
the system *you're* building. Ask what it grants — and whether the grant could
become a **derivation** (self-created keys) or a **merge** (conflict-free
coordination). Our ways of **coordinating and collaborating** are necessary
infrastructure now; build them resilient, build them un-ownable, so that what
we make together stays *ours*. The mesh is an invitation — and it merges.

---

## Timing budget

| # | Beat | Min | Cum |
|---|------|-----|-----|
| 1 | Cold open — demo | 5 | 5 |
| 2 | The promise | 3 | 8 |
| 3 | Implicit authority everywhere | 6 | 14 |
| 4 | Origin story | 5 | 19 |
| 5 | The move + lineage | 5 | 24 |
| 6 | DHCP case study | 6 | 30 |
| 7 | Reachability & naming (iroh) | 9 | 39 |
| 8 | Identity case study | 8 | 47 |
| 9 | What falls out | 3 | 50 |
| 10 | Myanmar | 5 | 55 |
| 11 | Locked open + CTA | 4 | 59 |

59 on paper → target ~50 spoken so Q&A fits. Compression candidates: beat 3
to 5 (fewer grant examples), beat 6 to 5, beat 8's browser dragon to one
slide. The demo (beat 1) is the one beat that should get *more* time if it's
working live, stolen from 3/6.

## Open questions

- Demo scope by Thursday: presence view (`http://hello.mesh`) is the floor;
  chat / browser walkie-talkie / wiki-or-MySpace-clone as stretch. Which one
  makes the *social fabric* most visible?
- Freifunk complementarity: one concrete sentence about the interconnection
  work, or keep it at "we're working on it"?
- Whether beat 2 folds into beat 1 — the demo already *is* the promise, shown
  rather than told.
- Slide device for the principles table: reveal terms one at a time as earned,
  assemble the full set on the beat-9 slide?

## Related documents

- [Commons-frame](dweb-2026-commons-frame.md) — beat 2 register.
- [Personal-narrative](dweb-2026-personal-narrative.md) — beats 4, 10, 11
  source material (tone superseded by this doc).
- [Technical-arc](dweb-2026-technical-arc.md) — beats 5–9 source material;
  one-liner bank.
- [Myanmar note](dweb-2026-myanmar-why-PRIVATE.md) — background; guardrails
  loosened per direct conversation (2026-07-07): nothing private/secret, the
  remaining rule is pacing — don't linger on the trauma.
