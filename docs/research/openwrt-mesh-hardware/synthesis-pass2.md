# OpenWrt Mesh Hardware — Pass 2 Synthesis (chips, caseless boards, tri-radio)

**Date:** 2026-06-24 · Complements `synthesis.md` (pass 1). Focus this round: a full **WiFi-6 chip × OpenWrt-driver × 802.11s** catalog, **caseless/bare-PCB** boards, and concrete **tri-radio** recipes. 802.11s treated as mandatory.

Detail files: `pass2/chip-driver-catalog.md`, `pass2/caseless-boards.md`, `pass2/tri-radio-recipes.md`.

---

## TL;DR answers to your three asks

1. **Which radio chips have an OpenWrt driver + 802.11s (ideally WiFi 6)?** A short list. For WiFi 6 mesh, it is essentially **MediaTek mt76 AP silicon only**: **mt7986, mt7981(+mt7976), mt7916, mt7915**. For reliability-over-speed, **Qualcomm ath9k** (WiFi 4) and **ath10k on non-CT firmware** (WiFi 5). **Everything else is out** — see the "avoid" list, which is large and includes the exact family that failed you.

2. **Dual-band boards, ideally 3 radios?** Yes — and the key realization is that a **dual-band chip = two independent OpenWrt radios**, and a **DBDC add-in card = two more**. So a BPI-R3 (dual-band onboard = 2 radios) **+ one DBDC mt76 card = 4 radios**. You don't need three card slots to get three radios.

3. **Caseless boards you can build a case around?** The whole MediaTek Filogic dev-board line ships as bare PCBs: **BPI-R3, BPI-R3 Mini, BPI-R4** (and the OpenWrt One, which ships with a *removable* case). All expose U.FL/IPEX (or MMCX) for your own antennas.

**Single best pick: Banana Pi BPI-R3** (bare PCB, MT7986/mt76, 2 onboard radios + 6× U.FL, mainline) — as a dual-radio fleet node, or **+ one DBDC mt7916 card for a tri-radio node**. **BPI-R3 Mini** (65×65 mm) for compact nodes on the same silicon.

---

## 1. The chip catalog (the part you specifically asked for)

The decisive test is whether the **mainline driver registers `NL80211_IFTYPE_MESH_POINT`** (visible as `* mesh point` in `iw list`). Verified from driver source + real `iw list` dumps:

