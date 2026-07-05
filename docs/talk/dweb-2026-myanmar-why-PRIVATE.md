<!--
  ██ PRIVATE — SOURCE REFLECTION — NOT FOR PUBLICATION ██
  Do NOT include this file (verbatim or paraphrased) in the public website build,
  slides, or any published doc. It holds the rawer account behind §11 of the
  personal-narrative talk and the opsec/ethics guardrails for using it. The
  stage-ready version lives in dweb-2026-personal-narrative.md §11. When in doubt,
  the concrete, witnessed, present-tense reality is shareable; sweeping history,
  graphic detail, and anything that could identify or endanger a person is not.
-->

# The Why Behind the Why — Myanmar (PRIVATE source note)

**Status:** PRIVATE source reflection — **not for publication** | **Feeds:**
[personal-narrative §11](dweb-2026-personal-narrative.md) | **Date:** 2026-07-04

This is the fuller version of the thing that reordered my priorities, kept out of
the public track on purpose. Two reasons: some of it is too raw for a tech-talk
stage, and some of it could get people hurt if it's said carelessly. Both matter.

---

## 1. What happened, in full (for me, not for the stage)

I was invited to a small, private gathering — people building distributed systems
around identity, technologists working on civic/government-grade services, and
several anonymous refugees from Myanmar. We spent days drilling into what they are
living through. The brutality of it. I genuinely had no idea it was this bad.

*(Specific affiliations of the other attendees are deliberately omitted, here and
everywhere — the smaller the identifying surface, the safer everyone in that room.)*

The way I came to understand it: this is the modern face of a very long domination
of the ethnic peoples — centuries of it — now carried by a military junta with a
full surveillance-state apparatus behind it. One of the deliberate methods of
warfare against the ethnic groups is to **eliminate their connectivity**: cell
towers cut down, Starlink banned, ISPs shut off. A region is made to go dark, and
then it is moved through.

What that does to a life is the part I can't unhear. People hand-copying zines and
carrying them by motorbike across hundreds of miles to move the only news there is —
which towns had burned, whether the soldiers were coming toward you or turning
somewhere else. The pit-of-the-stomach of not knowing. No way to reach anyone — not
the outside world, not the next valley — just your village, huddled together in the
dark, waiting to find out.

Cutting people off from one another to take them apart piece by piece is exactly the
thing the internet was supposed to make impossible. And here we are. That is when
"communication is a human right" stopped being a slogan and became the reason I
can't put this down.

## 2. The opsec / ethics guardrails (read before adapting §11 for anyone)

These are non-negotiable when any of this goes near an audience, a slide, a
recording, or a webpage.

1. **Protect the people, absolutely.** The refugees were anonymous *for a reason*.
   Never name a person, a village, a route, an organization, a date, or a specific
   method that could identify or endanger anyone. No photos, no quotes attributable
   to an individual, no "I met someone who…" that narrows the field. If unsure
   whether a detail is identifying, cut it.
2. **Don't hand the adversary a manual.** Do not publicly detail *how* connectivity
   is being restored on the ground, which hardware, which links, which crossings,
   which frequencies, which people carry what. The junta reads conference talks too.
   Talk about the *architecture's properties* (no tower to cut, no ISP to ban),
   never operational specifics of any real deployment.
3. **Witness, don't appropriate.** Frame it as *I was invited, I listened, I was
   changed, here is what I can do* — never as spectacle or as speaking *for* people
   whose story isn't mine. Their suffering is not a rhetorical device to sell nodes.
4. **Get consent for the framing.** Before this goes on a public stage, confirm with
   the gathering's organizers / the people involved that they are comfortable with
   the mesh project invoking their situation at all, and how. They may want specific
   words used or avoided. Honor that over anything written here.
5. **Keep the history light in public.** The centuries-of-domination framing is real
   but sweeping, contested in its details, and easy to get wrong or to reduce.
   In a short talk it invites derailment and risks sounding like I'm claiming
   expertise I don't have. Keep the *witnessed, present-tense, undeniable* reality
   (towers, Starlink, ISPs, the motorbike zines) on stage; leave the deep history
   for a longer form written with people who actually hold it.
6. **Don't overclaim the tech.** A mesh node is not a shield against an army. Say so.
   The honest claim is narrow and still powerful: connectivity with no single throat
   to choke is *harder to take away*. Overclaiming is both untrue and unsafe (it can
   raise expectations that get people hurt).
7. **No fundraising pledge on stage (for now).** An earlier draft promised "a
   portion of every node sale funds connectivity for Myanmar." Pulled: a pledge is a
   commitment, not a talk beat, and it can't be spoken until the percentage,
   recipient, accounting, and — hardest — a *safe* delivery path that exposes no one
   are all real. If it ever becomes a pledge, it goes through the same guardrails
   above. Until then the humanitarian point stands on witness and architecture
   alone, not on an ask.

## 3. Rawness triage — what's stage-safe vs. what stays here

| Element | Stage (public) | Stays private / longer-form |
|---|---|---|
| "Connectivity is a human right" thesis | ✅ keystone | |
| Towers cut / Starlink banned / ISPs shut off | ✅ concrete, witnessed, undeniable | |
| Zines by motorbike; "which town burned, are they coming toward you" | ✅ one image, told with restraint | avoid piling on more |
| The pit-of-the-stomach feeling, briefly, in first person | ✅ one honest sentence | don't dwell / catalog |
| Centuries-of-enslavement historical framing | ⚠️ one light clause at most | ✅ full context, written with those who hold it |
| Days of graphic detail about the brutality | ❌ | ✅ here, and even here kept non-identifying |
| Any identifying detail about people/places/methods | ❌ never | ❌ never (not even here) |
| Affiliations of other attendees | ❌ never | ❌ omitted even here |
| A fundraising pledge / ask | ❌ pulled for now | ✅ only if/when real + safe |

## 4. Delivery note (pacing)

This is the emotional peak of the personal-narrative talk. Structurally it sits at
§11, right after "split-brain is the whole test" and right before "locked open."
Land it, let it be quiet for a beat — then **move**. Don't linger, don't milk it,
don't follow it with a product slide. The turn out of it is the vow: *this is why
un-ownable infrastructure matters* — straight into "locked open." Grief into resolve,
then off the stage. No ask, no pledge, no fundraising turn — that would cheapen it.
If it becomes trauma tourism it fails both the audience and the people it's about.

## Related

- [Personal-narrative talk §11](dweb-2026-personal-narrative.md) — the stage-ready
  version this note feeds.
- [Technical-arc source](dweb-2026-technical-arc.md) — the architecture properties
  (no tower, no ISP, no CA, split-brain-survives) that are the countermeasures.
