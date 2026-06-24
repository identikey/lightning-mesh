# Caseless / Bare-PCB OpenWrt Mesh Router Boards (Pass 2)

**Research date:** 2026-06-24
**Goal:** Find currently-buyable **bare-PCB / caseless** router boards that are **dual-band**, carry **≥2 independent radios** (3 ideal), use WiFi chips with **OpenWrt mainline drivers** that support **802.11s mesh** (mandatory), with **external antenna connectors** (U.FL/IPEX or RP-SMA/MMCX). **WiFi 6 preferred.** User intends to fabricate their own enclosure, so a naked PCB is a *bonus*; sealed consumer routers are disqualified.

> **Mesh driver note:** All MediaTek `mt76` chips named here (MT7976, MT7916, MT7915, MT7995, plus legacy MT7612/MT7603) support `mesh point` mode in mainline OpenWrt. So do Qualcomm `ath9k`/`ath10k` chips on Compex boards. 802.11s requires `wpad-mesh-openssl` / `wpad-mesh-wolfssl` (replacing default `wpad-basic`). mt76 802.11s works but has historically had rough edges (encrypted-mesh and throughput-regression bugs reported on 23.05 with mt76; see openwrt/mt76 issues #72, #259, #12905). It is functional and actively maintained, but plan to test on a current snapshot. ath9k/ath10k 802.11s is the most mature/battle-tested mesh path.

---

## Ranked Caseless Board Table

Rank = fit to: **caseless + dual-band + ≥2 radios (3 ideal) + 802.11s + WiFi 6.**

| # | Board | SoC | WiFi chipset(s) | Radio count | Bands | Antenna conn. | OpenWrt | 802.11s | Form factor | Power | Price (USD) | Ships caseless? |
|---|-------|-----|-----------------|-------------|-------|---------------|---------|---------|-------------|-------|-------------|-----------------|
| 1 | **Banana Pi BPI-R3** | MediaTek MT7986A (Filogic 830) | MT7976 (onboard, dual-band) + **2x M.2/mPCIe** for a 3rd radio | **2 onboard** (2.4G 2x2 + 5G 3x3); **3+ with M.2 card** | 2.4 + 5 GHz (+6 GHz via card) | 6x IPEX/U.FL onboard | **Mainline** (`mediatek/filogic`) | Yes (mt76) | 148.5 x 100.5 mm | 12 V DC (5.5/2.1 barrel) | ~$95–110 board-only | **YES** (board-only SKU; case is optional bundle) |
| 2 | **Banana Pi BPI-R4** | MediaTek MT7988A (Filogic 880) | **No onboard radio** — WiFi via NIC card. BE14 card = MT7995AV + MT7976CN + MT7977IAN (tri-band WiFi 7) | 0 onboard; **2–3 via NIC-BE14**; 2x mPCIe (PCIe3 x2) for more | 2.4 + 5 + 6 GHz (with BE14) | up to 14x IPEX on BE14 card | **Mainline/snapshot** (`mediatek/filogic`) | Yes (mt76) | 148.5 x 100.5 mm | 12 V DC barrel | board ~$95 + BE14 ~$74 + antennas ~$14 | **YES** (board-only; needs big heatsink) |
| 3 | **OpenWrt One** (official) | MediaTek MT7981B (Filogic 820) | MT7976C (onboard, dual-band) | **2** (2.4G 2x2 + 5G 3x3/2x2, zero-wait DFS) | 2.4 + 5 GHz | 3x **MMCX** onboard (+RP-SMA pigtails) | **Mainline** (its own target; flagship dev board) | Yes (mt76) | 148 x 100.5 mm | USB-C PD / 12 V; PoE module | $89 (bundle w/ case) | **Partly** — designed as a bare dev board, but retail bundle includes a blue metal case. Case is removable/optional; the PCB is the product. |
| 4 | **Banana Pi BPI-R3 Mini** | MediaTek MT7986A (Filogic 830) | MT7976C (onboard, dual-band) | **2** (2.4G 2x2 + 5G 2x2/3x3) | 2.4 + 5 GHz | 3–4x IPEX/U.FL onboard | **Mainline** (`mediatek/filogic`) | Yes (mt76) | **65 x 65 mm** (smallest) | 12 V via USB-C PD | ~$70–80 board-only | **YES** (board-only SKU; case optional) |
| 5 | **AsiaRF / 524wifi mPCIe radio cards** (AW7916-NPD etc.) — pair with a host board | host SoC dependent | MT7916AN (DBDC; some bundle MT7976 front-end) | 1 card = **2 concurrent** (2.4G 2x2 + 5/6G 2x2/3x3) | 2.4 + 5 + 6 GHz | U.FL on card | **Mainline** (mt76 `mt7915` dir; supported since 21.02) | Yes (mt76) | mPCIe / M.2 A-E | host | ~$40–60/card | **YES** (bare module). Not a router on its own — host needed. |
| 6 | **Compex WPJ563 / WPJ344 / WPQ864** (+ WLE radio modules) | Qualcomm IPQ40xx / IPQ806x | Onboard ath9k 2.4G + mPCIe ath10k 5G (e.g. WLE600VX/QCA9882, WLE900VX/QCA9880) | **2** (1 onboard 2.4G + 1 mPCIe 5G); WPQ864 supports more slots | 2.4 + 5 GHz (802.11ac) | 3x U.FL onboard + 3x U.FL/module | **Mainline** (`ipq40xx` / `ipq806x`) | Yes (ath9k/ath10k — **most mature mesh**) | embedded PCBA | 12 V / PoE | WPJ563 ~$100; WPQ864 ~$240 (+modules) | **YES** (industrial bare PCBA). WiFi 5 only (no WiFi 6). |
| 7 | **Noah4C** (PC Engines APU successor, Varia) / **Noah6** | Intel Atom E3845 (x86-64) | none onboard — 2x mPCIe + 1x M.2 B-key for radio cards | **0 onboard; 2–3 via cards** | depends on cards (use mt76 mPCIe = 2.4+5+6 GHz) | on radio cards | **Mainline x86-64**; runs pfSense/OPNsense too | Yes (via mt76/ath cards) | bare board, fanless heatsink | 12 V DC | ~$200+ board-only | **YES** (bare board). Heaviest, highest power draw. |
| 8 | **UniElec U7623-02 / AsiaRF AP7623** | MediaTek MT7623A (Cortex-A7) | none onboard — 2x mPCIe | **0 onboard; 2 via cards** (MT7612/MT7615/MT7916 mPCIe) | 2.4 + 5 GHz with cards | on cards | **Mainline** (`mediatek/mt7623`) | Yes (mt76) | bare board | 12 V DC | ~$80–120 + cards | **YES** (bare board). Older A7 SoC, WiFi 5-class cards typical. |

---

## Detailed Board Notes

### 1. Banana Pi BPI-R3 — *best overall fit*
- **SoC:** MT7986A (Filogic 830), quad Cortex-A53, 2 GB DDR4, 8 GB eMMC + 128 MB SPI-NAND.
- **Radios:** Onboard **MT7976** = full dual-band (2.4 GHz 2x2 + 5 GHz up to 4x4), one driver instance per band = **2 independent radios out of the box.** Two M.2/mPCIe slots (one is the WiFi slot, one Key-M for NVMe) let you add a third radio (e.g., an MT7916 6 GHz card) for tri-radio.
- **Antenna:** 6x IPEX/U.FL onboard (3 per band) → user supplies pigtails to RP-SMA. Perfect for a custom enclosure.
- **OpenWrt:** Mainline, `mediatek/filogic` target, very active. ToH-style entry: https://openwrt.org/toh/banana_pi/bpi-r3 (page sparse; firmware-selector has it under mediatek/filogic).
- **802.11s:** Yes via mt76. **Ports:** 2x 2.5GbE SFP + 5x GbE.
- **Price:** Board-only ~$95–110; bundles with case/antennas higher.
- **Caseless:** **Yes** — sold as a naked PCB; metal case is a separate bundle option.

### 2. Banana Pi BPI-R4 — *most radios / WiFi 7, but radio is a separate card*
- **SoC:** MT7988A (Filogic 880), quad Cortex-A73 @1.8 GHz, 4/8 GB DDR4, 8 GB eMMC.
- **Radios:** **None onboard.** WiFi comes from a NIC card in one of the **2x mPCIe (PCIe 3.0 x2)** slots. The official **BPI-R4-NIC-BE14** card carries three chips: **MT7995AV (WiFi 7 controller) + MT7976CN (2.4/5 GHz) + MT7977IAN (6 GHz)** → effectively **tri-band, up to 3 radio instances** on one card. You could populate both mPCIe slots for even more radios.
- **Antenna:** up to 14x IPEX sockets on the BE14 card.
- **OpenWrt:** Mainline/snapshot `mediatek/filogic`. ToH URL: https://openwrt.org/toh/banana_pi/bpi-r4 .
- **802.11s:** Yes via mt76 (WiFi 7 mesh support maturing; WiFi 6 fallback solid).
- **Ports:** 2x 10GbE SFP+ + 4x GbE. **Power:** 12 V barrel. **Needs a large heatsink.**
- **Price:** board ~$95 + BE14 ~$74 + antennas ~$14.
- **Caseless:** **Yes** — bare board.

### 3. OpenWrt One — *the "official" dev board, 3x MMCX*
- **SoC:** MT7981B (Filogic 820), dual Cortex-A53 @1.3 GHz, 1 GB DDR4, 256 MB SPI-NAND + 16 MB backup.
- **Radios:** Onboard **MT7976C** dual-band → **2 radios** (2.4 GHz 2x2 + 5 GHz 3x3/2x2 with zero-wait DFS). No expansion radio slot (M.2 is NVMe-only).
- **Antenna:** **3x MMCX** onboard (ships with RP-SMA pigtail antennas).
- **OpenWrt:** First board co-designed with the OpenWrt project; rock-solid mainline support, guaranteed long-term. Firmware-selector id `mediatek/filogic glinet... ` → search "OpenWrt One".
- **802.11s:** Yes via mt76. **Ports:** 1x 2.5GbE WAN + 1x GbE LAN; USB 2.0; mikroBUS; PoE module option.
- **Dimensions:** 148 x 100.5 mm. **Power:** USB-C PD / 12 V.
- **Price:** $89 retail bundle (board + blue metal case + PoE module + 3 antennas + PSU).
- **Caseless:** **Partly.** It is fundamentally a bare dev board, but the retail unit ships *with* a removable blue metal enclosure. You can simply discard/omit the case for your own design; there is no listed "PCB-only at lower price" SKU. Only 2 radios (no 3rd-radio expansion).

### 4. Banana Pi BPI-R3 Mini — *smallest, cleanest 2-radio board*
- **SoC:** MT7986A (Filogic 830), 2 GB DDR4, 8 GB eMMC.
- **Radios:** Onboard **MT7976C** dual-band → **2 radios** (2.4 GHz 2x2 + 5 GHz). M.2 slot is for NVMe/4G, not a 2nd WiFi radio (so realistically 2 radios).
- **Antenna:** 3–4x IPEX/U.FL onboard.
- **OpenWrt:** Mainline `mediatek/filogic`. Forum/ToH active.
- **802.11s:** Yes via mt76. **Ports:** 2x 2.5GbE SFP. **Dimensions: 65 x 65 mm** (by far the most enclosure-friendly footprint). **Power:** 12 V via USB-C PD.
- **Price:** ~$70–80 board-only.
- **Caseless:** **Yes** — board-only SKU; case optional bundle.

### 5. AsiaRF / 524wifi mPCIe & M.2 radio cards (companions, not standalone)
- **AW7916-NPD (mPCIe) / AW7916-AED (M.2 A-E):** MT7916AN, DBDC, 2.4 GHz 2x2 + 5/6 GHz 2x3 → **2 concurrent radios per card**, WiFi 6/6E.
- **OpenWrt:** mainline mt76 (`mt7915` directory covers MT7916), supported since 21.02; **802.11s yes.**
- **Use:** these are the cards you drop into BPI-R3/R4, U7623, or x86 boards to add radios. ~$40–60 each. **Bare module — yes.**

### 6. Compex WPJ/WPQ series (Qualcomm, WiFi 5, most mature mesh)
- **WPJ563:** IPQ4019, onboard ath9k 2.4 GHz 3x3 + mPCIe slot for an ath10k 5 GHz module (WLE600VX/QCA9882 2x2 or WLE900VX/QCA9880 3x3) → **2 radios.** 3x U.FL onboard + module U.FL.
- **WPQ864:** IPQ806x, 1.4 GHz, more mPCIe slots → can host multiple radios; ~$240.
- **OpenWrt:** mainline `ipq40xx` / `ipq806x`. **802.11s:** Yes — **ath9k/ath10k is the most battle-tested OpenWrt mesh stack.**
- **Caveat:** **WiFi 5 only (802.11ac), no WiFi 6.** Industrial bare PCBA — ships caseless **yes.** Pick this if mesh stability matters more than WiFi 6.

### 7. Noah4C / Noah6 (x86 APU successors — Varia / Rack-Matrix)
- **Noah4C:** Intel Atom E3845 quad-core, 4x Intel i210 GbE, **2x mPCIe + 1x M.2 B-key + 2x mSATA**, fanless. **Noah6** is the APU6 alternative.
- **Radios:** none onboard; install 2–3 mt76 (AW7916) or ath10k mPCIe cards → **2–3 radios, dual/tri-band, WiFi 6 possible.**
- **OpenWrt:** mainline **x86-64** (also runs pfSense/OPNsense). **802.11s:** via the cards.
- **Caseless:** **Yes** — bare board. Trade-offs: higher cost (~$200+), higher power draw, larger; best where x86 horsepower/flexibility is wanted.

### 8. UniElec U7623-02 / AsiaRF AP7623
- **SoC:** MT7623A (quad Cortex-A7). **2x mPCIe** for radios (no onboard WiFi). Historically paired with MT7612 (5 GHz) + MT7603 (2.4 GHz) → 2 radios, or modern MT7916 cards.
- **OpenWrt:** mainline `mediatek/mt7623` (U7623-02 patches upstreamed). **802.11s:** yes via mt76. Bare board — **yes.** Older platform; fine as a budget 2-radio mesh node but slower than Filogic boards.

---

## GL.iNet status (mostly disqualified — sealed)
GL.iNet routers (GL-MT6000 "Flint 2" = MT7986+MT7976 WiFi 6; GL-MT3000 "Beryl AX" = MT7981) run OpenWrt-based firmware and support 802.11s, **but all ship as sealed consumer routers in plastic/metal enclosures** — they are **not** bare boards and are **disqualified** by the caseless requirement. The one GL.iNet *bare board* product is the **GL-M2 ("Dev Board")**, a **5G/modem development carrier** — it is not a multi-radio WiFi router board, so it does not fit this use case. If you want the MT7986+MT7976 silicon as a bare PCB, buy the **BPI-R3** (same chips) instead of opening a Flint 2.

---

## Top Picks

1. **Banana Pi BPI-R3 — primary recommendation.** True bare PCB, **2 independent onboard radios** (dual-band MT7976) with **6x U.FL** ready for your own antennas, **mainline** OpenWrt on the actively maintained `mediatek/filogic` target, 802.11s via mt76, ~$95. An M.2 slot lets you reach **3 radios** later (add a 6 GHz MT7916 card). Best balance of caseless + dual-band + ≥2 radios + WiFi 6 + mesh.

2. **Banana Pi BPI-R3 Mini — best when size matters.** Same MT7986+MT7976 silicon, same mesh story, but **65x65 mm** — trivial to design a small sealed enclosure around. 2 radios, ~$75, bare board. Pick this for compact mesh nodes; pick the full R3 when you want the extra Ethernet/SFP and the expansion radio slot.

3. **OpenWrt One — best for guaranteed long-term software support.** Official OpenWrt-project board, MT7976 dual-band (2 radios), **3x MMCX** connectors, mainline-forever. Only caveat: ships *with* a (removable) case rather than as a price-reduced naked PCB, and no 3rd-radio expansion. Choose it if mainline longevity/official support outranks the strict "naked PCB" bonus.

4. **For 3 radios / WiFi 7:** **BPI-R4 + NIC-BE14** gives tri-band (2.4/5/6 GHz) and up to 3 radio instances on one card, bare board, mainline — at the cost of needing the add-in card + big heatsink and a higher total (~$180 w/ card+antennas).

5. **For maximum mesh stability (accepting WiFi 5):** **Compex WPJ563** with an ath10k 5 GHz module — bare industrial PCBA, the most mature OpenWrt 802.11s stack (ath9k/ath10k). Use only if you can live without WiFi 6.

**Bottom line:** Standardize on the **BPI-R3 (large nodes) + BPI-R3 Mini (compact nodes)** — identical MT7986/MT7976 silicon means one firmware/mesh config across the fleet, both ship as naked PCBs with U.FL connectors for your custom enclosure, both are mainline with mt76 802.11s. Validate 802.11s on a current OpenWrt snapshot before committing (known historical mt76 mesh quirks).

---

## Sources
- Banana Pi BPI-R4 docs / CNX Software: https://www.cnx-software.com/2024/07/09/banana-pi-bpi-r4-nic-be14-wifi-7-dual-mini-pcie-module-for-banana-pi-bpi-r4-sbc/ ; https://wiki.banana-pi.org/Banana_Pi_BPI-R4 ; https://openwrt.org/toh/banana_pi/bpi-r4
- Banana Pi BPI-R3 wiki/docs: https://wiki.banana-pi.org/Banana_Pi_BPI-R3 ; https://openwrt.org/toh/banana_pi/bpi-r3
- Banana Pi BPI-R3 Mini: https://www.cnx-software.com/2023/07/26/banana-pi-bpi-r3-mini-low-profile-2-5gbe-router-board-mediatek-filogic-830-soc/ ; https://docs.banana-pi.org/en/BPI-R3_Mini/BananaPi_BPI-R3_Mini
- OpenWrt One: https://www.cnx-software.com/2024/10/02/buy-openwrt-one-wifi-6-router-filogic-820-soc/ ; https://docs.banana-pi.org/en/OpenWRT-One/BananaPi_OpenWRT-One ; https://wikidevi.wi-cat.ru/OpenWrt_One
- Compex boards: https://wiki.compex.com.sg/wiki/802.11ac_Wave_2_boardData ; https://www.524wifi.com/index.php/embedded-cpu-boards/dual-radio-boards.html
- AsiaRF / 524wifi MT7916 cards: https://asiarf.com/product/wi-fi-6e-mini-pcie-module-mt7916-aw7916-npd/ ; https://www.524wifi.com/index.php/mt7916porting
- UniElec U7623: https://openwrt.org/toh/asiarf/ap7623-a02 ; https://patchwork.ozlabs.org/patch/931872/
- Noah4C (PC Engines APU successor): https://www.varia.org/en/noah4c-the-successor-to-the-apu-series/ ; https://www.rack-matrix.com/en/blog-whats-new/news/item/noah6-the-replacement-alternative-for-the-apu6-board-from-pc-engines.html
- GL.iNet: https://www.gl-inet.com/products/datasheet/ ; https://www.gl-inet.com/en-us/products/gl-m2
- mt76 802.11s status: https://github.com/openwrt/mt76/issues/259 ; https://github.com/openwrt/openwrt/issues/12905 ; https://forum.openwrt.org/t/the-802-11s-mesh-with-openwrt-success-stories/119502