| Chip | Driver | WiFi gen | Bands | 802.11s | Verdict |
|---|---|---|---|---|---|
| **mt7986** | mt76 | 6 (ax) | 2.4/5 | **YES** | **Top WiFi-6 mesh pick.** Confirmed SAE mesh (BPI-R3 pairs). "6E" branding is false — no 6 GHz radio. |
| **mt7981 + mt7976** | mt76 | 6 (ax) | 2.4/5 | **YES** | Cheap, confirmed mesh (Xiaomi AX3000T). Disable 802.11r on kernel 6.12 (5 GHz phy bug). |
| **mt7916** | mt76 | 6E (ax) | 2.4/5/6 | **YES** | Only solid mainline path to **6 GHz mesh** — but verify your build exposes 6 GHz (#17632). |
| **mt7915** | mt76 | 6 (ax) | 2.4/5 | **YES** | Works; most bug-reported mt76 mesh target. Prefer mt7986/mt7981. |
| **ath9k** (AR9380/9580) | ath9k | 4 (n) | 2.4/5 | **YES (reference)** | Gold-standard mesh, no firmware blob. WiFi-4 speeds only. |
| **ath10k** (QCA9984/988x) | ath10k | 5 (ac) | 2.4/5 | **CONDITIONAL** | Mesh only on **non-CT** firmware (OpenWrt ships CT by default = no mesh). |
| mt7990 / mt7996 (mt7988) | mt76 (mt7996e) | 7 (be) | 2.4/5(/6) | conditional / immature | Driver declares mesh; degradation bug #1065, lockups. Snapshot-only. Validate first. |
| ath12k (WCN7850…) | ath12k | 7 (be) | 2.4/5/6 | experimental | Mesh in snapshots; no MLO; bleeding-edge. |
| **mt7921 / mt7922 / mt7925** | mt76 | 6/6E/7 | — | **NO** | **Client (Connac) silicon — no mesh, no IBSS in hardware.** Common trap on cheap M.2 cards. |
| **ath11k / wifi-qcom** | ath11k | 6/6E | — | **AVOID** | **This is what bit you.** Mesh advertised but ax link silently downgrades to VHT80 (open bug #19805, Aug 2025); closed firmware = no swap escape. |
| RTL8852A/B/C (rtw89), 88xx (rtw88) | rtw88/89 | 5/6/6E | — | **NO** | No mesh point; weak/unstable AP. Avoid. |
| Intel AX200/AX210/BE200 | iwlwifi | 6/6E/7 | — | **NO** | Client-only; `iw type mp` → "Operation not supported"; AP crippled to 2.4 GHz. |
| Broadcom (all) | brcmfmac/b43/wl | varies | — | **NO** | Mesh explicitly rejected (`-EOPNOTSUPP`); closed firmware. Avoid. |

**Why you failed before, precisely:** ath11k *does* advertise `mesh point` — so it wasn't a missing capability. The real fault is OpenWrt's `hostapd.sh`/`wpa_supplicant_set_fixed_freq()` silently downgrading ax mesh links to VHT80 (bug #19805, still open Aug 2025), plus multicast/data-path issues and driver instability — and because the firmware is closed, there's no CT-style alternative to swap in. Standardizing on mt76 sidesteps all of it.

**Important correction to pass 1 / common marketing:** **mt7986 and mt7981 are dual-band 2.4/5 GHz only — NOT WiFi 6E.** They have no 6 GHz radio, so 6 GHz mesh on a BPI-R3/R3-Mini/OpenWrt-One is physically impossible. Only **mt7916** (or WiFi-7 mt7996) gives real 6 GHz.

---

## 2. Radio counting: dual-band chip = two radios; DBDC card = two more

This reframes the "3 radios" goal and is the most useful structural insight of pass 2:

- A dual-band SoC radio (MT7976 on BPI-R3 / OpenWrt One) enumerates as **two independent mt76 PHYs** — `phy0` 2.4 GHz + `phy1` 5 GHz. That already meets the **dual-radio minimum** (mesh on one band, clients on the other).
- A **DBDC mPCIe/M.2 card** (AsiaRF AW7915-NPD / AW7916-NPD) is likewise **two independent PHYs**.
- So: **BPI-R3 onboard (2) + one DBDC card (2) = 4 radios** — use three: dedicated 5 GHz backhaul + 2.4 GHz clients + second 5/6 GHz clients. **Two slots is plenty; you never need a 3-card host.**

Caveat: the two PHYs on one DBDC chip share that chip's CPU/DMA. For a rock-solid dedicated backhaul, put the **backhaul on its own physical chip** (the onboard radio, or a separate card) and let a single DBDC card carry the two client bands.

---

## 3. Caseless / bare-PCB boards (ranked)

| Rank | Board | Radios (onboard) | Bands | Antenna | Caseless? | Price | Note |
|---|---|---|---|---|---|---|---|
| **1** | **Banana Pi BPI-R3** | 2 (MT7976 dual-band) + M.2 for 3rd | 2.4/5 | 6× U.FL | **Yes** (bare SKU) | ~$95–110 | Best all-round; mainline filogic |
| **2** | **BPI-R3 Mini** | 2 (MT7976) | 2.4/5 | 3–4× U.FL | **Yes** | ~$70–80 | **65×65 mm** — best for compact custom case |
| **3** | **OpenWrt One** | 2 (MT7976C) | 2.4/5 | 3× MMCX | Partly (removable case) | $89 | Official board; longest-term support; no 3rd-radio slot |
| **4** | **BPI-R4 + NIC-BE14 / mt7916 cards** | 0 onboard → 3–4 via cards | 2.4/5/6 | up to 14× U.FL | **Yes** | ~$95 + cards | WiFi 7 / tri-band; needs big heatsink |
| **5** | **Compex WPJ563 (+WLE ath10k card)** | 2 (ath9k + ath10k) | 2.4/5 | U.FL | **Yes** (industrial PCBA) | ~$100+ | Most mature mesh; **WiFi 5 only** |
| — | **Noah4C / UniElec U7623** | 0 → via mPCIe cards | per card | on cards | **Yes** | ~$80–200 + cards | x86 (Noah) / older MT7623 host options |

GL.iNet (Flint 2 etc.) are disqualified — sealed consumer units. If you want that MT7986/MT7976 silicon bare, buy the BPI-R3.

---

## 4. Recommended builds

**Dual-radio fleet node (simplest, recommended default):**
→ **BPI-R3** (or **BPI-R3 Mini** for compact). Onboard MT7976 = mesh on 5 GHz + clients on 2.4 GHz out of the box. All mt76, mainline, U.FL for your antennas, ~$75–110. Same silicon across both = one firmware/mesh config for the whole fleet.

**Tri-radio node (the "3 radios" goal):**
→ **BPI-R3 + one AsiaRF AW7916-NPD (or AW7915-NPD) DBDC card** → 4 radios, use 3: onboard 5 GHz = dedicated 802.11s backhaul (fixed non-DFS channel); onboard 2.4 GHz = clients; card 5/6 GHz = second client AP. Only one card = only one 3.3 V/3 A power concern. *Check the R3 M.2 slot is wired/powered for a WiFi card (vs NVMe-only) and budget an M.2↔mPCIe adapter.*

**Tri-radio with cleaner backhaul isolation + 10G uplink:**
→ **BPI-R4 + 2× DBDC mt7916 cards** = 4 radios across two physical chips. Budget two well-powered 3.3 V/3 A slots + active cooling. (Avoid the BE14 module if you want a slot free per chip — it spans both mPCIe slots.)

**Max mesh reliability, WiFi-5 acceptable:**
→ **Compex WPJ563 + WLE600VX** (ath9k + ath10k non-CT) — the most battle-tested 802.11s stack.

**Open-hardware purist:** **LibreRouter LR1** (3× ath9k radios, 802.11s-native, open design) — but WiFi 4/5 only and email/batch ordering. The WiFi-6 successor **LRr2** is not buyable in 2026 yet.

---

## 5. Operational reminders (carried from pass 1, reconfirmed)

- Install **`wpad-mesh-openssl`** (or `-wolfssl` on constrained flash) — default `wpad-basic` can't do SAE mesh.
- **Keep backhaul on a fixed non-DFS 5 GHz channel** — pass-2 evidence reinforces this is exactly where mt76 mesh has been weakest (channel/DFS/bandwidth handling).
- **Dedicate a PHY to mesh**; don't share mesh+AP on one radio if you can avoid it (that's the point of going tri-radio).
- **Validate `iw list | grep -A12 "Supported interface modes"` shows `* mesh point`** on the exact board/snapshot before fleet commit, and bench-test 5 GHz mesh — the mt76 bug-tail is real but version-specific.

---

## 6. Verification

- Coverage: chip-level catalog spans all 6 driver families (mt76, ath9k/10k/11k/12k, rtw88/89, iwlwifi, brcm); board sweep covers bare-PCB options; tri-radio covered via onboard+card / dual-card / open-hardware paths. ✅
- Every chip verdict traces to driver source and/or a real `iw list` dump (primary sources in `pass2/chip-driver-catalog.md`). ✅
- Corrections logged vs pass 1: mt7986/mt7981 are **not** 6E (no 6 GHz radio); mt7921/22/25 do **not** do mesh; ath11k mesh fails via the #19805 HE→VHT80 downgrade, not a missing capability. ✅
- Status: **PASS_WITH_WARNINGS** — warnings = mt76 snapshot-specific mesh reliability + R3 M.2-slot-for-WiFi-card confirmation + multi-card power/thermals.
