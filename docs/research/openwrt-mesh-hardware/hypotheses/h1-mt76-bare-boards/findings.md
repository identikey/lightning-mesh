# Hypothesis: H1 — Ready-made bare-board MediaTek mt76 (mt7915/mt7916) WiFi-6 OpenWrt mesh routers with external antenna connectors

## Summary
**SUPPORTED.** Multiple currently-buyable bare-board MediaTek (Filogic 8xx) router boards exist with external antenna connectors (U.FL/IPEX or MMCX) and mainline/snapshot OpenWrt support. The strongest fit for the user's "≥2 radios, one for 802.11s backhaul + one for AP/STA clients" requirement is the **Banana Pi BPI-R4**, which ships with NO onboard radio and two miniPCIe slots you populate with independent mt7916 NIC cards (each with its own U.FL antenna pigtails) — giving two physically separate, independently-configurable mt76 radios. The **Banana Pi BPI-R3** (onboard MT7976, 2x U.FL/IPEX) and **OpenWrt One** (official board, MT7976, 3x MMCX) are cheaper single-chip dual-band options but use one MT7976 chip serving both bands, which constrains the "dedicated mesh radio + dedicated client radio" topology. **Critical caveat:** mt76 802.11s mesh on mt7915/mt7916 has a documented history of intermittent breakage, especially on the 5 GHz radio and with WPA3/SAE — it works but is not bulletproof.

## Evidence

### Candidate 1 — Banana Pi BPI-R4 (best multi-radio fit)
From the official Banana Pi docs [1]: SoC is **MediaTek MT7988A (Filogic 880)**, quad-core Cortex-A73 @ 1.8 GHz, 4/8 GB DDR4, 8 GB eMMC. Crucially: **"2x miniPCIe slots with PCIe3.0 2lane interface for Wi-Fi NIC"** — the MT7988A SoC has no integrated WiFi MAC/PHY; you add radio cards. The docs explicitly recommend the **ASIA.RF AW7916-NPD: MT7916, "G-band 2T2R and A-band 3T3R 2ss Dual Bands Dual Concurrent mPCIe Card"** [1][4]. Board is 100.5 x 148 mm, has 2x 10GbE SFP + 4x GbE, USB 3.2, M.2 KEY-B (5G modem) and KEY-M (NVMe). This is the platform where you can fit **two separate mt7916 mPCIe cards** = two independent radios, each with U.FL/IPEX pigtails to external antennas — exactly matching "one radio for 802.11s backhaul, another for AP/STA." Forum thread [6] confirms users running 2–3 mPCIe WiFi cards in the slots (with some PCIe-enumeration troubleshooting needed). Bare board ~$120–150 USD (Amazon listings [2]); mPCIe mt7916 cards add ~$30–50 each.

### Candidate 2 — Banana Pi BPI-R3 (onboard dual-band, antenna pigtails)
**MT7986A (Filogic 830)**, quad-core A53, 2 GB DDR4, 8 GB eMMC + 128 MB NAND, 2x 2.5GbE SFP + 5x GbE [3]. Onboard WiFi via **MT7976C** (DBDC: 2×2 2.4 GHz + 3×3 5 GHz). The full-size BPI-R3 exposes onboard **IPEX/U.FL antenna connectors** for the onboard radio, plus a miniPCIe slot for an additional card. Mainline-supported on OpenWrt (mediatek/filogic target). ~$100–130 USD bare board.

### Candidate 3 — Banana Pi BPI-R3 Mini (most compact)
From CNX Software [5]: **MT7986A**, dual-band WiFi 6 via **MT7976C** (2.4 GHz 574 Mbps + 5 GHz 2402 Mbps), and notably **"3x U.FL antenna connectors"**. Tiny **65 x 65 x 10 mm**, 2x 2.5GbE, USB-C PD power (20W/12V), NANO SIM + M.2 for optional 4G/5G [3-newegg]. OpenWrt snapshot/mainline. ~$70–90 USD. Best "deployable anywhere" compact form factor, but single MT7976 chip (DBDC), so no truly independent second radio without the M.2/mPCIe add-on.

