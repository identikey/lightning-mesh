# Budget Hardware Synthesis — In-Stock OpenWrt 802.11s Routers Under ~$150 (Pass 3)

**Date:** 2026-06-24 · Complements `synthesis.md` (pass 1) and `synthesis-pass2.md` (pass 2). This round is about **price and availability**: the BPI-R3 Mini is great but unbuyable at a sane price, so what do you actually order *today*?

Detail files: `mt7981-cheap-routers.md`, `mt7986-budget-and-bareboard.md`, `ath9k-ath10k-cheap.md`, `glinet-ecosystem.md`.

---

## TL;DR — buy one of these

| Pick | Chip | Why | Price | Antennas | Where |
|---|---|---|---|---|---|
| **GL.iNet Flint 2 (GL-MT6000)** | **mt7986** | The real BPI-R3 replacement — *same MT7986+MT7976 silicon*, most-mature mt7986 OpenWrt target, confirmed 802.11s, 1 GB RAM, 2.5GbE, **in stock** | ~$140–170 (sales ~$140) | 4 external, **fixed** | gl-inet.com / Amazon |
| **Cudy WR3000** | mt7981 | Cheapest *easy* fleet node — official OpenWrt, **web-UI flash**, Amazon stock | **~$30–40** | 4 external, fixed | Amazon US |
| **CMCC RAX3000M** | mt7981 | Cheapest *with external antennas* (non-removable) | **~$25–40** | 4 external, fixed | AliExpress |
| **GL.iNet GL-A1300 (Slate Plus)** | ath10k | Best cheap node with **REMOVABLE RP-SMA** antennas (for high-gain later) | **~$60–70** | 2 detachable | gl-inet / Amazon |
| **Xiaomi Redmi AX6000** | mt7986 | Cheapest true mt7986, 6 antennas — but 512 MB RAM + fiddly flash | ~$90–100 | 6 external, fixed | AliExpress |

---

## 1. The BPI-R3 Mini reality (confirmed: stop waiting)

Your instinct was right. Across reputable Western resellers in June 2026, the **BPI-R3 Mini is out of stock or withdrawn** (ameriDroid sold out, eBay out, TME withdrawn). Where it's listed (Amazon 3P, AliExpress), street price is **~$180–200+** — a scarcity markup over its ~$80 list, which is not an attainable price right now. The full BPI-R3 is marginally easier to source (~$110 list) but still constrained and is a bigger board needing case + antennas + PSU.

**The key realization that frees you from the BPI-R3 premium:** the radio capability is **identical across the whole mt7981/mt7986 family** — same `mt76` driver (`kmod-mt7915e` + `kmod-mt798x-firmware`), same 802.11s behavior, **same mt76 mesh bug-tail**. A premium bare board does *not* buy you better mesh than a $35 Cudy or a $150 Flint 2. So optimize for **price, antennas, OpenWrt maturity, and stock** — not for the "premium" board.

