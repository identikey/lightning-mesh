# Node Operations: Management Plane, In-Band Updates, OTA

Status: living design note, 2026-07-01. Captures the node-management design
worked out alongside the single-overlay-TUN fork (`buw`). Implementation:
`deploy/openwrt/install-node.sh` + `deploy/openwrt/files/usr/sbin/mjolnir-apply`
(bead `mjolnir-mesh-6e5`); trajectory: `mjolnir-mesh-0kx` (OTA over the
overlay), `mjolnir-mesh-buw.9` (CRDT peer announcement).

## The intent

A mjolnir node is meant to be an **appliance**: someone gets a box, plugs it
in, and it joins whatever is around it — the 802.11s mesh locally, iroh
tunnels over any internet egress it can find. That only works as a product if
the box can also be **operated** without physical access:

- the **owner** can securely configure their node and receive updates
  remotely (IdentiKey key-based authorization — implementation pending);
- the **maintainers** of a fleet can push binaries and config to every node
  without walking an ethernet cable to each one.

The failure mode this design retires: every routine update used to require
plugging into the node's LAN port at `192.168.1.1`, because the update itself
(the `wpad-basic`→`wpad-mesh` swap, `setup-wireless.sh`) bounces the very
radio you'd be SSHing over.

## Principle 1: the overlay is the management plane

Every node already has a unique, stable, **derivable** address:
`10.254.<blake3(node_id)>` on the backhaul, in the shared `10.254.0.0/16`,
routed everywhere by babel. That address space *is* the management plane:

- `ssh root@10.254.x.y` works from any device on the mesh (the mesh
  interface sits in the `lan` firewall zone), across multiple radio hops,
  and — once the `buw` overlay carries cross-site traffic — across the
  internet.
- The address is computed forward from the node id known at enrollment, not
  discovered. A maintainer's static inventory (name → node id → `10.254.x`)
  is sufficient tooling today.
- Ethernet at `192.168.1.1` is demoted to **recovery of last resort**. It is
  the one address space where every node collides, and it requires physical
  presence — the opposite of the appliance story.

## Principle 2: discovery is the CRDT, not mDNS

For *finding* nodes dynamically (liveness, human names, cross-site), the
answer is the gossip/CRDT peer announcement (`buw.9`): node id → name →
overlay address → last-seen, queryable locally (`mjolnirctl nodes` shape).
Management/fleet inventory is a second consumer of that CRDT, alongside
cross-site dialing.

mDNS is explicitly **not** the answer for management discovery: it only
floods the flat-L2 island, doesn't cross `br-lan`/`br-mesh` without
reflectors on every node, and dies entirely in the L3-routed future
(`mesh_fwding=0`, `0yb`) and across sites. mDNS remains what it already is —
a link-local bootstrap signal — nothing more.

## Principle 3: updates are staged, detached, and self-healing

You cannot depend on a live session through the interface you are replacing.
So updates never do: they use the commit-confirm pattern (Cisco
`commit confirmed`, LuCI's uci rollback), implemented in
`deploy/openwrt/files/usr/sbin/mjolnir-apply`:

1. **Stage** everything first, non-disruptively: binary, init scripts,
   config, and **prefetched packages** — both wpad variants, so even
   rollback needs no internet mid-apply. A local `pkg-cache/` covers boxes
   with no WAN at all.
2. **Apply detached** (`setsid`): the operator's SSH session dying during
   the wifi bounce is expected and harmless.
3. **Health-gate**: after a disruptive apply, a mesh peer or a pre-apply
   `10.254.x` neighbour must answer within a timeout, or the node **rolls
   itself back** (configs, previous binary, previous wpad) and comes back up
   on the old config. Either way the operator SSHes back in; a result file
   reports `OK` / `ROLLED_BACK` / `FAILED`.
4. Applies are **idempotent**: a re-run on an up-to-date node touches
   nothing and never bounces wifi.

Mechanics and runbook: `deploy/openwrt/README.md`.

## Trajectory: OTA updates over the overlay (`mjolnir-mesh-0kx`)

The applier was written to become the OTA agent. The delta from here to
"boxes in the field update themselves":

- the payload arrives over an **iroh stream** instead of `scp` — same
  staging dir, same applier, same health gate;
- payloads and management actions are **signed and authorized via
  IdentiKey** key-based auth (the current blocker — implementation
  incomplete);
- nodes advertise version/health through the CRDT peer announcement
  (`buw.9`), so the fleet inventory is live rather than hand-kept;
- the maintainer's local node list remains the bootstrap/recovery inventory.

The through-line: **the mesh manages itself over itself**, with physical
access only as the escape hatch.