### Candidate 4 — OpenWrt One (official OpenWrt project board)
From Banana Pi official docs [7] and CNX Software [8]: **MT7981B (Filogic 820)** dual-core A53 @ 1.3 GHz, **MT7976C** dual-band WiFi 6 (2×2 2.4 GHz + 3×3/2×2 + zero-wait DFS 5 GHz), 1 GB DDR4, 256 MiB NAND + 16 MiB protected NOR (unbrickable dual-boot), 1x 2.5GbE + 1x GbE, M.2 2230/2242 NVMe, USB 2.0, mikroBUS. **3x MMCX antenna connectors** (note: MMCX, not U.FL — you need MMCX pigtails/antennas or MMCX→RP-SMA adapters). **148 x 100.5 mm, compatible with the BPI-R4 case**. **$89 USD** [8]. It is the reference OpenWrt board with first-class community support (snapshot images via firmware-selector). Single MT7976 chip though — same DBDC constraint as BPI-R3.

### Candidate 5 — HLK-RM65 module (Hi-Link, embedded OEM)
From Hi-Link: **MT7981B + MT7976C + MT7531A** AX3000 dual-band WiFi 6 module, marketed as an OpenWrt routing module. This is a solder-down/castellated OEM module rather than a hobbyist board — relevant if building a custom carrier PCB, but more integration work. Antenna access via U.FL on module.

### Rejected: NanoPi R-series
FriendlyElec NanoPi R3S/R4S/R5S use **Rockchip RK3566/RK3399** SoCs, not MediaTek, and rely on USB/SDIO WiFi dongles (e.g., RTL chips) — **not mt76**, so out of scope for this hypothesis.

### mt76 802.11s/IBSS mesh status (the key caveat)
The mt76 driver (mac80211-based) is the upstream Linux/OpenWrt driver for mt7915/mt7916. Mesh works but has a documented troubled history:
- openwrt/mt76 issue #675: "**Mt7915 5ghz 802.11s mesh is not working**" on a snapshot build [9].
- openwrt/mt76 issue #259: "802.11s nodes not meshing" — reporter notes **"802.11s works perfectly fine on the other radio at 2.4 GHz"** (i.e., 2.4 GHz mesh OK, 5 GHz problematic) [10].
- openwrt/mt76 issue #564: "Mesh network **WPA3 problems** for MT7622BV + MT7915E" [11].
- openwrt/mt76 issue #622: "mesh having been broken in recent snapshots."
- OpenWrt forum thread 194776 shows users running 802.11s on GL-MT6000 (MT7986) on snapshot.
Takeaway: 802.11s on mt7915/mt7916 is functional but version-sensitive; 2.4 GHz mesh is more reliable than 5 GHz, and SAE/WPA3-encrypted mesh has had bugs. Recommend pinning a known-good snapshot and testing per-band.

### 6 GHz / WiFi 6E note
MT7986/MT7976 is **WiFi 6 (no 6 GHz)** despite some vendor listings loosely saying "6E." True 6 GHz on this family requires the mt7916-based 6E mPCIe cards (e.g., AsiaRF AW7916-NPD markets "6E") or the MT7988A WiFi 7 NIC. mt76 6 GHz support is newer and less battle-tested than 2.4/5 GHz. Do not assume 6 GHz mesh is production-ready.

## Confidence
**Level: high** (for hardware existence, specs, connectors, OpenWrt support) / **medium** (for mesh reliability nuances).

Hardware specs and antenna-connector facts come from multiple independent primary sources (Banana Pi official docs, OpenWrt-affiliated docs, CNX Software). The mesh-caveat assessment is high-confidence on "issues exist" (multiple upstream GitHub issues) but medium on exact current-snapshot status since the driver changes frequently.

