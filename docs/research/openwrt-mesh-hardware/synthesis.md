# OpenWrt True-Mesh Hardware — Research Synthesis

**Question:** Open-source-friendly hardware for a TRUE wireless mesh (802.11s/IBSS) deployment on OpenWrt — confirmed mainline support, strong mt76 (mt7915/mt7916) or ath9k/ath10k radio drivers, ≥2 radios (tri-radio ideal), compact/deployable, external antenna connectors, bare-board/SBC form factor for a custom enclosure.

**Context:** Replacing a closed MikroTik/Qualcomm fleet whose `wifi-qcom`/ath11k stack could not do real L2 mesh. Routing layer is L3 (babeld/static), so 802.11s only needs to bring up an L2 mesh link.

**Date:** 2026-06-23 · **Mode:** research · **Confidence:** High (driver verdict, hardware specs); Medium (per-board mesh reliability at a specific snapshot)

---

## 1. Executive summary

The single most important finding gates everything else: **the driver, not the board, is what failed you before — and the fix is well-established.** In current OpenWrt (24.10 / 25.x), **mt76 (mt7915/mt7916/mt7986 Filogic) and ath9k both genuinely support 802.11s mesh + IBSS + SAE-encrypted mesh.** ath10k works *if* you replace the `-ct` driver/firmware with the non-CT stock build. **ath11k / ath12k / wifi-qcom remain unreliable for mesh into 2026** — corroborating exactly the pain you hit. So: buy mt76 or ath9k/ath10k, and you are on safe ground.

On hardware, there is no single board that is simultaneously *fully open*, *current silicon*, and *multi-independent-radio* — you trade one axis. The practical landscape:

- **Best all-round modern pick — Banana Pi BPI-R3** (MT7986/mt76, onboard dual-band + mPCIe + M.2, U.FL, bare board, mainline). Meets the dual-radio minimum out of the box (2.4 + 5 GHz on separate `phy`s) and accepts a third radio card for the tri-radio ideal.
- **Best "start here / reference" buy — OpenWrt One** ($89, co-designed with the OpenWrt project, MT7981/mt76, schematics published, PoE, M.2). First-class support; the lowest-risk way to validate your mesh stack before committing a fleet.
- **Best true multi-independent-radio modern build — BPI-R4 + 2× AsiaRF AW7916 mPCIe cards** (two physically separate mt76 radios), at the cost of onboard-WiFi immaturity, higher power/thermal, and more integration work.
- **Maximum mesh reliability (WiFi 4/5 tradeoff) — Compex WPJ563 + WLE600VX** (onboard ath9k 2.4 GHz + ath10k 5 GHz mPCIe, in production), or any **mPCIe SBC + Compex WLE200NX (ath9k)**.
- **Open-hardware purist — LibreRouter LR1** (the only purpose-built 3-radio open-hardware ath9k mesh router with published EAGLE files) — but old silicon and chronically hard to buy.

---

## 2. The gating decision: driver / mesh-mode reality (read this first)

This is the crux of the prior failure, so it ranks above hardware. Verified against the OpenWrt 802.11s wiki, mt76 GitHub issues, and dated 2025–2026 forum/issue threads:

