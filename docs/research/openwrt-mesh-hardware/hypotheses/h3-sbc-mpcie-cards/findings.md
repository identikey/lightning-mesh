# Hypothesis: H3 — Linux SBC + mPCIe/M.2 mt76 (or ath9k/ath10k) wireless card as the most flexible DIY mesh node

## Summary

**Supported (high confidence).** A Linux/OpenWrt SBC with mPCIe and/or M.2 (PCIe) slots paired with mt76 (MT7915/MT7916) or Atheros (ath9k/ath10k) radio cards is a proven, well-documented approach that delivers exactly the flexibility the user wants: free choice of radio count, bands, streams, and external antennas, with full 802.11s/IBSS mesh support in the driver. The dominant gotcha is **power delivery** — most mt7915/mt7916 4T4R cards demand 3.3V at 2.5–3A, which several SBC mPCIe slots under-provision. A secondary caveat is that the *board's own integrated WiFi* (e.g. BPI-R4's MT7988/MT7976) may be immature in OpenWrt, but that does not affect add-in PCIe cards, which are the relevant path here.

## Evidence

### (A) SBC / router-board side

**Banana Pi BPI-R4** — MediaTek MT7988A (Filogic 880), 4GB DDR4, 8GB eMMC. Exposes **2× mPCIe slots on a PCIe 3.0 2-lane interface**, plus M.2 (for 4G/5G/NVMe) [1]. Sold as a bare board (~$170 retail) [2]. **Important nuance:** the *onboard/companion* MT7988+MT7976 WiFi 7 support in OpenWrt is still snapshot-only and reportedly buggy — slow throughput (20–110 Mbps), reduced TX power, 6GHz disabled in some builds [3][4]. This is the board's *built-in* radio path, not the add-in card path. For this hypothesis the relevant fact is that the **mPCIe slots accept standard mt76/ath cards** and PCIe enumeration works.

**Banana Pi BPI-R3** — MT7986 (Filogic 830). Confirmed running OpenWrt 21.02+ with WiFi working; supports WiFi-6 cards via M.2/mPCIe adapters (e.g. AW7916-NPD with a BPI M.2-to-mPCIe adapter) [5]. More mature OpenWrt status than R4. There is a known forum thread on M.2-slot WiFi card recognition issues on R4 that does not appear on R3 [6].

