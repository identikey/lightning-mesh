# Lightning Mesh — Commercial License

Lightning Mesh is **dual-licensed**. You may use it under **either**:

1. The **GNU Affero General Public License v3.0 or later** (AGPL-3.0-or-later),
   the license in [`LICENSE`](LICENSE) — free of charge, forever; or
2. A **commercial license** from the Lightning Mesh maintainers, described
   below.

You only need a commercial license if the AGPL's obligations don't work for
your product. If you're happy to comply with the AGPL, you owe nothing and can
stop reading.

> **Legal entity note:** the commercial licensor is currently
> **Duke Jones** (`duke@worldtree.io`). If/when a formal company is
> incorporated to hold this project, replace "Duke Jones" throughout this file
> and in `CLA.md` with that entity's legal name.

---

## When you need a commercial license

The AGPL is a strong copyleft license. Under it, **section 5** requires that
anyone you convey a modified version to receives the complete corresponding
source under the AGPL, and **section 13** extends that to *network use*: if you
let users interact with a modified Lightning Mesh over a network, you must
offer them its complete corresponding source.

For most operators — community mesh networks (e.g. Freifunk-style collectives),
researchers, hobbyists, and any deployment willing to share source — the AGPL
is a perfect fit and is free.

You will likely want a **commercial license** if any of the following apply and
you do **not** wish to release your corresponding source under the AGPL:

- You embed `mjolnir-meshd` or any Lightning Mesh code in a **proprietary
  hardware appliance** or firmware image that you sell or lease (e.g. mesh
  gear sold to hotels, venues, or enterprises).
- You offer a **hosted / SaaS management plane** or cloud service built on
  Lightning Mesh, and cannot or will not disclose its source under AGPL §13.
- You **link or combine** Lightning Mesh with proprietary software in a way
  that would make that software a derivative work subject to the AGPL.
- Your legal/procurement policy **prohibits AGPL** software in shipped
  products (many enterprises ban it).

A commercial license grants you the same code under terms that **remove the
AGPL's copyleft and network-source-disclosure obligations**, so you can ship a
closed product.

## What a commercial license typically covers

Terms are negotiated per deal, but a standard commercial license grants a
non-exclusive, worldwide right to use, modify, and distribute Lightning Mesh
**without** AGPL source-disclosure obligations, in exchange for a fee. Common
structures:

| Model | Typical fit |
|-------|-------------|
| **Per-unit / OEM royalty** | Vendors shipping Lightning Mesh inside sold hardware appliances |
| **Per-site / per-deployment** | Venue, hotel, or campus operators running a managed fleet |
| **Annual subscription** | Ongoing use plus updates, security patches, and support |

A commercial license does **not** relicense third-party components (e.g.
`babeld`), which retain their own licenses; it covers the Lightning Mesh
source in this repository.

## How to obtain one

Email **duke@worldtree.io** with:

- your company and product,
- roughly how Lightning Mesh will be used (embedded appliance, hosted
  service, internal deployment, etc.), and
- expected scale (units/sites/seats).

We'll follow up with a quote and a license agreement.

---

*This document is a summary of the commercial-licensing offer, not the
commercial license agreement itself. The binding terms are in the signed
agreement provided at purchase. Nothing here modifies your rights under the
AGPL, which remain available to everyone.*
