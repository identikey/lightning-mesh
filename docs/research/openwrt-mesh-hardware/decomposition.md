# Decomposition — OpenWrt True-Mesh Hardware Selection

**Question:** Find open-source-friendly hardware for a TRUE wireless mesh (802.11s/IBSS) deployment running OpenWrt, with confirmed mainline support, strong mt76 (mt7915/mt7916) or ath9k/ath10k radio drivers, ≥2 radios (tri-radio ideal), compact/deployable, external antenna connectors, and a bare-board/SBC form factor for custom enclosures.

**Mode:** research · **Depth:** 2 · **Max branches:** 5

## Ranked Hypotheses

### H1 — Ready-made bare-board mt76 (mt7915/mt7916) WiFi-6 mesh routers exist with external antenna connectors (HIGH plausibility, HIGH info-value)
There are commercially available OpenWrt-target boards built on MediaTek MT7981/MT7986 (Filogic 820/830) SoCs paired with mt7915/mt7916 radios, sold as bare PCBs or in openable cases, with U.FL/IPEX pigtails to external RP-SMA. Investigate concrete models, radio counts, bands, connectors, price.
- investigation_type: web

### H2 — ath9k/ath10k boards remain the gold standard for rock-solid 802.11s/IBSS, at the cost of WiFi-5/4 only (HIGH plausibility, HIGH info-value)
The most battle-tested mesh drivers are ath9k (802.11n, IBSS+mesh flawless) and ath10k (802.11ac). Investigate currently-buyable bare boards/SBCs using these (e.g. ath9k mPCIe combos, legacy AP boards) and whether the maturity tradeoff is worth it vs WiFi-6.
- investigation_type: web

### H3 — Linux SBC + mPCIe/M.2 mt76 card is the most flexible DIY path (HIGH plausibility, HIGH info-value)
A generic OpenWrt-supported SBC (with mPCIe/M.2 + USB/Ethernet) plus one or two mt7915/mt7916 (or ath9k/ath10k) cards lets you choose radio count and antennas freely. Investigate which SBCs are OpenWrt-supported with working PCIe, which mt76 mPCIe/M.2 cards expose U.FL, and integration gotchas.
- investigation_type: web

### H4 — Driver/mesh-mode reality check: which chipsets ACTUALLY do 802.11s/IBSS in current OpenWrt, and the WiFi-6/6GHz caveats (HIGH plausibility, HIGH info-value)
The crux of the prior failure. Verify mt76 802.11s/IBSS status (incl. mt7915/mt7916, 6 GHz mt7986 caveats), ath9k/ath10k status, and confirm wifi-qcom/ath11k/ath12k limitations. Surface mesh-mode gotchas (encryption/SAE, mesh+AP concurrency, DFS).
- investigation_type: hybrid (web + driver/kernel knowledge)

### H5 — Open-source-hardware options & antenna/RF/form-factor considerations (MEDIUM plausibility, MEDIUM info-value)
Fully open hardware (e.g. boards with open schematics, community projects) plus the cross-cutting RF/packaging concerns: U.FL vs RP-SMA, antenna gain/placement, board size, power input (PoE/12V/USB-C), thermals for a custom enclosure.
- investigation_type: web