> The mt76 802.11s caveats (5 GHz throughput drop with a mesh VIF — openwrt#12905; 2.4 GHz mesh negotiating only 20 MHz — openwrt#13112) are **chip-wide**. They hit the BPI-R3 Mini exactly as hard as the Flint 2. Mitigate by running 802.11s at HE80 on a fixed non-DFS 5 GHz channel (your existing plan).

## 2. The honest answer is GL.iNet Flint 2

The **GL-MT6000 (Flint 2)** is the practical BPI-R3 replacement: **identical MT7986AV + MT7976 silicon**, the most-documented mt7986 OpenWrt target (stable since 23.05.3, current 25.12.x — *no snapshot roulette*), confirmed working 802.11s, 1 GB RAM, dual 2.5GbE, 4 external antennas, and **always in stock** at ~$140–170. It's a finished router (case + PSU), so it's also *less* work than any bare board. Only compromise vs the bare board: the 4 antennas are external but **not removable**, and MSRP ($170) straddles your $150 line — though it routinely sells ~$140–160.

If you want to stay strictly under $150 / go cheaper, drop to the mt7981 tier (below) — same mesh stack, 2×2 instead of 4×4.

## 3. Cheapest fleet nodes (mt7981, same mesh family)

- **Cudy WR3000 (~$35)** — the default cheap node. Official OpenWrt, **flash from the stock web UI** (easiest in class), reliably stocked on Amazon. ⚠️ Check serial: units S/N ≥ 2543 are a "new flash" hardware revision needing a different install procedure. Antennas fixed.
- **CMCC RAX3000M (~$25–40, AliExpress)** — cheapest with external (non-removable) antennas; Chinese-ISP surplus, plentiful. Confirm NAND vs eMMC/DDR3 variant before flashing.
- **Cudy TR3000 (~$50)** — compact, easy web-UI flash, internal antennas.
- **Avoid for a fleet:** Xiaomi AX3000T — cheapest sticker (~$40) but exploit-flash + a Winbond-vs-ESMT **flash-chip brick lottery**; and TP-Link AX55 Pro/EX520 — not yet mainline-merged.

## 4. If you need REMOVABLE antennas (for high-gain upgrades)

This is the real gap: **cheap mt7981/mt7986 consumer routers almost all have fixed antennas.** Removable RP-SMA in this price class comes from:

- **GL.iNet GL-A1300 (Slate Plus), ~$65** — IPQ4018/**ath10k** (mature 802.11s), WiFi5, **2× detachable RP-SMA**, always in stock. Best value for swappable antennas.
- **GL.iNet GL-AR750S-Ext (Slate), ~$60** — QCA9563 ath9k + QCA9886 ath10k, **2× detachable**, OpenWrt preinstalled; ath9k 2.4 GHz is gold-standard mesh. (Buy via Amazon — GL's US store was closing ~June 2026.)
- **Netgear R7800 (used, ~$50–90)** — QCA9984 ath10k, **4× removable RP-SMA**, top-tier OpenWrt target, best throughput-per-dollar of the cheap lot.
- **Zbtlink Z810x (mt7981, ~$60–120)** — industrial, 7–8 external (some removable) antennas, if you also want cellular.

Tradeoff: the removable-antenna options are mostly **ath9k/ath10k (WiFi 4/5)** — slower, and ath10k needs the **non-CT firmware swap** for mesh. But ath9k/ath10k is the *more* battle-tested mesh stack, so for a backhaul-grade link that's a fair trade.

## 5. GL.iNet gotcha worth knowing before you buy

GL.iNet's **GUI "Mesh" feature is proprietary — NOT 802.11s.** But every GL.iNet unit ships full **LuCI** (Advanced settings), and **LuCI does expose 802.11s mesh-point mode**. So on GL.iNet hardware you either: (a) keep stock firmware, SSH in once to install `wpad-mesh-openssl`, and configure 802.11s **only via LuCI** (never touch the GL GUI Wireless page afterward), or (b) flash vanilla OpenWrt (clean — recommended where support is mature, e.g. Flint 2 is in stable). You do **not** have to flash vanilla to get 802.11s, but it's tidier where supported.

---

## 6. Recommendation

1. **Default buy — GL.iNet Flint 2 (GL-MT6000), ~$140–160.** This is your BPI-R3 Mini, in stock, same silicon, more mature, with a case. Get one or two, validate the 802.11s + babeld stack, then scale.
2. **Cheapest scale-out fleet — Cudy WR3000 (~$35) on mt7981.** Same mesh family, trivial flashing, Amazon stock. Accept fixed antennas.
3. **Where you need high-gain/removable antennas — GL.iNet GL-A1300 (~$65, ath10k, removable RP-SMA)** or a used **Netgear R7800 (~$70, 4× removable)**.
4. **Mixed fleet is fine** — 802.11s interoperates across mt76/ath9k/ath10k as long as channel, mesh ID, band, and SAE encryption match, and all nodes run `wpad-mesh-*`.

**Net:** don't pay the BPI-R3 Mini scarcity tax. The Flint 2 is the same chip done better-supported and in-stock; the Cudy WR3000 is the same mesh family for $35; and GL.iNet's QCA models cover the removable-antenna need cheaply.

---

## 7. Node-role → recommended model

Mesh nodes aren't interchangeable — match the hardware to the job. A typical fleet mixes these roles, and 802.11s interoperates across mt76/ath9k/ath10k (same channel, mesh ID, band, SAE; all on `wpad-mesh-*`), so a mixed fleet is expected and fine.

| Role | What it needs | Recommended | Why / alternates |
|---|---|---|---|
| **Gateway / hub** (wired uplink, serves many clients, central) | Fast wired ports, RAM, high client throughput | **GL.iNet Flint 2 (GL-MT6000)** ~$150 | Dual 2.5 GbE uplink, 1 GB RAM, WiFi 6 4×4, mt7986 (on-standard), stable OpenWrt. *Cheaper alt:* **Cudy TR3000** ~$50 (2.5 GbE + USB3, but internal antennas — fine for a hub that doesn't need high-gain radio). |
| **Backhaul / relay** (mesh reach matters; wants directional high-gain antenna) | **Removable** antennas, rock-solid mesh driver, fixed non-DFS 5 GHz | **GL.iNet Slate Plus (GL-A1300)** ~$65 | 2× detachable RP-SMA + battle-tested ath10k mesh. WiFi 5 is fine here (backhaul gains little from WiFi 6). *More throughput:* used **Netgear R7800** ~$70 (4× removable RP-SMA, QCA9984). *Bare-board tri-radio:* **BPI-R3 + AW7915-NP1** (dedicated 5 GHz backhaul) — see `synthesis-pass2.md`. Budget the ath10k non-CT firmware swap. |
| **Leaf / client AP** (coverage node, cheap, fixed antennas OK) | Low cost, easy flash, WiFi 6 for clients | **Cudy WR3000** ~$35 | Official OpenWrt, web-UI flash, 4 external (fixed) whips, mt7981 (same mesh family). *AliExpress alt:* **CMCC RAX3000M** ~$25–40. |
| **Compact / deploy-anywhere leaf** (tight enclosure, low power) | Small footprint, USB-C power | **Cudy TR3000** ~$50 (pocket, internal antennas) | Or **BPI-R3 Mini** if you can get it at a sane price (currently can't). |
| **Validation / bench** (prove the 802.11s + babeld stack first) | Easy flash, low cost, representative silicon | **GL.iNet Flint 2** (on-standard mt7986) or **Cudy WR3000** | Buy 2, stand up an 802.11s link, validate before fleet commit. |

**Antenna rule of thumb:** only the **backhaul/relay** role truly needs removable high-gain antennas → that's the ath10k GL.iNet/R7800 lane (or a bare-board build). Gateway and leaf roles are fine on fixed/internal antennas, which is exactly where the cheap mt76 routers (Flint 2 / WR3000 / TR3000) live.

---

## 8. Verification

- All four branches written with primary sources (OpenWrt ToH/firmware-selector, vendor stores, retailer listings, forum threads). ✅
- Stock claims are "as listed June 2026" — live inventory can't be guaranteed; flagged honestly per item (esp. used-market ath9k/ath10k and BPI-R3 Mini scarcity). ⚠️
- Cross-check vs pass 1/2 standard (mt76 AP family + ath9k/ath10k): all picks are on-standard. ✅
- Antenna-removability gap surfaced explicitly rather than glossed (most cheap mt798x = fixed antennas). ✅
- Status: **PASS_WITH_WARNINGS** (warnings = live stock/price volatility; mt76 mesh bug-tail; ath10k non-CT firmware step; Xiaomi flash-chip lottery).