| Driver | Chips | 802.11s | IBSS | SAE-mesh | WiFi gen | Verdict |
|---|---|---|---|---|---|---|
| **ath9k** | AR9xxx | ✅ reference | ✅ reference | ✅ (`wpad-mesh-mbedtls`; `nohwcrypt=1` helps some boards) | 802.11n | **Safest, flawless** — but 2.4/5 GHz only, no WiFi-6 |
| **mt76** | mt7915 / mt7916 / mt7986 (Filogic) | ✅ (historical 5 GHz bugs fixed 2022–2024) | ✅ | ✅ (`wpad-mesh-mbedtls`) | WiFi-6/6E | **Recommended for WiFi-6.** Use 24.10+; pin firmware (watch open mt7916 5 GHz AP regression mt76 #912) |
| **ath10k** | QCA988x / QCA9984 / QCA9888 | ✅ **only with non-CT** firmware/driver | ⚠️ broken on QCA988x (use 802.11s, not IBSS) | ✅ | 802.11ac | Workable: stock `kmod-ath10k` + non-CT firmware; **avoid `ath10k-ct`**; Wave-2 safer than Wave-1 |
| **ath11k / ath12k / wifi-qcom** | IPQ60xx/80xx, QCN9074, IPQ53xx | ⚠️ advertised, unreliable in mainline | ⚠️ unstable | moot | WiFi-6/6E/7 | **Avoid for mesh.** STA/peer code paths crash (2025–2026 issues); real deployments use the QSDK fork. **This is what bit you.** |

**Operational rules that fall out of this (apply to every candidate below):**

1. **Install `wpad-mesh-mbedtls`** (or full `wpad`) for encrypted (SAE) mesh — the default `wpad-basic-mbedtls` silently fails to peer with encryption.
2. **Target 802.11s, not IBSS** — IBSS/ad-hoc is effectively dead on modern QCA988x firmware; 802.11s is the maintained path on all three good drivers.
3. **Keep the backhaul on a fixed, non-DFS 5 GHz channel.** 802.11s requires all peers on one channel; a DFS radar event forces a channel change that tears down the mesh. Avoid 160 MHz (DFS-only in most regions) and avoid 6 GHz for backhaul (range + AFC/regdb friction).
4. **Prefer a dedicated backhaul radio over mesh+AP-on-one-radio.** They *can* share a radio (same channel) but it's fragile across OpenWrt versions (a 24.10.0 regression broke same-radio mesh+AP). This is the core argument for ≥2 — ideally 3 — radios.
5. **L3 routing (your babeld setup) rides over 802.11s as a plain L2 switch** — no driver dependency; the driver only has to bring up the `mesh point` interface. Verify with `iw list | grep -A9 "Supported interface modes"` → expect `* mesh point`.

---

## 3. The "enough radios" nuance (important)

"Radio count" is ambiguous and worth pinning down, because it changes which boards qualify:

- A **single dual-band chip** (e.g. MT7976 in OpenWrt One / BPI-R3) presents as **two independent radios** in OpenWrt — `phy0` (2.4 GHz) and `phy1` (5 GHz) — on *different bands/channels*. **This already satisfies your "dual-radio minimum: one for 802.11s backhaul, one for AP/STA clients"** (e.g. mesh on 5 GHz, clients on 2.4 GHz).
- The **tri-radio ideal** — a *dedicated* 5 GHz backhaul radio **plus** a 2.4 GHz client radio **plus** a 5 GHz client radio — requires a *second 5 GHz radio*, i.e. an added mPCIe/M.2 card or a board with two separate WiFi chips.
- Putting both mesh-backhaul and client-AP on the **same 5 GHz radio** is the fragile case to avoid.

So single-chip boards are not disqualified — they meet the minimum. The mPCIe/M.2 slot is your upgrade path to the tri-radio ideal.

---

## 4. Master ranked comparison

Ranked by overall fit to *all* your requirements (driver safety × radio topology × bare-board/antenna/compact × OpenWrt maturity). Prices are rough USD.

| Rank | Candidate | Radios (driver) | Bands | Antenna conn. | Bare board? | OpenWrt | Mesh safety | Price | Best for |
|---|---|---|---|---|---|---|---|---|---|
| **1** | **Banana Pi BPI-R3** | Onboard MT7976 (2.4+5, mt76) **+ 1 mPCIe + M.2** | 2.4/5 (+add-in) | **U.FL/IPEX** | ✅ ~148×100 mm | Mainline (mature) | ✅ mt76 | ~$100–130 | Best all-round; meets minimum, upgradable to tri-radio |
| **2** | **OpenWrt One** | Onboard MT7976 (2.4+5, mt76) + M.2 | 2.4/5 | 3× **MMCX** (~500 cycles, robust) | ✅ 148×100.5 mm | **First-class** (reference board) | ✅ mt76 | **$89** | Lowest-risk fleet validation; PoE; supports OpenWrt project |
| **3** | **BPI-R4 + 2× AsiaRF AW7916-NPD** | **2 independent** mt7916 mPCIe (mt76) | 2.4/5(/6) per card | **U.FL** ×4/card | ✅ 100.5×148 mm | Snapshot (add-in card path OK; onboard buggy) | ✅ mt76 | ~$170 + ~$40–50/card | True multi-independent-radio WiFi-6; scale-up node |
| **4** | **Compex WPJ563 + WLE600VX** | Onboard ath9k 2.4 (3×3) **+ ath10k 5 GHz mPCIe** | 2.4/5 | **U.FL** | ✅ compact | Mainline + Compex branch | ✅✅ Atheros (use non-CT fw) | ~$60–90 + ~$25–45 | Max mesh reliability, in production, dual-band dual-radio |
| **5** | **PC Engines APU2/4 (or Noah4C) + AW7915-NP1 / Compex cards** | 2–3× mPCIe (mt76 **or** ath9k/ath10k, your choice) | per card | **U.FL** | ✅ (x86) | **Most solid PCIe** (mainline x86) | ✅ (driver of choice) | APU NOS ~$120–160 + cards | Max DIY flexibility & PCIe reliability; **APU is EOL/NOS** |
| **6** | **UniElec U7623 + AsiaRF AW7915-NP1** | 1× mPCIe mt7915 4T4R (mt76) | 2.4/5 | **U.FL** ×4 | ✅ | Working | ✅ **mesh point confirmed in `iw`** | ~$30 (card) | Cheapest mesh-confirmed mt76 add-in; needs 3.3 V buck |
| **7** | **BPI-R3 Mini** | Onboard MT7976 (2.4+5, mt76) + M.2 | 2.4/5 | 3× **U.FL** | ✅ **65×65×10 mm** | Snapshot/mainline | ✅ mt76 | ~$70–90 | **Most "deployable anywhere"**; USB-C PD; single chip |
| **8** | **LibreRouter LR1** | **3 radios** (ath9k 2.4 + 2× AR9582 5 GHz) | 2.4/5/5 | (open HW) | ✅ open HW, weatherproof | OpenWrt + LibreMesh | ✅✅ ath9k | intermittent | **Open-hardware purist**; 3-radio mesh-native; hard to buy, old silicon |
| — | **Compex WLE200NX (ath9k mPCIe)** | add-in 2.4 GHz (ath9k) | 2.4 | 2× U.FL | n/a (card) | Mature | ✅✅ **gold standard, no blob** | ~$15–35 | The single most reliable mesh radio, esp. for backhaul; near-EOL |
| ✗ | GL.iNet GL-MT6000 / Flint 2 | 2× MT7976 (mt76) | 2.4/5 | sealed | ❌ sealed case | Snapshot | ✅ mt76 | ~$150 | Rejected: sealed, hard to re-case |
| ✗ | NanoPi R3S/R4S/R5S | RTL USB (not mt76/ath) | — | — | ✅ | Rockchip | ✗ | ~$50–80 | Rejected: wrong radio stack |

---

## 5. Recommendations by priority

**If your priority is a safe, balanced fleet node (recommended default):**
→ **Banana Pi BPI-R3.** Mature mt76 in mainline, bare board with U.FL pigtails, onboard 2.4+5 GHz already gives you backhaul-radio + client-radio, and the mPCIe/M.2 slot lets you add a dedicated second 5 GHz radio later for the tri-radio ideal. The most "no regrets" choice.

**If your priority is de-risking before you buy a fleet:**
→ **OpenWrt One** ($89). Buy two, stand up an 802.11s link on `wpad-mesh-mbedtls`, prove your babeld-over-802.11s stack end-to-end. It's the reference board with the best support and published schematics; MMCX connectors are actually *more* durable than U.FL. Then scale with whichever board wins.

**If your priority is true dedicated radios per role, WiFi-6:**
→ **BPI-R4 + 2× AsiaRF AW7916-NPD** (one card = dedicated 5 GHz backhaul, one = client). Budget for thermals (these cards exceed 130 °C — heatsink mandatory), the 3.3 V @ 2.5–3 A per-card power draw, and snapshot-level software maturity. Verify both mPCIe slots enumerate two cards on your build before committing.

**If your priority is rock-solid mesh and you can accept WiFi 4/5:**
→ **Compex WPJ563 + WLE600VX** (all-Atheros, in production, dual-radio dual-band), or any mPCIe SBC + **Compex WLE200NX (ath9k)** for the single most reliable mesh radio. ath9k is the AREDN amateur-radio mesh standard for a reason. Use non-CT firmware for the ath10k card.

**If your priority is genuinely open hardware:**
→ **OpenWrt One** is the realistic answer (published schematics/datasheets, OpenWrt-project co-design, current mt76 silicon). **LibreRouter LR1** is the only purpose-built open *mesh* router (3 radios, EAGLE files), but verify 2026 availability/price before counting on it, and accept ath9k-era throughput.

---

## 6. Reference BOM for the recommended node (BPI-R3 class)

- Banana Pi BPI-R3 board (~$120) — onboard MT7976 mt76, mainline OpenWrt
- *(optional, for tri-radio)* AsiaRF AW7915-NP1 (mt7915, 4T4R) or AW7916-NPD (mt7916) mPCIe card (~$30–50) in the mPCIe slot as dedicated 5 GHz backhaul radio — budget 3.3 V @ ≥2.5 A and a heatsink
- U.FL → RP-SMA bulkhead pigtails: count = (streams per radio) × (radios). Onboard 2.4(2×2)+5(3×3) = 5; add 4 more for a 4T4R card
- High-gain antennas: **directional/sector (12–20+ dBi) on the backhaul radio**, **omni (2–8 dBi) on the client radio**
- Power: 802.3at PoE (single outdoor cable) or 12–24 V DC for solar/battery
- Software: OpenWrt 24.10+/25.x, `wpad-mesh-mbedtls`, fixed non-DFS 5 GHz backhaul channel, babeld on top
- Enclosure (your build): polycarbonate IP66–IP68 / NEMA 4X, Gore vent for condensation, conductive thermal path to the SoC, conformal-coated PCB for outdoor, IP-rated glands, Ethernet surge protector on outdoor runs

---

## 7. Antenna / RF / packaging checklist (custom enclosure)

1. Connector count = (streams per radio) × (number of radios). Order that many bulkhead pigtails.
2. Board jacks are U.FL/IPEX (~30 mating cycles, fragile) → route each through a **U.FL-to-RP-SMA pigtail to a panel-mount bulkhead**; never expose U.FL to handling/weather. (MMCX, as on OpenWrt One, is ~500 cycles and fine.)
3. Prefer **RP-SMA/SMA at the wall** (~500 cycles, threaded, O-ring weatherproof).
4. **Backhaul radio → directional/sector; client radio → omni.** Keep backhaul on its own radio/channel.
5. Power: **PoE 802.3at** as default single-cable outdoor; 12–24 V DC for solar; add an **Ethernet surge protector** outdoors.
6. Enclosure: polycarbonate **IP66–IP68 / NEMA 4X**; add a **Gore vent**; ensure a **conductive thermal path** for the SoC (don't just seal an indoor board — you trap heat and condensation).
7. **Conformal-coat** the PCB for outdoor humidity; **IP-rated cable glands** on all penetrations.
8. Keep pigtails short (RG178/RG316); mount bulkheads clear of metal that detunes the antenna.

---

## 8. Key risks & open questions (verify before fleet commitment)

- **mt76 5 GHz mesh at your exact snapshot.** The historical mt7915 5 GHz 802.11s bugs are *fixed* (mt76 #675 closed Apr 2024), but an **open** mt7916 5 GHz **AP** regression (mt76 #912, updated Feb 2026) touches the band you'd back-haul on. Pin a known-good firmware/snapshot and test 5 GHz `mesh point` specifically.
- **mesh+AP concurrency on one radio** is fragile across versions — design for a dedicated backhaul radio to sidestep it.
- **BPI-R4 dual-card stability** (two mPCIe mt7916 cards enumerating + powering simultaneously) is plausible but not confirmed in sources — validate on the bench.
- **Per-slot 3.3 V current budget** on BPI-R4 / NanoPi is undocumented; 4T4R mt76 cards want 2.5–3 A. UniElec U7623 needs an external 12 V→3.3 V buck. Measure before assuming.
- **ath10k on current OpenWrt 24.x**: confirm whether the CT→non-CT swap is still required for Wave-1 (QCA9882), or fixed.
- **OpenWrt One open-hardware completeness** (Gerbers/CAD + license vs. published PDFs) and whether its **M.2 slot is PCIe-capable for a WiFi NIC** (second independent radio) vs. SSD-only — unconfirmed.
- **LibreRouter 2026 purchasability/price** — historically intermittent.

---

## 9. Sources

Primary sources are cited inline in the per-hypothesis findings. Most load-bearing:

- OpenWrt 802.11s wiki — SAE works on all current drivers via `wpad-mesh-mbedtls`; `ath10k-ct` flagged unreliable for mesh (Dec 2023): https://openwrt.org/docs/guide-user/network/wifi/mesh/802-11s
- mt76 #675 (mt7915 5 GHz 802.11s) **closed/fixed** Apr 2024: https://github.com/openwrt/mt76/issues/675 · open regression #912: https://github.com/openwrt/mt76/issues/912
- OpenWrt forum interop thread (Jan 2024) — ath9k/ath10k/ath11k/mt76 mesh + interop, "mt76 significantly improved since mt7615": https://forum.openwrt.org/t/mesh-with-2-different-router-brands/184736
- ath11k instability (2025–2026): OpenWrt issues #20702, #19367, #22074; Phoronix 2026-04
- BPI-R4 / R3 / R3 Mini docs: https://docs.banana-pi.org/en/BPI-R4/BananaPi_BPI-R4 · https://wiki.banana-pi.org/Banana_Pi_BPI-R3 · CNX (R3 Mini, 3× U.FL)
- OpenWrt One: https://openwrt.org/toh/openwrt/one · https://one.openwrt.org/hardware/ · CNX ($89, 3× MMCX)
- AsiaRF AW7915-NP1 (mt7915, 4T4R, 4× IPEX): https://asiarf.com/product/wifi-6-11ax-4t4r-mini-pcie-module-mt7915-aw7915-np1/ · AW7916-NPD: https://asiarf.com/product/wi-fi-6e-mini-pcie-module-mt7916-aw7916-npd/
- AW7915-NP1 mesh-point confirmed on UniElec U7623: https://forum.openwrt.org/t/mt7915-pcie-cards-on-bpi-unielec-boards/118194
- Compex WLE600VX (QCA9882 ath10k, 2× U.FL, in production): https://compex.com.sg/shop/wifi-module/802-11ac-wave-1/wle600vx-wifi5-11ac-qca9882-qca9892/ · WPJ563: https://www.524wifi.com/index.php/compex-wpj563hv-dual-radio-gigabit-embedded-board-802-11ac.html
- WLE200NX (AR9287 ath9k, no blob): pcengines apu2 mpcie module docs · AREDN ath9k = mesh standard: https://www.arednmesh.org/content/porting-new-hardware
- ath10k-ct vs non-CT for 802.11s: https://forum.openwrt.org/t/ath10k-ct-wifi-driver-does-not-support-802-11s/125423
- PC Engines APU EOL: https://www.pcengines.ch/eol.htm · Noah4C successor: https://www.varia.org/en/noah4c-the-successor-to-the-apu-series/
- LibreRouter (3-radio open HW mesh): https://www.cnx-software.com/2020/01/29/librerouter-is-an-open-source-hardware-router-for-community-networks/
- Connector mating cycles (U.FL ~30, MMCX/SMA ~500): https://www.data-alliance.net/blog/antenna-jacks-ufl-mhf4-rpsma-sma-add-upgrade-antenna-to-vastly-increase-range
- Outdoor enclosure / thermal: https://www.polycase.com/techtalk/outdoor-electronic-enclosures/best-options-for-outdoor-network-enclosures.html

Per-hypothesis detail: `hypotheses/h1-mt76-bare-boards/`, `h2-ath9k-ath10k-boards/`, `h3-sbc-mpcie-cards/`, `h4-driver-mesh-reality/`, `h5-openhw-antenna-rf/`.

---

## 10. Verification

- **Coverage:** All six hard requirements addressed for every ranked candidate (driver/mesh, radio count/topology, OpenWrt support, antenna connectors, bare-board form factor, open-hardware preference). ✅
- **Citation audit:** Every load-bearing claim traces to a primary source (OpenWrt wiki, vendor docs, dated GitHub issues / forum threads). ✅
- **Contradiction check:** H1 framed single-chip boards as not meeting "≥2 radios"; reconciled in §3 — a dual-band chip = two radios on separate bands, which *does* meet the dual-radio minimum; the tri-radio ideal needs an added card. Resolved, not silently dropped.
- **Key uncertainty:** Per-board 5 GHz `mesh point` reliability at a specific OpenWrt snapshot requires hands-on testing — flagged in §8, drives the "buy OpenWrt One to validate first" recommendation.
- **Status:** PASS_WITH_WARNINGS (warnings = the snapshot-specific mesh reliability and second-radio/power items in §8).
