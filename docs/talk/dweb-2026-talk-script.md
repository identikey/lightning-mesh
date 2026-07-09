# Locked Open: Building Networks With No Authority to Capture — Stage Script

**Status:** Stage script, in progress — written beat by beat against
[the outline](dweb-2026-talk-outline.md) | **Register:** spoken word, first
person | **Convention:** *[bracketed italics]* are staging/delivery cues, not
spoken. | **Style rule:** simplicity is the aesthetic of confidence. No
self-announcing importance ("this is the core of the talk", "I want to be
really precise here") — the material earns attention by being true and vivid,
not by asking for it. The talk gets one or two explicit lean-in moments total;
"Cut the internet" owns the first. Enthusiasm and vivid imagery: yes, always.
And: **technologist, not showman.** No "this one's my favorite," no "watch
this" framing — rhetorical devices guide awareness (structure, contrast, a
well-placed image); they don't sell the next paragraph. Clarity and precision
are the register; color comes from the material being genuinely strange and
good.

---

## Beat 1 — Cold open: the demo (~5 min)

*[Before you're introduced, if logistics allow: the venue mesh routers are
already up. The slide behind you shows only two lines, big:]*

> **Join wifi: `lightning`**
> **Open: `http://hello.mesh`** *(yes, type the `http://`)*

