# Hypothesis: H2 — ath9k/ath10k boards as the gold standard for rock-solid 802.11s/IBSS (WiFi-5/4 tradeoff)

## Summary
**Supported, with one critical caveat.** ath9k (802.11n) is genuinely the most battle-tested mesh/IBSS driver in Linux/OpenWrt — it is the de facto standard for amateur-radio mesh (AREDN) precisely because of its maturity and reliability. ath10k (802.11ac) mesh works but is **driver-and-firmware-dependent**: the *default OpenWrt ath10k-CT driver historically did not reliably support 802.11s on Wave-1 QCA988x chips*, and IBSS/adhoc on QCA988x is effectively broken — you must switch to the mainline `ath10k` kmod (e.g. `kmod-ath10k-smallbuffers` + non-CT firmware) for solid mesh. Buyable dual-radio bare boards still exist (Compex WPJ563 + WLE-series mPCIe cards), but many of the classic platforms are EOL or NOS-only (PC Engines APU).

## Evidence

### Driver maturity (the heart of the hypothesis)
- **ath9k is the proven gold standard.** AREDN (Amateur Radio Emergency Data Network) states the open-source driver used is ath9k, and its hardware requirement is explicitly "802.11n with a Qualcomm Atheros (QCAxxxx or ARxxxx) chipset using the linux ath9k driver." ath9k is described as "the reliable, well-established option" and "the de facto standard for amateur radio mesh networking because of its maturity, open-source nature... and proven reliability in AREDN deployments worldwide" [1]. Notably, 802.11ac/ath10k is *not* used by AREDN partly because the drivers lack the needed maturity/capabilities.
- **ath9k stability flag:** `nohwcrypt=1` is a known stability tweak; ath9k otherwise "works well for older Atheros cards and avoids hardware encryption issues" [2].
- **ath10k mesh is conditional.** The OpenWrt forum thread is the key primary source: the default **ath10k-CT driver only supports 802.11s on Wave-2 chips**, and the working resolution was to switch to mainline driver: "`kmod-ath10k-smallbuffer` and `ath10k-firmware-qca9888` instead of the default -ct variants," combined with `wpad-mesh-wolfssl` [3]. Consensus in-thread: "use non-CT variants for reliable 802.11s mesh functionality."
- **IBSS on QCA988x is effectively dead.** For IBSS specifically, OpenWrt guidance recommends 802.11s instead of adhoc/IBSS with recent QCA988* firmware; the only IBSS path was "the firmware fork of Ben Greear, but it was never really working and also still had bugs" [2]. **Takeaway for the user: target 802.11s, not IBSS, on ath10k.**
- **By contrast, MediaTek mt76 mesh is flakier:** documented issues include nodes not meshing at all or seeing each other at "ridiculously low signal strength and tx rate" (MT7610E), and poor cross-vendor (QCA↔MT) mesh throughput [2][4]. This reinforces the relative superiority of the Atheros stack for mesh.

