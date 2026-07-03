# Archived Design Docs

Superseded or historical design documents, kept for reference. Each file carries a
dated status banner explaining why it was archived and what replaced it. Nothing in
this directory describes the shipped system — for that, start at
`docs/network-coordination/network-architecture.md`.

## network-coordination/

The original shared-L2 / unified-DHCP design trio, archived 2026-07-02:

- `mesh-network-coordination.md` — superseded overview; the shared-subnet model was
  replaced by per-node routed /24s with babel routing over the 802.11s backhaul.
- `dhcp-crdt.md` — the subnet-claim CRDT shipped as designed
  (`crates/mjolnir-mesh/src/crdt/`); the lease/DHCP/deauth machinery was never built.
  Design reference for the service-mesh phase (bead `e21`).
- `dnsmasq-integration.md` — never implemented; shipped reality is stock OpenWrt
  dnsmasq steered by UCI reconciliation of the claimed /24. Design reference for `e21`.

## mikrotik-container/

The retired RouterOS-container deployment track (MikroTik nodes running the mesh
daemon in a container). Retired 2026-07-02 along with the AP/STA radio architecture —
the fleet standardized on OpenWrt mt76 hardware with native 802.11s backhaul.