## Sources
- [1] **url**: https://docs.banana-pi.org/en/BPI-R4/BananaPi_BPI-R4 — "2x miniPCIe slots with PCIe3.0 2lane interface for Wi-Fi NIC"; recommends "ASIA.RF AW7916-NPD: WiFi6E ... mPCIe Card ... MT7916"; MT7988A Filogic 880; 100.5x148mm
- [2] **url**: https://www.amazon.com/BPI-R4-Wi-Fi-Dual-Band-Router-Board/dp/B0CPFJQCG2 — BPI-R4 bare board listing, 2x 10GbE SFP + 4x GbE, OpenWrt
- [3] **url**: https://wiki.banana-pi.org/Banana_Pi_BPI-R3 — "MediaTek MT7986(Filogic 830) ... 2G DDR RAM, 8G eMMC"; onboard MT7976C dual-band
- [3-newegg] **url**: https://www.newegg.com/p/0E6-01U7-00036 — BPI-R3 Mini: MT7986A Filogic 830, "Sizes 65 x 65 x10 mm", 20W/12V USB-C PD, 4G LTE
- [4] **url**: https://asiarf.com/product/wi-fi-6e-mini-pcie-module-mt7916-aw7916-npd/ — AW7916-NPD MT7916 2T2R+3T3R DBDC mPCIe card (the BPI-R4 radio module)
- [5] **url**: https://www.cnx-software.com/2023/07/26/banana-pi-bpi-r3-mini-low-profile-2-5gbe-router-board-mediatek-filogic-830-soc/ — BPI-R3 Mini: MT7986A, MT7976C dual-band (2.4GHz 574Mbps + 5GHz 2402Mbps), "3x U.FL antenna connectors"
- [6] **url**: https://forum.banana-pi.org/t/wifi-cards-not-recognized-in-m-2-slot/19167 — users running multiple mPCIe WiFi cards in BPI-R4 slots (enumeration troubleshooting)
- [7] **url**: https://docs.banana-pi.org/en/OpenWRT-One/BananaPi_OpenWRT-One — OpenWrt One: MT7981B (Filogic 820) + MT7976C, "3x MMCX antenna connectors", 148x100.5mm, snapshot firmware-selector link
- [8] **url**: https://www.cnx-software.com/2024/10/02/buy-openwrt-one-wifi-6-router-filogic-820-soc/ — OpenWrt One $89, MT7976C 2×2 2.4 + 3×3 5 GHz zero-wait DFS, 3x MMCX connectors
- [9] **url**: https://github.com/openwrt/mt76/issues/675 — "Mt7915 5ghz 802.11s mesh is not working"
- [10] **url**: https://github.com/openwrt/mt76/issues/259 — "802.11s nodes not meshing"; 2.4 GHz mesh works, 5 GHz problematic
- [11] **url**: https://github.com/openwrt/mt76/issues/564 — "Mesh network WPA3 problems for MT7622BV + MT7915E"
- [12] **url**: https://techinfodepot.shoutwiki.com/wiki/GL.iNet_GL-MT6000_(Flint_2) — Flint 2 internals: MediaTek MT7986AV + MT7976GN + MT7976AN (sealed consumer unit)
- [13] **url**: https://www.hlktech.net/index.php?id=1174 — HLK-RM65: "Dual Band WiFi 6 Gigabit AX3000 OpenWrt Routing Module MT7981B+MT7976C+MT7531A"
- [14] **url**: https://www.gl-inet.com/products/gl-mt6000/ — GL.iNet Flint 2 GL-MT6000 product page (Wi-Fi 6 + OpenWrt VPN router, sealed)

## Candidate Table

| Model / Vendor | SoC | Radio chipset(s) & count | Bands | Spatial streams | Antenna connectors | OpenWrt support | mt76 802.11s mesh | Form factor / dims | Power | Price (USD) | Form |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **Banana Pi BPI-R4** | MT7988A (Filogic 880) | **None onboard** — 2x mPCIe slots; populate w/ 2x MT7916 NIC (e.g. AsiaRF AW7916-NPD) | 2.4/5 (+6 w/ 6E card) | 2T2R + 3T3R per card | **U.FL/IPEX** on each mPCIe card (multiple) | Mainline/snapshot (mediatek/filogic) | Yes (mt76), per-band caveats | Bare PCB, 100.5×148 mm | 12V DC | ~$120–150 board + ~$30–50/card | **Bare board** |
| **Banana Pi BPI-R3** | MT7986A (Filogic 830) | 1x MT7976C onboard (+1 mPCIe slot) | 2.4/5 | 2×2 + 3×3 (DBDC) | **IPEX/U.FL** onboard + mPCIe | Mainline | Yes, per-band caveats | Bare PCB ~148×100 mm | 12V DC | ~$100–130 | **Bare board** |
| **Banana Pi BPI-R3 Mini** | MT7986A (Filogic 830) | 1x MT7976C onboard (+M.2) | 2.4/5 | 2×2 + 3×3 (DBDC) | **3x U.FL** | Snapshot/mainline | Yes, per-band caveats | Bare PCB **65×65×10 mm** | USB-C PD / 12V (20W) | ~$70–90 | **Bare board (compact)** |
| **OpenWrt One** (official) | MT7981B (Filogic 820) | 1x MT7976C onboard | 2.4/5 | 2×2 + 3×3 (DBDC) | **3x MMCX** (need MMCX pigtails) | **First-class** (reference board) | Yes, per-band caveats | Bare PCB 148×100.5 mm (BPI-R4 case-compatible) | USB-C / 12V | **$89** | **Bare board** |
| **HLK-RM65** (Hi-Link) | MT7981B | MT7976C + MT7531A | 2.4/5 | DBDC AX3000 | U.FL on module | OpenWrt (vendor + community) | Yes (same driver) | Castellated OEM module | 3.3V | ~$25–40 | **Solder-down module** |
| GL.iNet GL-MT6000 (Flint 2) | MT7986AV | 2x MT7976 (AN+GN) | 2.4/5 | up to 4×4 | Internal/external (sealed) | Snapshot | Yes | **Sealed consumer case** | 12V DC | ~$150 | Sealed (re-case hard) |
| NanoPi R3S/R4S/R5S | Rockchip RK3566/RK3399 | **Not mt76** (RTL USB) | — | — | — | OpenWrt (Rockchip) | N/A | Bare board | USB-C | ~$50–80 | Rejected (not MediaTek) |