*[Walk out. Don't introduce yourself yet.]*

There's a wifi network in this room called
`lightning`. Join it. Then open your browser and go to `http://hello.mesh`.
You have to type the `http` part — your browser doesn't believe this website
exists. 

*[Beat. Let people fumble with phones. Switch the screen to your own view of
hello.mesh — the live presence page.]*

While you do that, watch the screen. Every one of those entries appearing —
that's one of you. That's you joining a network. Not my network — a network.
One that you're now a first-class citizen of. And notice — it's
bigger than it was a minute ago *because* you joined. You're not entering a
thing that already exists. You're creating it. Right now, together.

*[As identities populate:]*

What you're looking at is the network seeing itself. Each row is an identity —
a cryptographic key that your browser just created, on your phone, by itself.
Nobody issued it to you. Nobody approved it. There's no account. And here's a
question worth asking about any web page: *what server is this running on?*
The answer here is: whichever router is nearest to you. Every router in this mesh runs a
tiny web server — the front desk — and serves this same page from itself. The
little liveness dot next to each name — who's here, right now — isn't coming
from *a* server, because there is no *the* server. Every router in this room
holds this same list, merged via a CRDT, and they agree on it without any of them being in
charge.

Now for the second half of the demo.

*[Hold up, or point to, the demo box — an ordinary small computer (SBC or the
talk laptop) wired to one of the mesh routers.]*

This is a completely ordinary computer. Earlier today I plugged it into one of
the mesh routers here — the same way you'd plug anything into the router
behind your couch. I didn't configure it. No port forwarding, no dynamic DNS,
no VPS in the middle, no Tailscale account, no certificate. I plugged it in,
and it announced a name. You can see it from where you're sitting — it's the
app linked right there on the front desk page.

*[Tap the link on hello.mesh — the chat / walkie-talkie. If shipped: send a
message to the room, or take a p2p voice call from a phone's browser. One
interaction, fast, no dwelling.]*

That app is running on *this box*, and its name resolves from every node on
this mesh. And if my home mesh were linked in, it would
resolve from my house too. From anywhere on Earth, over an encrypted overlay,
by the same name.

That's nice. Plenty of products will sell you that. Here's what no product
sells you.

*[The turn. Slow down.]*

Cut the internet.

*[If staged live: actually pull the venue uplink, or kill the upstream on the
gateway router. Let the presence page keep ticking.]*

Nothing happened. Look at the page. Everyone's still there. The name still
resolves. If the box is serving a chat, the chat still works. Because none of
this depends on the internet being there. As long as these radios can
reach each other — hop by hop, across the room or across a valley — you can
plug a computer into any router in the mesh and every other node can reach it,
by name, exactly the same way. No egress required. No permission required.

*[Beat.]*

There are people in this room — and I'll come back to this near the end —
for whom "the internet got cut" is not a hypothetical. It is a weapon that is
being used against them, right now. Hold that thought.

So here's what this talk is about. Everything you just watched is
*structurally impossible* on the internet you use every day. Not difficult —
impossible, by design. Because at every single layer of the modern stack,
there's an **authority** baked in: something that grants you an address, grants
you a name, grants you trust, grants you an identity. And whatever is granted
can be revoked, rented, surveilled, or switched off.

What I want to give you tonight is not a product pitch. It's a method — a
portable method — for finding those hidden authorities in any system, and
removing them. We applied it to networking. You can apply it to whatever
you're building. And I want to show you the strange, wonderful thing that
falls out when you do: a network with no center and no configuration necessary. A network that can't be owned — by anyone,
including me.

My name is Duke. Let's get into it.

*[Slide: talk title. Only now.]*

---

### Delivery notes — beat 1

- **The two-line slide stays up for the whole walk-in.** Resist narrating over
  the join instructions; give the room 20–30 quiet seconds to actually do it.
  The fumbling *is* the point — it's the last time joining a network will feel
  like work all evening.
- **"Not my network" is the first thesis statement.** Don't rush it.
- **The internet cut is the money moment.** If it can be done live, rehearse
  it twice with the actual venue uplink. If it can't, say so honestly —
  "I can't cut the venue's internet, they'd tackle me, so here's the same
  thing on video" — honesty plays better than a faked live moment.
- **The Myanmar seed is one sentence, delivered plainly, not ominously.**
  It's a promise to the audience, not a mood shift. Move on with energy.
- **Architecture, for accuracy on stage:** `hello.mesh` (the front desk) is
  served by a tiny web server running on *every router* — whichever router
  you're nearest answers. The apps (chat/walkie-talkie/wiki) run on the demo
  box and are *linked from* the front desk page. Don't blur the two: the front
  desk shows there's no *the* server; the demo box shows anyone can plug one
  in.
- **Optional flex, if rehearsed:** power off the router serving your own
  hello.mesh view mid-beat and reload — the page comes back from the next
  router. Only do this if it's been tested at the venue; it's a second money
  moment but not worth a fumble.
- **Fallbacks:** if `hello.mesh` won't demo, the entire beat works narrated
  over screenshots — but fight for the live version; the room joining the
  mesh with their own phones is the single most persuasive minute available.
- **Timing:** ~5 min with a live join; ~3:30 narrated.

### Where beat 2 folds in

The outline's open question — whether "the promise" (beat 2) folds into the
cold open — is answered *mostly yes* by this draft: "a network that can't be
owned, by anyone, including me" carries the promise register already. What
remains of beat 2 is the **web3 line** ("everything web3 promised, delivered
by keys, peering, and convergence — no speculative asset in the path") — which now lands better
as the *closer* of the problem section (end of beat 3), as the pivot from
"here's what's wrong" to "here's the method."

---

## Beat 3 — Learning to see the authority (~6 min)

*[Title slide is up from the end of beat 1. Advance to a plain slide:
"Whose permission am I asking for right now?"]*

I want to start with something that's designed to be invisible.

Here's an exercise. Think about what happened, at a protocol level, when your
phone joined the wifi just now — this one, or any network, ever. Your phone's
first act, before it can say anything to anyone, is to ask permission to
exist. It broadcasts, essentially: *is there an authority here? May I have an
address?* That's DHCP. And somewhere, a server — one particular box, with one
particular config file — decides what you're called and whether you're allowed
on. Your device cannot participate in the network until something in charge
says yes.

That's the first rung. 

*[Slide: the Ladder of Centralizing Control. Reveal one rung at a time.]*

You want a **name** — something people can find you by? Names are granted by a
registrar. You rent yours. Stop paying, and it's someone else's — along with
everyone's links to you.

You want to be **trusted** — the little padlock? Trust is granted by a
certificate authority. A company on a list you've never read, baked into your
browser by a vendor you didn't choose. The padlock doesn't mean "this is
safe." It means "an authority vouched."

You want to **be someone** — to log in? Being someone is granted by an issuer.
Sign in with Google, sign in with Apple. Your ability to be yourself on the
internet is a service, provided to you, revocably, by a company.

And the deepest one, the one we've stopped even noticing: the identifier
people actually reach you by — your **phone number**. You lease it from a
carrier. Every app demands it as proof you're real. And it is, not by
accident, the single best surveillance and tracking handle that has ever
existed. The thing you're *called by* is a tracking device you pay for
monthly.

*[Pause on the full ladder.]*

Address, name, trust, identity. Four layers, and at every one, the same shape:
something you'd think was *yours* is actually **granted** to you. And it's not that the people running these authorities are bad.
The problem is structural. **Every grant point is a locus of control.** And
loci of control get used — not always, not by everyone, but eventually,
reliably, by whoever can gain advantage or profit from them the most.

Control gets used as *rent*: you can't reach your own computer behind your own
router without paying somebody — a cloud provider, a tunnel service — for the
privilege of reachability. The infrastructure to reach each other exists; it's
been enclosed; access is sold back by subscription.

Control gets used as *surveillance*: the phone number, the account, the
certificate log — every grant is data that lives in a registry, a map
of who you are and who you talk to. We all know that collected data gets sold, and used.

And control gets used as a *switch*: whatever is granted can be revoked. An
account, a name, a route, a whole region's connectivity. 

*[Beat. Shift weight — history now.]*

The internet's original design brief — the
actual founding requirement of the ARPANET — was *survive the loss of your
center*. A network that keeps working when any part of it is destroyed. That
was the whole point. And then we spent forty years carefully engineering that
property back out — recentralizing onto a handful of clouds, a handful of CAs,
a handful of corporate platforms — until a few well-placed failures, or decisions, can
switch off enormous swaths of it. We took the one network designed to survive
decapitation and gave it a head.

*[Beat. Then the pivot — energy up, this is the turn to possibility.]*

Now — none of this is new. This room knows it better than anyone.
And a whole movement already named the cure: own your identity, own your data,
permissionless participation, services no one can enclose. 
You may recognize this: the
promise of web3. Good promises! And then it strapped a
speculative asset to every single one of them and set the whole thing on fire with grift and greed.

So here's the claim of this talk. **Keep the original promise of web3. Throw away the
mechanism.** Everything web3 said we'd get — a network we own, that no one can
capture — is deliverable with three ingredients, none of which puts a
speculative asset in the path.

Self-generated **keys**: identity you mint yourself. Nobody issues it, so
nobody can revoke it.

**Peering** — and I'm using that word deliberately, because it already means
exactly the right thing. In internet engineering, *peering* is the
relationship between networks that connect as **equals**: settlement-free, no
money changes hands, because neither one is above the other. Its opposite is
*transit* — the relationship where you're a customer. The internet's
backbone already runs on peering. The giants peer with each
other — *transit is what they sell to the rest of us.* So ingredient two is
simply: give every node the relationship the backbone reserves for itself.
Your home router peers. A Raspberry Pi peers. A phone peers. Nobody in this
network is anybody's customer.

And the third ingredient: **convergence**. You've all seen `conflicted copy
(2)` show up in your Dropbox. That's sync hitting a question it can't answer:
*whose version wins?* — and punting it to you. Convergence is sync that always has the answer.
Every node computes the same winner, independently, by math. And this is an example of what I mean by a symmetric protocol. Everyone does the same operations and arrives at the same answer, together. Split the
network in half, let both halves live separate lives for a week, rejoin them —
they converge. No referee, no merge conflict.

**Keys, peering, and convergence.** No blockchain, no
speculative asset in the path. A checklist — three questions
you can point at any system that stands between people, that intermediates: *Who mints your
identity? Are you a peer, or a customer? And who decides whose version wins?*
Every centralized service is an answer to those three questions. So is
everything I'm about to show you.

For the next half hour I'm going to show you how those three ingredients
dissolve every rung of that ladder — the address, the name, the trust, the
identity. But first I have to be honest about how I know where the ladder's
bolts are. I found each one by hanging from it.

*[→ beat 4, origin story.]*

---

### Delivery notes — beat 3

- **The ladder slide is the talk's home base.** Build it one rung at a time
  here; it comes back in each case study with one rung dissolving. Same visual
  vocabulary throughout: *granted → derived* or *granted → merged*.
- **"The problem is the shape, not the people"** is load-bearing for this
  audience — several of them *run* the authorities in question (registrars,
  CAs, ISPs, platforms). It keeps the room on your side and matches the
  no-enemy framing of the title.
- **Phone number rung gets the most dwell time** — it's the freshest example
  and lands personally with everyone. The other rungs can accelerate.
- **"Gave it a head"** deliberately plants the vocabulary for the Myanmar
  beat's "no head to cut off." Same image, first in irony, later in earnest.
- **web3 paragraph: keep it playful, not sneering.** One laugh line ("set the
  whole thing on fire"), then move immediately to the constructive claim.
  There will be chain people in the room; the posture is "we're claiming your
  promise," not "you were fools."
- **The peering-vs-transit beat is a gift to this audience** — they know the
  terms cold, and "transit is what they sell to the rest of us" reframes a
  dry BGP concept as the class structure of the internet. Let it land; it's
  the triad's teeth. ("Routing" was deliberately rejected here: it names
  plumbing, not a principle, and reachability appears in beat 7 as the
  *payoff* of peering instead.)
- **The three-question checklist is the take-home** — it's the portable
  method in pocket form (*who mints your identity? peer or customer? who
  decides whose version wins?*). It returns in the close (beat 11); consider
  a slide of just the three questions.
- **The triad's arc is worth feeling as you say it:** keys (what you *hold*)
  → peering (how you *relate*) → convergence (where you all *arrive*). The
  `conflicted copy (2)` hook is the whole CRDT pitch with zero jargon —
  "merge" was rejected because it only speaks to people who already know
  CRDTs or git; "consensus" because it's the blockchain word and technically
  wrong (CRDTs never run consensus).
- **The last line is the seam to beat 4** — it should feel like a confession
  coming, which buys the origin story its intimacy.
- **Timing:** ~6 min. If running long, the compression is the rent/
  surveillance/switch triplet — it can be two sentences instead of three
  paragraphs without losing the structure.

---

## Beat 4 — The road here: I built a gate by accident (~5 min)

*[Slide: one word — "IdentiKey". Register shift: slower, personal. This is
the confession the last line promised.]*

Some years ago I set out to build what I thought was the smallest ingredient
of the three. Just the keys.

The project is called IdentiKey: identity rooted in cryptographic keys
instead of accounts. The idea is old and quiet: **you are your key.** Not
"you are row 40,000 in someone's user table" — you are the holder of
something you generated yourself, and the network's only job is to verify
signatures. The right to *be someone* online without asking anyone's
permission to exist.

And to get something working on a deadline, I did the practical, sensible thing.

I stood up an OAuth server.

*[Let the room laugh — half of them have done exactly this.]*

And it worked! Login worked, tokens flowed, apps integrated. And it took me
an embarrassingly long time to look at what I had actually built. An issuer.
A place that says yes or no. A thing every login phones home to. **I had set
out to abolish the gatekeeper, and the first working thing I built was a
gate.**

I don't think this was stupidity. It's that every part on the shelf
is authority-shaped. Every spec, every library, every protocol you can reach
for was designed with authoritative model as a starting assumption. It's a kind of engineering shortcut, it solves the problem decently, you get your paycheck and never come back to it.  You
reach for "just verify who someone is" and your hand closes around a
certificate authority, a domain name, a relying-party server, a platform
vendor. You can't assemble those parts into anything but a gate. The
centralization isn't in the products. It's in the *parts*.

*[Beat.]*

Meanwhile — because apparently one impossible project isn't enough — I was
also building a decentralized compute system called Mjolnir: programs as
processes spread across many machines, cooperating with no central scheduler.
I had islands of these servers in different places, and I needed them to
reach each other, so I built some network plumbing to stitch the islands
together. That plumbing is why the code in this talk still says
`mjolnir-mesh` everywhere. It was supposed to be a weekend of glue.

*[Small beat. The turn.]*

Here's what those years taught me. Every time I got the identity design
close — really close —
I'd find I had quietly leaned on something. A CA being reachable. A DNS name
being trustworthy. A cloud database holding the canonical copy. And each
time, the censorship-resistance I was promising evaporated — not because the
crypto was weak, but because **the censor just moves to the assumption.** You
don't have to break a key if you can revoke a certificate. You don't have to
forge a signature if you can seize a server.

A key is *what you hold*. But holding means nothing if there's nowhere to
relate, nowhere to arrive. A key with no commons is a ticket to a theater
that doesn't exist (as we learned in crypto).

So the conclusion, when I finally accepted it: **I had to build the ground
first.** A network with no authority in it anywhere — not hidden, not
deferred, none. And to build *that* honestly, I had to build it somewhere no
authority could be assumed even by accident: radios that drop, nodes that
vanish mid-conversation, networks that split in half on a Tuesday, no
coordinator, no CA, no cloud. The mesh isn't a pivot away from identity. It's
identity's foundation, poured in the most adversarial soil I could find — on
purpose. Because anything that grows there doesn't need anyone's permission
to keep existing.

The weekend of glue became this project. Let me show you what came
out of it.

*[→ beat 5, the move.]*

---

## Beat 5 — The method (~5 min)

*[Slide: three words — keys · peering · convergence.]*

What came out is a method with three commitments. You've met them as words;
here's what each one costs as engineering — and what it buys.

**First commitment: every node runs identical software.** There is no
controller build, no server edition, no admin mode. This is peering made
real, and it's stricter than it sounds. It's not that we avoid electing a
leader — it's that there is no role a leader could occupy. The moment your
design includes one special node, you've built a throne, and history says
somebody sits in it. Symmetry is how you make sure we're all on equal footing.

**Second commitment: shared state converges by math.** Every node keeps a
small database — who's on the mesh, which addresses are claimed, what names
mean. Nodes trade entries with their neighbors, the way gossip moves through
a town. No node ever has the whole truth first; every node ends up with the
same truth eventually.

The interesting case is conflict. Two nodes, out of contact, claim the same
name. When the networks touch again, who wins? Every entry carries a stamp:
the writer's wall-clock time, a counter, and the writer's public key. You
compare stamps the way you compare words in a dictionary: earlier clock wins;
if the clocks tie, the counter breaks it; if those tie, the key breaks it —
and keys are unique, so there is always exactly one winner. Any node can run
that comparison — this week, next month, on another continent — and gets the
same answer. "Who decides?" turns out to have the most boring possible
answer: *everyone, identically.* The literature calls this a CRDT with a
hybrid logical clock. The whole trick fits on one slide, and it replaces the
referee.

**Third commitment: nothing is allocated.** Anything that must be unique — an
address, a name — is either derived from a key or claimed-and-converged. An
allocator is an authority with a spreadsheet, so the method forbids it: if
something needs handing out, redesign it until it doesn't.

*[Beat. Slide: an empty config file.]*

And here's what the three commitments buy, together. Think about what
configuration *is*. Every line of network config you've ever written points
at an authority: which box is the DHCP server, what range it hands out,
which controller to enroll with, which CA to trust. **Configuration is the
paperwork of authority.** Remove the authorities and the paperwork doesn't
get easier — it disappears. Plug a node in: it mints its identity, claims its
addresses, gossips what it knows, converges with everyone else. Unplug it:
the mesh adapts. Nothing was set up, because there is nothing to set up. And
notice what that means for the network as a whole: it grows by the plugging
in. Joining doesn't just *use* the commons — joining is what the commons is
made of.

*[Beat. Slide: 1974 — a photo of the SRI packet radio van, if you can get
one.]*

I want to be clear that none of this came from nowhere. In 1974, Vint Cerf
and Bob Kahn had a pile of networks that couldn't be unified — leased lines,
satellite links, literal radio vans driving around the Bay Area — and their
answer was: don't unify the links, unify the layer above them. They called
it the catenet. It's the only network design that has ever scaled five
orders of magnitude, and it's the shape we build.

And for the last twenty years, community networks kept that flame when
almost nobody else did — Freifunk, Guifi.net, NYC Mesh, the CeroWrt folks.
They ran collaboratively-owned networking at real scale, hit the walls
first, wrote down what they learned, and converged in the field on this same
shape: small link islands, stitched by routing, owned by the people on them.
Some of them are in this room. We didn't discover this territory — we
started from your maps, with gratitude. And meshes built on these
principles can interconnect; making our networks complementary is work
we're doing right now.

So: three commitments — identical software, convergence by math, nothing
allocated. Now let's climb back down the ladder from the beginning of the
talk and dissolve it, rung by rung. Starting with the address.

*[→ beat 6, DHCP.]*

---

### Delivery notes — beat 5

- **This beat is the engineering restatement of the triad** — beat 3 sold
  the words, this one shows the machinery. Don't re-explain peering-vs-
  transit or the conflicted-copy hook; they're already paid for.
- **The stamp-comparison walkthrough is the one place the talk goes
  algorithm-deep.** Keep it at dictionary-comparison level; say "CRDT" and
  "hybrid logical clock" only *after* the mechanism is understood, as the
  name for a thing the room already gets. Resist adding merge-rule detail —
  beat 7's liveness dragon is the second (and last) dip into internals.
- **"Configuration is the paperwork of authority"** is the beat's coin. The
  empty-config-file slide should sit silent behind it.
- **"Joining is what the commons is made of"** — the organizing principle,
  stated in passing, not announced. It gets its full weight later (beat 9
  and the close).
- **The Freifunk/Guifi/NYC Mesh paragraph is spoken TO people in the room.**
  Look at them. "We started from your maps" only works if it's clearly
  meant. Keep the complementarity line to one sentence — a promise of work,
  not a promise of results.
- **Timing:** ~5 min. Compression: the catenet paragraph can lose the van
  (sadly) and run one sentence if needed.

---

## Beat 6 — First rung: the address (~6 min)

*[Slide: the ladder returns, rung 1 highlighted — "address: granted".]*

DHCP. The protocol that answers your device's very first question: *may I
have an address?*

Here's how it works everywhere today: one box on the network holds a config
file and a lease table. Your device broadcasts into the dark, that box
answers, and whatever it says, goes. That box is the network's memory —
every device's existence is a row in its table — and it's the
adjudicator: it decides who gets what.

Anyone who has ever set up a network knows the feeling this produces:
somewhere in the mysterious stack, something isn't configured right — and
nothing works at all. No error, no sign or symbol. Just
silence, and an evening gone. Most of that mystery traces back to grant
points: some authority, somewhere, that didn't get told the right thing.
DHCP alone supplies a whole catalog — the home router plugged in backwards,
quietly handing out addresses to half an office; the two networks that both
grew up as `192.168.1.x`, so joining them means one side renumbering its
entire world.

That last one points at a general truth: **authorities don't merge.** Two
adjudicators with overlapping jurisdiction can't compose into one network —
one of them has to lose, and a human has to step in and negotiate the
treaty. Every seam between networks is painful *because* there's an
authority on each side of it. That's not a DHCP bug. It's what authority
costs.

*[Beat. Slide: rung 1 dissolving — "address: claimed & converged".]*

So here's the move, applied for the first time. Delete the server. Don't replace it.

In the mesh, each router claims its own block of addresses — a /24 of its
own — and writes that claim into the shared state, the same gossiped
database from a few minutes ago. The claim spreads node to node. Every
router ends up knowing every block: who claims it, stamped how. And if two
routers ever claim the same block — two networks that grew up apart, meeting
for the first time — nobody panics and nobody negotiates. It's just a
conflict in the database, and we already know exactly what happens to those:
the stamps get compared, everyone computes the same winner, and the loser
derives a new block and moves on. Automatically. In seconds.

We actually derive each router's claim from
a hash of its public key, so collisions are rare to begin with. But that's a
convenience, not the mechanism. Random claims would work fine. **The
convergence is doing the real work** — the hash just makes the common case
quiet.

And look at what falls out. Plug in a router: it addresses itself, claims
its block, starts routing. No DHCP range to plan, no address spreadsheet, no
authority to designate — the config file from the empty-config slide stays
empty. Unplug it: the mesh adapts. And plug two *whole meshes* into each
other — two networks with separate histories — and the seam heals. Claims
flow across, conflicts converge, routes stitch. Because claims merge, and
authorities don't. Networks compose now.

One more thing falls out, quietly: every router owning its own block means
every router owns its own segment. A misbehaving device's broadcast chatter,
an ARP spoofer, somebody's cursed IoT gadget — the blast radius ends at the
segment boundary, structurally. Containment isn't a firewall rule someone
maintains. It's the shape of the network.

That's rung one: the address, granted by a server, becomes the address,
claimed and converged. Next rung up is the name — and with it, the question
of how you find anything at all.

*[→ beat 7, reachability & naming.]*

---

### Delivery notes — beat 6

- **The frustration is named, not dramatized.** One recognizable feeling
  (something in the stack isn't configured, nothing works, no arrow points
  at why) plus two quick examples — a nod from the room is enough; this
  rung's job is to set up "authorities don't merge," not to be a story.
- **"Authorities don't merge. Claims do."** is the beat's coin — it's also
  the organizing principle wearing engineering clothes (two commons that
  meet become one; two authorities can't).
- **"Delete the server. Nothing replaces it."** — say it flat. The
  temptation is to soften with "essentially" or "in a sense." Don't; it's
  literally true.
- **The hash-derivation honesty matters** — this audience will ask about
  collisions in Q&A anyway; naming it preemptively ("a convenience, not the
  mechanism") buys credibility for every claim after it.
- **Ladder visual convention starts here:** rung 1's label morphs
  "granted" → "claimed & converged." Rungs 2–4 repeat the pattern; keep the
  animation identical each time so the method feels mechanical.
- **Timing:** ~6 min. Compression: the blast-radius paragraph is one
  sentence if needed.

---

## Beat 7 — Second rung: the name, and being reachable at all (~9 min)

*[Slide: the ladder, rung 2 highlighted — "name: granted".]*

The name rung is really two problems wearing one coat. A name has to *mean*
something — that's the directory problem. And the thing it points to has to
be *reachable* — and on today's internet, that second part is quietly broken
for almost everyone.

Think about what it takes, today, for you to run something as small as a
photo album for your family. Your computer sits behind a NAT, maybe two.
It has no address anyone can dial. So you either become a network wizard —
port forwarding, dynamic DNS, TLS certificates — or you pay the problem to
go away: a VPS, a tunnel service, somebody's cloud. An entire industry
exists to rent you back the ability to be reached. Not to host anything —
just to be *reachable*. That's transit-thinking all the way down: you, at
the bottom of the ladder, asking the network for permission to answer a
phone call.

Here's the other way, and it's the piece of this stack everyone can build on right now. It's called Iroh, and it's open
source and usable in your projects today.

**Your public key is your address.** Not "your key maps to an address" —
the key *is* the address. You dial a key. And the connection figures out
how to get there: same room, it goes over the wire directly; across the
mesh, it hops the radios; across the world, it traverses the NATs and rides
the open internet — an encrypted QUIC connection, end to end, between two
identities. The routers in between, mine, yours, a café's — they carry
ciphertext they cannot read. You get a trusted connection over hardware
nobody has to trust.

Notice what that means for the question "where are you?" It stops having a
network answer. Behind a NAT, on a mesh island, on fiber in a datacenter —
those become the connection's problem, solved underneath you. Dial the key;
the dial is identical in a disaster zone with no internet and on a gigabit
line with full internet. One system for both worlds — the abundant one and
the deprived one — which is the property everything later in this talk
stands on.

And in case anyone's waiting for the routing lecture: inside the mesh,
routes are computed by Babel, a lovely boring protocol older than some
people in this room. Routing turned out to be a commodity. The parts worth
building were never the routing.

*[Slide: rung 2 dissolving — "name: claimed & converged".]*

So reachability comes from keys. But nobody wants to dial a key, any more
than they want to memorize a phone number times four. Names.

You can already guess what we didn't build: a DNS server would be an authority,
names are just entries in the same
gossiped database as the address blocks. When the demo box joined this
morning, it wrote a claim: this name, this key, this stamp. The claim
spread router to router; now every node resolves it locally, from its own
copy. No name server to run, no registrar to pay, nothing to seize. Split
the mesh in half and both halves keep resolving every name they knew;
rejoin, and the claims converge like everything else.

Honesty about the frontier: today a name belongs to whoever claims it first
— first-writer-wins, arbitrated by the stamps. For a household or a
neighborhood mesh, that's plenty. The upgrade path is the interesting part:
because every claim is signed by a key, names can accumulate *reputation* —
vouches, webs of trust — so "first to claim" matures into "first
*legitimate* claimant." And notice that's the same machinery for a router,
a service, and a person. One identity method, all the way up. That's the
next rung, and we'll get there in a minute.

*[Point back at the presence page — the liveness dots.]*

One more piece of this rung: the little dot that says who's here *right
now*. It looks trivial, and it hides a genuinely hard problem.

Our shared database stores facts — this key claims this name, this stamp.
Facts merge; that's the whole trick. But "Alice is here right now" is not
that kind of fact. It's true, and then it silently *stops* being true, and
no event marks the moment. There's nothing to merge, because absence
doesn't write. You cannot gossip your way to "she's gone."

So presence doesn't live in the database at all. It rides a separate,
deliberately forgetful channel: tiny beacons, sent often, never stored,
never relayed, never merged. And each node judges staleness by its *own*
clock — I trust your clock to order your writes, but I will never use your
clock to decide whether you're still breathing. Durable truth in one plane,
perishable truth in the other, and neither pretends to be the other. Most
of the painful bugs in distributed systems come from mixing those two up.

*[Beat. Status, plainly.]*

Where this stands in the real world: the shared database and the address
book have been running on a real router fleet for months. One field report:
a router was powered off through an entire fleet software
update. It came back days later, gossiped with its neighbors, and converged
in seconds. Nobody noticed. Nobody had to. The names and services layer is
younger — what you're using tonight is it.

And that closes the loop on the opening demo. The box I plugged in this
morning: its name was a claim that gossiped, its reachability came with its
key, and the internet's presence or absence never entered into it.
**Reachability stopped being a product you subscribe to and became a
property of joining.** That's rung two.

*[→ beat 8, identity.]*

---

### Delivery notes — beat 7

- **This is the beat where the talk pays its cold-open debt** — the last
  paragraph should feel like a lock clicking shut. Don't add material after
  "property of joining"; leave on it.
- **The iroh section is deliberately evangelical** — "everyone can build on
  right now" is meant literally. This is promulgation, not product
  placement; the room should hear "this layer exists and you can have it."
  Accuracy check on the lineage line: iroh is built by **n0** (number 0),
  and they famously *moved away from* libp2p/IPFS rather than descending
  from it — "from the libp2p lineage" may draw a correction from this
  audience. Safer: "from the folks at n0" or "born out of the IPFS world."
- **"The key *is* the address"** — the em-dash correction ("not maps to —
  is") carries the whole idea; keep it slow.
- **The Babel aside is one breath of comic relief** and quietly answers
  "why isn't routing in your triad." Don't expand it.
- **The liveness passage is the second and last internals dip** (beat 5's
  stamp comparison was the first). The line doing the work is "absence
  doesn't write." "Whether you're still breathing" foreshadows the Myanmar
  register by a hair — deliberate; don't heighten it further.
- **TOFU honesty stays** — naming the first-writer-wins limitation before
  the audience does is what lets the web-of-trust line read as a roadmap
  instead of a dodge.
- **Timing:** ~9 min. Compression: the Babel aside and the field report can
  each drop; the liveness passage and the closing loop cannot.

---

## Beat 8 — Top of the ladder: trust, and being someone (~8 min)

*[Slide: the ladder, rungs 3 and 4 highlighted — "trust: granted" and
"identity: granted".]*

The top two rungs — trust, and identity — are really one rung viewed from
two sides. Both come down to the same question: *who vouches for you?*

Today the answers are: a certificate authority vouches for your server, and
an issuer vouches for you. Sign in with Google. Sign in with Apple. Verify
with a text message to — there it is again — your phone number. The
identifier you're reachable at is leased from a carrier, demanded by every
service as proof you're a person, and joined against every database you
touch. We built a world where *being someone* is a service: provided to
you, rate-limited, terms-of-service'd, and revocable. And the identifier at
the center of it doubles as the best tracking handle ever devised.

This rung is where the whole project started for me, so here is IdentiKey's
answer, and it fits in one sentence: **the protocol verifies only
signatures.**

Your identity is a keypair you generate yourself. Each of your devices
holds its own key, and your identity key signs a short statement — *this
device key acts for me* — that anyone can check. Checking it is pure math:
no issuer to call, no registry to consult, no home to phone. A router in
this room can verify that your phone speaks for you while the mesh is
split from the internet, from me, from everything — because verification
needs nothing but the signature and the key in front of it. That one rule,
held everywhere with no exceptions, is what it actually takes for "nobody
can switch your identity off" to be true rather than aspirational.

We did look hard at passkeys — they're real cryptography and a real
improvement. But a passkey is bound to a DNS domain and to a platform
vendor's account system, which is to say: to two authorities. Adopting
them would have welded the top of the ladder back onto the bottom. We
declined.

*[Beat.]*

Two consequences of "only signatures," and they're the ones I care about
most.

First: **anonymity is a first-class citizen.** A keypair minted for one
conversation and thrown away afterward is a complete, valid identity — not
a degraded mode, not a suspicious edge case. The protocol cannot tell the
difference between a throwaway key and a lifelong one, and that's by
design. You all did this, forty minutes ago: every identity on that
presence page was minted by a browser, no email, no phone number, no
CAPTCHA. From there it's a spectrum you climb by choice — keep the key in
your browser, move it to an app, put it in hardware, or hand custody to
someone you trust who runs an ordinary service on the mesh. Every rung of
custody looks identical to every service: a key, and valid signatures.

Second: **there is no registry.** This one is structural, and it matters
more than it sounds. Every identity system accumulates a database, and
every such database is a liability with a timer on it — because a registry
read backwards is a map of people: who exists, who talks to whom, who to
find. Registries change hands. Here, there is nothing to seize, because
identity was never *recorded* anywhere — it's verified pairwise, offline,
at the moment of use, and forgotten. An identity system with no panopticon
in it, because the panopticon was never built. Near the end of the talk
I'll tell you about the people who taught me how much this matters.

*[Beat. Slight smile — the payoff of the oldest setup in the talk.]*

Now. At the very start, I made you type `http://`, and your browser
refused to believe the site was real. Here's what that was.

Browsers gate their own cryptography — `crypto.subtle`, the good API, the
one with non-extractable keys — behind what they call a *secure context*,
which in practice means HTTPS with a certificate from an authority on the
browser's list. Refuse the certificate authority, and the browser turns
its crypto off. Think about the shape of that: the most
security-conscious API in the platform becomes unavailable at precisely
the moment you decline the landlord. On a mesh with no CA in reach, the
browser — your one universal client — shows up with its best tools
disabled.

We climb out with a ladder of our own. In the bare browser, we ship a
small, audited pure-JavaScript Ed25519 — a real key, held honestly: we
call it soft custody, and it's plenty for
saying hello. One rung up, a browser extension page *is* a secure context
— full hardware-backed WebCrypto, no CA involved. And between mesh-native
software, the problem doesn't exist at all: an iroh connection uses the
node's key *as* its TLS identity, so every hop you saw tonight was already
encrypted, mutually authenticated, and certificate-free. The gate is real,
the gate is annoying, and we can hop the fence.

*[Slide: the full ladder, every rung dissolved — "granted" struck through,
"self-created / claimed / converged" beneath.]*

Step back and look at what happened to the ladder. Your address: derived
from your key. Your name: claimed via your key. Trust: a signature check.
Being someone: holding a key. Every "granted" became "created" or
"claimed" — and the network, all of it, became a projection of a set of
keys. The physical substrate — which radio, which building, which
continent — is routing detail underneath the thing that actually matters:
*who*, cryptographically, is talking to *whom*.

*[→ beat 9, what falls out.]*

---

### Delivery notes — beat 8

- **The `http://` payoff is the beat's structural high point** — a setup
  planted in minute one, paid off in minute forty. Let the room enjoy the
  click before explaining secure contexts.
- **"The protocol verifies only signatures" is the beat's coin.** Every
  claim in the beat is a corollary of it; if a sentence doesn't trace back
  to it, cut the sentence.
- **Passkeys get one breath, no dunking** — "real cryptography, real
  improvement, two authorities, declined." Passkey advocates in the room
  should feel accurately described, not caricatured.
- **The no-registry passage seeds Myanmar without naming it** — "registries
  change hands" and the closing sentence are the seed; the checkpoint/ID
  story belongs to beat 10. Don't spend it here.
- **The anonymity callback to the demo** ("you all did this, forty minutes
  ago") is the cheapest proof in the talk — the audience is the evidence.
- **IPv6 stays in the Q&A pocket** (projection-of-keys is the answer if it
  comes up: everything IPv6 promised, the identity layer delivers; IP
  demotes to plumbing).
- **Timing:** ~8 min. Compression: the passkey paragraph and the
  extension-rung sentence can drop; the no-registry passage and the
  `http://` payoff cannot.

---

## Beat 9 — What falls out (~3 min)

*[Slide: blank, then one line at a time as each property is named.]*

Before the last part of this talk, I want to collect what the method
produced — because the striking thing is that none of these were features
we set out to build. Each one fell out of removing an authority, the way
silence falls out of turning off an engine.

**Nothing to configure.** Not simplified setup — no setup. Configuration
was the paperwork of authority, and the authorities are gone.

**Networks compose.** Two meshes with separate histories meet, and the
seam heals: claims converge, routes stitch. Growth has no admission
process, so the commons grows by whoever shows up.

**Partition is not failure.** Split this network in half, and both halves
keep working — resolving names, routing packets, serving the room — on
whatever they knew last. When the halves find each other again, they
gossip and converge, and no one resyncs against a primary, because there
is no primary. Most systems treat partition as the disaster case and hope
it's rare. Out here it isn't rare — a radio mesh partitions *constantly* —
so it's the normal case, engineered to be boring. A network that only
works while it's whole hasn't been tested yet.

**The network outlives the hardware.** Every link — a radio, a wire, a
QUIC tunnel — is plumbing under the identity layer. We've already retired
an entire radio generation from the fleet, and the mesh didn't notice.
Vendors sunset products; a projection of keys doesn't have a product line.

And the sum of the four: **nobody is in charge — and nobody is in a
position to *become* in charge.** Not because of governance, or a
foundation, or good intentions. There is no role to seize, no registry to
subpoena, no server to acquire. Sovereignty here isn't a policy. It's the
topology.

*[Slide: the principles, assembled — permissionless, self-sovereign,
disintermediated, symmetric, censorship-resistant, resilient... and a
seventh, dimmed, unnamed.]*

That's the vocabulary of this movement, earned one mechanism at a time —
and there's one word left on the list. I can't get to it through
engineering. I have to get to it through a story, and it's the one I
promised you at the beginning.

*[→ beat 10, Myanmar. Register drops here — take a breath, slow down.]*

---

### Delivery notes — beat 9

- **This beat is a collection, not a re-explanation** — every property was
  already earned; each gets two or three sentences, no more. The rhythm is
  the point: the room should feel the pieces click into one shape.
- **"The way silence falls out of turning off an engine"** sets the beat's
  quiet-consequence register; resist re-inflating with superlatives.
- **"A network that only works while it's whole hasn't been tested yet"**
  is the line that carries into Myanmar — it's the same thought that beat
  10 makes human. Deliver it plainly here; its weight arrives later.
- **The dimmed seventh principle on the slide** does the transition's work
  visually; don't name "locked open" yet — beat 11 owns it.
- **Timing:** ~3 min. This beat is also the talk's schedule buffer: if
  running long, each property can compress to its bolded line and one
  sentence.

---

## Beat 10 — Why it matters (~5 min)

*[No slide, or a plain dark slide. Stand still. Plain voice — no
performance in this beat at all.]*

Some of you know that people from Myanmar have been part of this community
for years. They come to DWeb working on a problem most of us have never
had to pose: how do you run the services a society needs — records,
coordination, communication — when you cannot use the main internet?

I've had the privilege of sitting with some of them. We worked together on
identity systems for a provisional government — which is where I learned,
concretely, why an identity system must not have a registry. In Myanmar,
being stopped at a checkpoint with the wrong ID can mean arrest. A
database of who people are, read backwards by the wrong hands, is a
weapon. When I said earlier that we never built the panopticon — that
design decision has names and faces attached to it.

But the first thing they taught me came before any of that. We kept
trying to start on identity, and the conversation kept returning to
something more basic: connectivity itself. Because in Myanmar,
disconnection is one of the weapons.

The military cuts the cell towers. Starlink is banned. ISPs are shut off,
region by region. An area is made to go dark, and then it is moved
through. And I heard what the dark is like from people who lived it: news
reduced to what someone can hand-copy onto paper and carry by motorbike,
hundreds of miles — which towns had burned, whether the soldiers were
coming toward you or turning somewhere else. Your village, and silence,
and waiting to find out.

Cutting people off from each other, to take them apart piece by piece, is
exactly the thing the internet was built to make impossible. That was the
founding requirement — survive the loss of your center. We spent forty
years giving it a head. And somewhere right now, that head is being held.

I want to be careful and honest here, because overclaiming would be a
disservice to exactly these people: a mesh node is not a shield against an
army, and I will never stand up here and tell you it is. The honest claim
is narrower, and it's still worth everything: **a network with no head to
cut off is harder to take away.** No tower whose removal darkens a region
— the mesh rides whatever link exists: a radio, a wire, one shared
satellite hop. No ISP to ban, because joining *is* reachability. No
coordinator to seize, no registry to read backwards, and when the network
is split — not if, *when* — both halves keep working, because we built
split-brain to be boring.

Resilience. Of all the principles on that slide, that's the one with
faces on it. Connectivity is a human right — not because a declaration
says so, but because being cut off from each other is one of the oldest
ways human beings are broken. A network worth building is one that cannot
be taken away this easily.

*[Hold one quiet beat. Then — the turn, into resolve.]*

*[→ beat 11, locked open.]*

---

### Delivery notes — beat 10

- **The register rule for this beat: witness, plainly.** No performance,
  no crescendo, no music in the voice. The material carries itself;
  anything added subtracts. One image (the motorbike), told once, with
  restraint — do not pile on detail.
- **Land it, hold one beat, then move.** The turn out is resolve, not
  grief — straight into "locked open." No product slide after this beat,
  no demo callback, no ask, no pledge. (Per the direct conversation:
  nothing here is secret; the one rule that remains is *don't linger on
  the trauma*.)
- **"We spent forty years giving it a head. And somewhere right now, that
  head is being held."** — the beat 3 irony ("gave it a head") returns in
  earnest. This is the sentence to know cold.
- **The checkpoint/registry passage completes beat 8's seed** — "that
  design decision has names and faces attached to it" is the whole
  connection; don't re-explain the mechanism.
- **The honesty paragraph is load-bearing** — "not a shield against an
  army" protects the people invoked *and* the talk's credibility. Keep it
  even under time pressure.
- **Timing:** ~5 min spoken slowly. This beat does not compress; take the
  time from beat 9 if needed.

---

## Beat 11 — Locked open (~4 min)

*[Slide: the principles return. The seventh word lights up: **un-ownable —
locked open.** Energy returns to the voice here — resolve, then invitation.]*

So here's the last word on the list, the one the title of this talk comes
from.

Everything I've shown you tonight serves one design goal, and it was never
a technical one. **Make it fundamentally not ownable.** Locked open — and I
mean that phrase precisely. Not open the way a product is open, where
somebody chose to open it and somebody could choose otherwise later. Open
the way a mathematical fact is open. There is no center to buy. No center
to seize. No center to subpoena, pressure, acquire, or slowly bend toward
extraction — because there is no center at all, and nothing in the design
allows one to form. The shortcut of authority always leaves a throat, and
history says someone eventually grips it. So we did the harder engineering,
all the way down, to build a thing with no throat.

That's my definition of the decentralized web, for whatever it's worth —
not centralized services with the logo filed off and a token bolted on,
but systems whose **correctness does not route through anyone's
authority.**

*[Beat. Now hand it over.]*

And the method is yours. Three questions, for whatever you're building or
using or depending on: *Who mints your identity? Are you a peer, or a
customer? Who decides whose version wins?* Wherever the answer is "an
authority" — that's not a fact of nature. It's a design decision, and it
can be redesigned. Every "granted" you find can become something created,
or claimed, or converged. It costs real engineering — you've seen tonight
roughly what it costs — and what it buys is the only guarantee that
survives every owner, every acquisition, every regime: there's nothing
there to take.

Because the ways we coordinate — the group chat, the shared document, the
directory, the map of which towns are safe — this is necessary
infrastructure now, as necessary as water. And necessary infrastructure
cannot live in the hands of anyone whose incentive is to own it. Build it
so it can't be.

I should say: half the ideas in this talk got sharp inside this community
— people in this room pushed on them, lent me theirs, told me where I was
wrong. That's what a commons does.

*[Beat. One more admission — quieter, then done.]*

And one thing about ownership, since the whole talk is about not having any.
This project used to belong to my company. As of this week, it doesn't. I
signed it over to a foundation — the World Tree Network Foundation — that
exists to hold it and nothing else. Same reason as everything else tonight:
a thing that's locked open can't have *me* as its throat either. No center
to seize, and now no owner to pressure.

And one last thing about the network you all made at the top of this hour.
It's still running. It'll be running when I walk off this stage — and it
isn't mine to turn off. A network is created by the people who join it.
That was true in this room tonight, and there's no reason it can't be true
everywhere.

Come find me — the mesh is an invitation, and it merges.

Thank you.

*[Slide: title. Then Q&A.]*

---

### Delivery notes — beat 11

- **The seventh word lighting up is the title's payoff** — the phrase
  "locked open" has been on screen unexplained since minute five. Let the
  slide land before speaking.
- **"No throat" is the one permitted callback to violence in this beat** —
  it inherits its weight from beat 10 without re-invoking it. Say it and
  move.
- **The three questions are spoken slowly, as a gift, not a summary** —
  this is the empowerment payload; the room should be able to write them
  down.
- **"As necessary as water"** keeps the infrastructure claim concrete;
  resist expanding into a policy argument.
- **The community-gratitude line is spoken to the room, like beat 5's
  Freifunk moment** — specific and warm, one breath.
- **The ownership admission is the thesis enacted, not announced** — spoken
  as a quiet aside, one breath, no applause line. It's the ownership-layer
  proof of "no throat": the speaker removes *himself* as the center. It
  sets up "it isn't mine to turn off" so that line now rhymes — not the
  code, not the network. If the talk is over time this is the first cut in
  the beat; the network callback must survive.
- **The last move is the demo, re-seen** — not a callback for charm, but
  the thesis made physical: the network exists because they joined it, and
  it outlives the speaker. "It isn't mine to turn off" is the whole talk
  in six words.
- **Q&A pockets ready:** IPv6 (projection of keys; IP demotes to
  plumbing), Recrypt/data-at-rest (proxy re-encryption — same method
  applied to data; future talk), routing security (route-origin validation
  via the identity layer — planned, honest frontier), Sybil/name squatting
  (TOFU today, web-of-trust arbitration next), scaling ceilings (full
  replication comfortable to thousands of routers; sharding is a fun
  future problem).
- **Timing:** ~4 min. Nothing here compresses well; if the talk is over
  time, the cut is in beats 3–6, never here.

---

### Delivery notes — beat 4

- **The OAuth confession is the beat's engine.** Deliver "I stood up an OAuth
  server" deadpan, as the reasonable act it was; the room's laugh is
  recognition, not mockery. The thesis line follows it: *built a gate by
  accident.*
- **"The centralization is in the parts"** is the beat's transferable insight
  — it generalizes the confession to the audience's own work and re-arms the
  ladder slide.
- **Mjolnir stays to one breath.** It exists here to (a) explain the repo
  name with a smile, (b) set up "the weekend of glue became the project."
  No π-calculus on stage — it's a Q&A pocket.
- **"The censor just moves to the assumption"** is the intellectual core —
  slow down for it. The two follow-ons (revoke a certificate / seize a
  server) make it concrete; don't add a third.
- **The commons line** ("a key with no commons is a ticket to a theater that
  doesn't exist") ties this beat to the triad's arc: keys are *hold*; beats
  5–7 build *relate* and *arrive*.
- **Timing:** ~5 min. Compression: the Mjolnir paragraph can drop to a single
  sentence inside the realization paragraph if needed.
