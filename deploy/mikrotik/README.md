# deploy/mikrotik — retired RouterOS-container track

The scripts and artifacts in this directory belong to the **retired**
RouterOS-container MikroTik track (beads ut9/ns1/ecd closed): running
`mjolnir-meshd` inside a RouterOS container with an AP/STA radio backhaul.
That architecture was superseded by the native-OpenWrt fleet — 802.11s
backhaul, CRDT /24 claims, babel — which is the live deployment path:
see `deploy/openwrt/README.md`.

Everything here is kept for historical reference only. The accompanying
docs were moved to `docs/archive/mikrotik-container/`.

If MikroTik hardware is ever revisited, the path is native OpenWrt on the
hardware, not RouterOS containers — see the `deploy/openwrt/l23-port/`
recon.