### Buyable dual-radio bare boards / cards
- **Compex WPJ563** — QCA9563 SoC (775 MHz), on-board 2.4 GHz 3×3 MIMO (ath9k) **plus** a MiniPCIe slot for a 5 GHz ath10k card (WLE600VX/WLE900VX) → true dual-radio dual-band node. Sold currently via 524wifi.com. OpenWrt-supported (mainline target + Compex's own LEDE 18.06 branch) [5][6]. Note: a known mainline bug exists for USB on WPJ563 [6].
- **Compex WLE600VX** (the 5 GHz ath10k card) — **QCA9882** (commercial) / QCA9892 (industrial), dual-band selectable 2.4/5 GHz, 2×2 MIMO, up to 867 Mbps, standard mPCIe 30×51 mm, **2× U.FL connectors**, FCC/CE/IC certified, **currently in production and buyable** from Compex directly, 524wifi, Techship, eBay [7][8]. ath10k driver.
- **Compex WLE900VX** — QCA9880/QCA9882, 3×3 MIMO 802.11ac mPCIe, ath10k [9]. Still sold (e.g. Teklager WLE900VX kit).
- **Compex WLE200NX** — Atheros AR9287, 2.4 GHz only, ath9k mPCIe card. The most mature/reliable mesh radio, but 2.4 GHz-only and 802.11n [10].
- **Compex branch supports** WPJ563, WPJ558, WPJ342, WPJ334, WPQ864, WPQ865, WPJ428, WPJ419 [11]. WPQ864 (IPQ806x) is mainline OpenWrt. **WPQ672 is NOT in the supported list** — avoid for the user's needs.

### Platform availability caveats
- **PC Engines APU2/APU4 is EOL.** PC Engines discontinued APU production; hardware reached end-of-life and firmware updates stopped (~2023) [12]. The APU2/APU4 + WLE200NX (ath9k) / WLE600VX/WLE900VX (ath10k) was the classic 2-radio reference combo, but it is now **NOS/used-only**. Known quirk: ath9k WLE200NX needs `options ath9k use_msi=1` due to IOMMU/INTx interrupt remapping [12][10].
- **Successor: Noah4C (Rack Matrix)** — Intel Atom E3845 quad-core, **two MiniPCIe slots** (WLAN/LTE/GPS), reuses APU cases/mounting; positioned as "the powerful successor to the APU series." x86, so any ath9k/ath10k mPCIe card works. Price/availability not published on the source page [13].
- **GL.iNet GL-AR300M(-Ext)** — QCA9531 (650 MHz), 2.4 GHz 2×2 MIMO **single-radio**, ath9k, 2× external antennas (-EXT variant), 23 dBm, cased, OpenWrt pre-installed. Compact and reliable, but **single-radio 2.4 GHz only** — does not meet the ≥2-radio requirement on its own [14]. (GL-AR750 "Creta" adds a 5 GHz ath10k radio → dual-band dual-radio in a case.)

## Candidate Table

| Candidate | Vendor | SoC | Radio chipset(s) + driver | Radios / bands | Antenna conn. | OpenWrt status | Form factor | Power | Price (rough) | Production? |
|---|---|---|---|---|---|---|---|---|---|---|
| **WPJ563** (board) + WLE600VX (card) | Compex | QCA9563 775 MHz | On-board AR-class 2.4 GHz 3×3 (ath9k) + QCA9882 2×2 (ath10k) | 2 radios, 2.4 + 5 GHz | U.FL on card (2×) + board ipex | Mainline + Compex LEDE branch [5][6][11] | Bare board (~ credit-card class) | DC barrel/PoE variants | board ~$60-90 + card ~$25-40 | **Yes** (524wifi) |
| **WLE600VX** (mPCIe card) | Compex | n/a | QCA9882/QCA9892 (ath10k) | 1 radio, 2.4/5 GHz selectable, 2×2 | **2× U.FL** | ath10k (use mainline kmod for mesh) [7][8] | mPCIe 30×51 mm | from host | ~$25-45 | **Yes** |
| **WLE900VX** (mPCIe card) | Compex | n/a | QCA9880/9882 (ath10k), 3×3 | 1 radio, 2.4/5 GHz | 3× U.FL | ath10k [9] | mPCIe full | from host | ~$30-50 | Yes (Teklager etc.) |
| **WLE200NX** (mPCIe card) | Compex | n/a | AR9287 (ath9k) | 1 radio, 2.4 GHz, 2×2 | 2× U.FL | ath9k (most mature mesh) [10][12] | mPCIe | from host | ~$15-25 | Limited/NOS |
| **APU2/APU4 + 2× mPCIe cards** | PC Engines | AMD GX-412TC quad | mix: WLE200NX ath9k + WLE600VX ath10k | 2-3 radios | U.FL per card | Full mainline x86_64 | ~6×6" board, cased | 12 V DC | NOS ~$120-160 | **EOL — NOS/used only** [12] |
| **Noah4C** | Rack Matrix | Intel Atom E3845 quad | any ath9k/ath10k mPCIe (BYO) | 2 slots | per card | x86_64 mainline | APU-compatible | DC | TBD (no public price) | New, niche [13] |
| **GL-AR300M-Ext** | GL.iNet | QCA9531 650 MHz | AR-class 2.4 GHz 2×2 (ath9k) | **1 radio**, 2.4 GHz only | 2× RP-SMA external | OpenWrt pre-installed [14] | Cased mini | 5 V USB | ~$40 | Yes |
| **GL-AR750 "Creta"** | GL.iNet | QCA9531 | ath9k 2.4 GHz + ath10k (QCA9887) 5 GHz | 2 radios, dual-band | external | OpenWrt | Cased | 5 V USB | ~$50-70 | Yes |

## Verdict vs requirements
- **TRUE mesh (802.11s)** ✅ — Atheros stack is the strongest choice; **target 802.11s, not IBSS** (IBSS on QCA988x is broken). For ath10k, **plan to swap CT firmware/driver for mainline `ath10k` + non-CT firmware** to get reliable 802.11s.
- **≥2 radios** ✅ — Compex WPJ563 (on-board ath9k 2.4 GHz + mPCIe ath10k 5 GHz) is the cleanest single-board answer; APU2/Noah4C give 2-3 mPCIe slots for full BYO-radio flexibility.
- **Bare-board / SBC** ✅ — WPJ563 bare board and Noah4C fit; GL.iNet are cased (less "bare").
- **External antenna connectors** ✅ — WLE cards expose U.FL; boards/cases expose RP-SMA pigtails.
- **Compact** ✅ — WPJ563 and mPCIe cards are very compact; APU is larger (~6").
- **Tradeoff confirmed** ✅ — WiFi 4/5 only, lower throughput (≤867 Mbps 5 GHz on 2×2 ath10k, ≤300 Mbps 2.4 GHz ath9k), but most reliable mesh.

**Recommended buyable pairing:** Compex **WPJ563** board + **WLE600VX** (QCA9882) 5 GHz card → dual-radio, dual-band, all-Atheros, mainline OpenWrt, currently in production.

## Confidence
**Level**: medium-high

Driver-maturity claims are corroborated by multiple independent sources (AREDN docs + OpenWrt forum + OpenWrt mesh guidance). Buyability/production status of Compex WLE600VX and WPJ563 confirmed on vendor pages. Medium (not high) because: exact prices are approximate, the OpenWrt official Compex ToH page failed to render (relied on the Compex LEDE GitHub repo + 524wifi instead), and the ath10k-CT-vs-mainline mesh situation comes largely from one forum thread (though consistent with OpenWrt's IBSS guidance).

## Sources
- [1] **url**: https://www.arednmesh.org/content/porting-new-hardware — "driver used in AREDN is ath9k... 802.11n with a QualComm Atheros chipset using the linux ath9k driver"; ath9k = de facto mesh standard
- [2] **url**: https://forum.openwrt.org/t/ath10k-ct-wifi-driver-does-not-support-802-11s/125423 (and OpenWrt mesh setup notes, https://phb-crystal-ball.org/setup-openwrt-mesh-network/) — ath9k `nohwcrypt=1`; "802.11s recommended instead of adhoc/IBSS" for QCA988x; Ben Greear IBSS fork "never really working"
- [3] **url**: https://forum.openwrt.org/t/ath10k-ct-wifi-driver-does-not-support-802-11s/125423 — "kmod-ath10k-smallbuffer and ath10k-firmware-qca9888 instead of -ct variants"; CT only does 802.11s on Wave-2; success with `ath10k-smallbuffers` + `wpad-mesh-wolfssl`
- [4] **url**: https://github.com/openwrt/mt76/issues/259 and https://github.com/openwrt/mt76/issues/387 — mt76 802.11s nodes not meshing; poor QCA↔MT mesh throughput (contrast against Atheros maturity)
- [5] **url**: https://www.524wifi.com/index.php/compex-wpj563hv-dual-radio-gigabit-embedded-board-802-11ac.html — WPJ563 QCA9563 775 MHz, on-board 2.4 GHz 3×3, mPCIe slot for WLE900V5/WLE900VX ath10k card; LEDE/OpenWrt + ath9k/ath10k
- [6] **url**: https://github.com/openwrt/openwrt/issues/13650 — WPJ563 mainline OpenWrt (USB quirk noted)
- [7] **url**: https://compex.com.sg/shop/wifi-module/802-11ac-wave-1/wle600vx-wifi5-11ac-qca9882-qca9892/ — WLE600VX: QCA9882/9892, 2.4/5 GHz selectable, 2×2, mPCIe 30×51 mm, 2× U.FL, in production, "Buy Sample"
- [8] **url**: https://techship.com/product/compex-wle600vx/ — WLE600VX 802.11ac Wi-Fi 5, currently sold (corroborates buyability)
- [9] **url**: https://teklager.se/en/products/router-components/wle900vx-wireless-wifi-kit — WLE900VX QCA9880/9882 3×3 ath10k mPCIe kit, sold
- [10] **url**: http://pcengines.github.io/apu2-documentation/mpcie_modules/ — WLE200NX = AR9287 (ath9k) 2.4 GHz; WLE600VX/WLE900VX ath10k; interrupt/IOMMU quirks
- [11] **url**: https://github.com/compex-systems/lede — Compex LEDE branch supported devices: WPJ563, WPJ558, WPJ342, WPJ334, WPQ864, WPQ865, WPJ428, WPJ419 (WPQ672 absent)
- [12] **url**: https://www.pcengines.ch/eol.htm (and http://pcengines.github.io/apu2-documentation/mpcie_modules/) — APU platform EOL, firmware frozen ~2023; ath9k WLE200NX needs `use_msi=1`
- [13] **url**: https://www.varia.org/en/noah4c-the-successor-to-the-apu-series/ — Noah4C: Intel Atom E3845, 2× MiniPCIe, APU-case compatible, "successor to APU series"
- [14] **url**: https://docs.gl-inet.com/router/en/2/hardware/ar300m/ (and Amazon GL-AR300M-Ext listing https://www.amazon.com/dp/B01K6MHRJI) — QCA9531, 2.4 GHz 2×2 ath9k, 2× external antennas, single-radio, cased, OpenWrt pre-installed

## Open Questions
- **Exact mainline-OpenWrt mesh status on QCA9882 (Wave-1) as of current OpenWrt 24.x** — is the CT-vs-mainline `ath10k-smallbuffers` swap still required, or has 802.11s on Wave-1 been fixed in recent firmware? The evidence is from older forum threads; needs verification against the current OpenWrt release notes/wiki.
- **Whether 5 GHz ath10k 802.11s holds up under multi-hop load** vs ath9k 2.4 GHz — the user's design (per repo: radio backhaul + multi-hop) may favor ath9k 2.4 GHz for backhaul reliability even at lower throughput.
- **Noah4C concrete price, lead time, and whether its E3845 + 2 mPCIe slots leaves room for both an ath9k and ath10k card with adequate cooling** — vendor page lacked pricing.
- **WLE200NX (AR9287 ath9k) current procurability** — appears to be tapering to NOS; a current-production 2.4 GHz ath9k mPCIe equivalent should be identified if the user wants the most-mature radio specifically for backhaul.
- (Outside this hypothesis, for synthesizer) MediaTek mt76 / WiFi-6 boards are the throughput-vs-maturity counterpoint and are tracked under a sibling hypothesis — the mesh-reliability gap documented in [4] is relevant cross-reference material.

## Sub-Hypotheses
None — per DEPTH_REMAINING guidance, concrete candidates were prioritized over sub-branching. (The current-OpenWrt-24.x ath10k mesh verification noted in Open Questions is the strongest candidate for a follow-up branch if the swarm chooses to spend depth.)