**UniElec U7623 (MT7623)** — Frequently cited as a clean host for AsiaRF MT7915 cards on OpenWrt. The AW7915-NP1 is "confirmed functional" with proper FCC ID and MAC; supports AP/managed/IBSS/monitor/**mesh point** modes (up to 19 interfaces) [7]. This is the clearest primary confirmation of mesh-mode availability on an add-in mt7915 card.

**NanoPi R5C / R6C (FriendlyElec)** — R5C has **one M.2 E-Key slot (PCIe 2.1 x1 + USB2.0)**; R6C has an **M.2 M-Key (PCIe 2.1 x1)** intended for NVMe/PCIe WiFi [8]. Both run FriendlyWrt (OpenWrt-derived). Caveat: E-key/M-key keying and single x1 lane constrain card choice and antenna count vs. the BPI-R4's dual mPCIe; FriendlyElec documents its own RTL8822CE module rather than mt7915, so mt76 add-in support is community-territory, not vendor-validated [8].

**PC Engines APU2/3/4 (x86)** — The canonical bare-board reference. Its mPCIe-modules documentation explicitly lists and characterizes the relevant cards (table below) [9]. x86 + mainline OpenWrt gives the most solid PCIe support of any option here.

### (B) Radio-card side

**AsiaRF AW7915-NP1** (vendor primary source) [10]: MediaTek **MT7915**, Wi-Fi 6 (802.11ax/ac/abgn), dual-band 2.4/5GHz, **4T4R**, **mPCIe** form factor, **4× IPEX/U.FL connectors**, ~2401 Mbps PHY. Power: **9W max, requires 3.3V 3A (min 2.5A)**. Price **~$30**. Runs on mt76; tested working on apu2c4 + OpenWrt 21.02 with no firmware tweaks, but "gets hot (>130°C), heatsink recommended" [9][11]. Mesh/IBSS confirmed via mt76 driver iw output on the U7623 test [7].

**AsiaRF AW7916-NPD** [12]: **MT7916AN**, Wi-Fi **6E** (adds 6GHz), DBDC (G-band 2T2R + A-band 2T3R), up to ~3000 Mbps, **mPCIe**, FCC-certified 2.4/5/6GHz. Supported in OpenWrt 21.02, shares the mt7915 mt76 driver directory. **Known limitation:** a single MT7916 card cannot run 5GHz and 6GHz simultaneously (need two cards), and DBDC/6GHz has had regressions in some snapshot/24.x builds [12]. There's also an M.2 AE-key variant (AW7916-AED) and a 4T4R M.2 AE variant of the 7915 (AW7915-AE1).

**Compex WLE600VX / WLE900VX** (ath10k) [13]: WLE600VX = **QCA9882, 2T2R 802.11ac**, mPCIe, dual-band, ~867 Mbps; WLE900VX = QCA9880/9882, **3T3R**, up to ~1300 Mbps. Both ath10k-supported with **802.11s mesh listed as partial** support. Industrial-temp variants exist. More mature/stable than mt76 in many builds but no 802.11ax.

**Compex WLE200NX** (ath9k) [14]: **Atheros AR9280, 2T2R 802.11abgn**, dual-band, mPCIe, **U.FL connectors**, up to 300 Mbps. Fully open-source ath9k driver, **no binary firmware blob** needed — the gold standard for mesh reliability and firmware freedom. ath9k has long-standing, robust 802.11s/IBSS support. Price ~$28–35. Now near EOL at some vendors.

### Integration gotchas (cross-cutting)

- **Power delivery is the #1 issue.** MT7915/7916 4T4R cards need 3.3V @ 2.5–3A. The UniElec U7623 supplies only 1A on 3.3V; the documented fix is an external 12V→3.3V buck converter fed from the SATA connector back into the board's 3.3V rail (no soldering) [7]. Verify each SBC's mPCIe 3.3V budget before committing.
- **Thermals.** 4T4R mt7915 cards exceed 130°C without cooling — a heatsink is effectively mandatory in a custom enclosure [11].
- **Form-factor/keying.** mPCIe vs M.2 (and M.2 A/E-key vs M-key) are not interchangeable; adapters exist (BPI M.2-to-mPCIe) but add bulk [5]. mPCIe is the most universally available card format.
- **Antenna pigtails.** Card has U.FL/IPEX; you supply U.FL→RP-SMA pigtails matching stream count (4 for a 4T4R card). Antenna count is genuinely free choice, as the hypothesis assumes.
- **OpenWrt PCIe support** is solid on x86 (APU) and mature MediaTek SBCs (R3, U7623); newer boards (R4, R6C) have working PCIe enumeration but the board's *integrated* WiFi may lag.

## Candidate Pairings Table

| # | SBC / board | Slot(s) | Radio card | Chipset / driver | Bands / streams | Antennas (U.FL) | Mesh (802.11s/IBSS) | OpenWrt status | Price (card) | Key gotcha |
|---|-------------|---------|------------|------------------|-----------------|------------------|---------------------|----------------|--------------|------------|
| 1 | **PC Engines APU2/3/4** (x86) | 2–3× mPCIe | AsiaRF **AW7915-NP1** | MT7915 / mt76 | 2.4+5, 4T4R Wi-Fi6 | 4 | Yes (AP/IBSS/mesh) | Mature; tested on 21.02 | ~$30 | Needs 3.3V 3A; runs hot, heatsink req'd [9][10][11] |
| 2 | **UniElec U7623** (MT7623) | mPCIe | AsiaRF **AW7915-NP1** | MT7915 / mt76 | 2.4+5, 4T4R | 4 | **Confirmed mesh point** in iw | Working | ~$30 | Board only gives 1A@3.3V → external buck needed [7] |
| 3 | **Banana Pi BPI-R4** | 2× mPCIe (PCIe3 x2) | AsiaRF **AW7916-NPD** | MT7916 / mt76 | 2.4/5/6 (DBDC), Wi-Fi6E | 3–4 | Yes via mt76 | Snapshot; add-in card path OK, onboard WiFi buggy | ~$40–50 | No simultaneous 5+6GHz on one card; DBDC regressions [3][12] |
| 4 | **Banana Pi BPI-R3** | M.2 / mPCIe (adapter) | AsiaRF AW7916-NPD | MT7916 / mt76 | 2.4/5/6, Wi-Fi6E | 3–4 | Yes | More mature than R4 (21.02 OK) | ~$40–50 | Needs M.2→mPCIe adapter for mPCIe card [5] |
| 5 | **APU / any mPCIe SBC** | mPCIe | **Compex WLE900VX** | QCA9880 / ath10k | 2.4+5, 3T3R 11ac | 3 | Partial 802.11s | Mature | ~$30–45 | No Wi-Fi6; ath10k firmware blob; IOMMU notes [9][13] |
| 6 | **APU / any mPCIe SBC** | mPCIe | **Compex WLE200NX** | AR9280 / ath9k | 2.4+5, 2T2R 11n | 2 | **Full, robust** mesh/IBSS, no blob | Most mature | ~$28–35 | Older/EOL; only 300 Mbps [14] |
| 7 | **NanoPi R5C/R6C** | 1× M.2 (E/M-key, x1) | M.2 mt76 card (e.g. AW7916-AED) | MT7916 / mt76 | varies | 2–3 | Yes via mt76 | FriendlyWrt | ~$40 | Single x1 lane; keying limits; vendor validates only RTL8822CE [8] |

## Confidence

**Level: high**

Multiple independent primary sources agree: AsiaRF's own datasheet/product pages [10][12], PC Engines' module compatibility documentation [9], OpenWrt forum reports with concrete `iw`/test output confirming mesh-point mode [7], and the Banana Pi/FriendlyElec hardware docs [1][5][8]. The mesh-capability and flexibility claims are directly evidenced; the power/thermal gotchas are corroborated across vendor and forum sources.

## Sources

- [1] **url**: https://docs.banana-pi.org/en/BPI-R4/BananaPi_BPI-R4 — BPI-R4: MT7988A, 2× mPCIe on PCIe 3.0 2-lane, M.2 for 4G/5G/NVMe
- [2] **url**: https://www.amazon.com/BPI-R4-Wi-Fi-Dual-Band-Router-Board/dp/B0CPFJQCG2 — BPI-R4 bare router board, retail listing
- [3] **url**: https://forum.banana-pi.org/t/whats-the-best-firmware-for-bpi-r4-now-extreme-slow-wifi-7-speed-on-openwrt-snapshot/19528 — onboard WiFi 7 slow/buggy on OpenWrt snapshot
- [4] **url**: https://github.com/openwrt/openwrt/issues/20188 — "No wireless available on BPi-R4 with BE14" (onboard radio immaturity)
- [5] **url**: https://wiki.banana-pi.org/Banana_Pi_BPI-R3 — BPI-R3 OpenWrt 21.02, AW7916-NPD via M.2-to-mPCIe adapter
- [6] **url**: https://forum.banana-pi.org/t/wifi-cards-not-recognized-in-m-2-slot/19167 — M.2-slot card recognition issue thread (R4)
- [7] **url**: https://forum.openwrt.org/t/mt7915-pcie-cards-on-bpi-unielec-boards/118194 — AW7915-NP1 confirmed on U7623; mesh point in iw; 3.3V 1A→buck fix
- [8] **url**: https://wiki.friendlyelec.com/wiki/index.php/NanoPi_R5C — R5C M.2 E-key PCIe2.1 x1; R6C M.2 M-key; FriendlyWrt; RTL8822CE module
- [9] **url**: http://pcengines.github.io/apu2-documentation/mpcie_modules/ — APU supported modules: WLE200NX(ath9k), WLE600VX/900VX(ath10k), AW7915-NP1(mt76, heatsink req'd)
- [10] **url**: https://asiarf.com/product/wifi-6-11ax-4t4r-mini-pcie-module-mt7915-aw7915-np1/ — MT7915, mPCIe, 4T4R, 4× IPEX, 9W/3.3V 3A, ~$30
- [11] **url**: https://forum.openwrt.org/t/aw7915-np1-x86-access-point/134441 — tested on apu2c4 OpenWrt 21.02; >130°C, heatsink recommended
- [12] **url**: https://asiarf.com/product/wi-fi-6e-mini-pcie-module-mt7916-aw7916-npd/ — MT7916AN Wi-Fi 6E, DBDC, mPCIe, shares mt7915 mt76 driver; no simultaneous 5+6GHz
- [13] **url**: https://wireless.docs.kernel.org/en/latest/en/users/drivers/ath10k.html — ath10k QCA9880/9882, 802.11s partial support
- [14] **url**: https://www.electromaker.io/shop/product/compex-wle200nx-80211n-mpcie-wi-fi-card-compatible-with-the-mnt-reform-laptop-uses-open-source-ath9k-drivers-no-binary-firmware-needed — AR9280 ath9k, open-source driver, no binary firmware, U.FL

## Open Questions

- **Exact 3.3V current budget per mPCIe slot on BPI-R4 and NanoPi R5C/R6C** is undocumented in the sources found — needs measurement or vendor confirmation before assuming a 4T4R card will run without an auxiliary supply.
- **Multi-radio (2× mPCIe) mesh on a single BPI-R4** is plausible but not directly evidenced; whether both PCIe 3.0 lanes enumerate two high-power cards simultaneously under their combined 3.3V draw is unverified.
- **mt76 802.11s stability/throughput in mesh mode specifically** (vs. AP mode) on current OpenWrt — forum reports confirm mode availability but not sustained mesh performance; ath9k/ath10k mesh maturity is better understood.
- Whether the **M.2 A/E-key vs M-key** distinction on FriendlyElec boards admits common mt7915/mt7916 A+E-key cards needs per-card keying verification.

## Verdict vs requirements

| Requirement | Verdict |
|---|---|
| Maximum flexibility (free radio count / bands / antennas) | **Met** — mPCIe SBCs (APU, U7623, BPI-R4) give 1–3 slots; cards from 2T2R to 4T4R; 2–4 antennas freely chosen |
| TRUE mesh (802.11s / IBSS) | **Met** — confirmed in mt76 (iw shows mesh point) and ath9k/ath10k (ath9k fully, ath10k partial) |
| Bare-board / custom enclosure | **Met** — APU, BPI-R3/R4, NanoPi all ship as bare boards |
| External antenna connectors | **Met** — all listed cards expose U.FL/IPEX (2–4) |
| Compact | **Met** — mPCIe + small SBC; thermals/power add the real bulk |
| OpenWrt PCIe support solid | **Mostly met** — excellent on x86/APU and BPI-R3/U7623; add-in card path on BPI-R4 fine, but its *onboard* WiFi 7 is immature |

**Recommended best pairings:** For Wi-Fi 6 flexibility, **APU2/3/4 or UniElec U7623 + AsiaRF AW7915-NP1** (mature, cheap, mesh-confirmed) — budget for the 3.3V supply and a heatsink. For maximum firmware-freedom mesh reliability, **any mPCIe SBC + Compex WLE200NX (ath9k)**. For Wi-Fi 6E/6GHz, **BPI-R4 or R3 + AsiaRF AW7916-NPD**, accepting current DBDC/6GHz software immaturity.

**Note for synthesizer (outside my hypothesis):** the BPI-R4's *integrated* MT7988/MT7976 WiFi 7 being buggy in OpenWrt is a finding that may be relevant to any sibling hypothesis evaluating turnkey/onboard-radio router boards rather than the add-in-card approach.