## Verdict vs Requirements
- **TRUE mesh (802.11s/IBSS) on OpenWrt with mt76**: Achievable on all MediaTek candidates, but with documented reliability caveats — 2.4 GHz mesh is most stable; 5 GHz and WPA3/SAE mesh have a history of regressions. Pin a tested snapshot.
- **≥2 radios (tri-radio ideal), one for 802.11s backhaul + one for AP/STA**: **Only the BPI-R4 cleanly satisfies this** with two physically independent mt7916 mPCIe cards (each a separate `phy`/`radio` in OpenWrt). The single-MT7976 boards (BPI-R3, R3 Mini, OpenWrt One) are DBDC — one chip, two bands — so a "dedicated mesh radio + dedicated client radio" requires using the 2.4 GHz band for one role and 5 GHz for the other on the same chip, or adding an mPCIe/M.2 card. True tri-radio means BPI-R4 with multiple cards (or BPI-R3 onboard + mPCIe).
- **Bare-board / build-own-enclosure**: BPI-R4, BPI-R3, BPI-R3 Mini, OpenWrt One all ship as bare PCBs. BPI-R3 Mini (65×65mm) is the most "deployable anywhere." GL-MT6000 is sealed and a poor fit for re-casing.
- **External antenna connectors for high-gain antennas**: BPI-R3/R3 Mini and mPCIe radio cards = U.FL/IPEX pigtails (run to RP-SMA bulkheads). OpenWrt One = MMCX (needs MMCX→RP-SMA adapters). All satisfy the requirement; U.FL is the more common ecosystem.

**Recommended primary candidate: Banana Pi BPI-R4 + 2x MT7916 mPCIe cards (AsiaRF AW7916-NPD or similar)** for true multi-independent-radio mesh. **Recommended compact/cheap candidate: OpenWrt One ($89, best OpenWrt support) or BPI-R3 Mini (65×65mm)** where DBDC single-chip is acceptable.

## Open Questions
- Exact current-snapshot 802.11s reliability on mt7916 (vs older mt7915) — the GitHub issues skew toward mt7915/MT7622 era; need to confirm whether recent (2025) mt76 commits resolved 5 GHz mesh on Filogic 830/880. Worth a targeted test before fleet commitment.
- Whether the BPI-R4's two mPCIe slots can run **two mt7916 cards simultaneously** with stable PCIe enumeration in OpenWrt (forum thread 19167 shows users hit recognition issues with 3 cards; 2-card stability unconfirmed).
- MMCX vs U.FL pigtail availability/cost for OpenWrt One in the user's region (MMCX is less common than U.FL).
- Whether mt76 supports concurrent 802.11s mesh + AP (mesh gate / AP on same radio) reliably, or whether dedicating a full radio per role is required — affects whether single-chip boards are viable.
- 6 GHz mesh maturity on mt7916 6E cards — likely too immature for production backhaul.

## Sub-Hypotheses
None warranted — concrete buyable candidates delivered directly. The one item that could merit a sub-investigation (current mt76 5 GHz 802.11s reliability on Filogic 830/880 at a specific snapshot revision) is flagged in Open Questions for the synthesizer rather than spawned, since resolving it requires hands-on testing rather than further web search.