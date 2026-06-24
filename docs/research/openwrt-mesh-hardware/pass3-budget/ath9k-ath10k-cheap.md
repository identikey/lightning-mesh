# Cheap ath9k / ath10k Routers for OpenWrt 802.11s Mesh — Budget Fallback

**Research pass:** Pass 3 — Budget fallback to MediaTek mt76 boards
**Date:** June 2026
**Goal:** Cheap (<$150, ideally <$80), in-stock NEW or used routers/APs using Qualcomm Atheros **ath9k** (WiFi 4) or **ath10k** (WiFi 5) — the most battle-tested OpenWrt 802.11s mesh drivers. Dual-radio (2.4 + 5 GHz) preferred; external antennas a big plus.

---

## TL;DR

- **ath9k (WiFi 4) is the gold-standard 802.11s mesh driver** — fully mainline, no firmware blob caveats, encryption works. The tradeoff is throughput: WiFi 4 (802.11n), typically capped well under 100 Mbps real-world per hop, and ath9k mesh runs slow at HT40.
- **ath10k (WiFi 5) works for 802.11s but with caveats:** the default OpenWrt **CT (Candela Technologies) firmware historically does NOT do 802.11s well** (broken on wave-1 / QCA988x; the ath10k-ct driver fails the common mesh-VIF + AP-VIF combo). Use the **non-CT (upstream/stock) ath10k firmware** for mesh, and note mesh is generally limited to **VHT20** with **no encryption** on many ath10k parts. So you trade the simplicity/robustness of ath9k for more bandwidth, but with config fiddling.
- **Best cheap dual-radio pick for the value:** **TP-Link Archer C7 v5 / A7 v5** (ath9k 2.4 GHz + ath10k 5 GHz, 3 external antennas, ~$30-60 used) — but be aware the 5 GHz mesh needs non-CT firmware and is throughput-limited.
- **Best "just works" mesh pick:** **GL.iNet GL-AR750S-Ext (Slate)** — dual-band, **2 external antennas**, OpenWrt preinstalled, cheap, and the 2.4 GHz radio is rock-solid ath9k. Caveat: GL.iNet's US store was closing June 2026; buy via Amazon/Walhmart instead.
- **Best throughput-for-dollar if you can tolerate ath10k mesh fiddling:** **Netgear R7800** (QCA9984, **4 external antennas**, dual-radio, top-tier OpenWrt target) — ~$50-90 used.

---

## Ranked comparison table

Ranked for the user's priorities: cheap, dual-radio, external antennas, solid 802.11s.

| Rank | Model | Chipset / Driver | Radios | Ext. antennas | Price (new / used) | Where to buy | 802.11s mesh notes |
|------|-------|------------------|--------|---------------|--------------------|--------------|--------------------|
| 1 | **GL.iNet GL-AR750S-Ext (Slate)** | QCA9563 (2.4 ath9k) + QCA9886 (5 ath10k) | Dual (2.4+5) | **Yes — 2 ext.** | ~$50-70 new; ~$16-30 refurb | Amazon, Walmart, GL.iNet regional stores | OpenWrt preinstalled. 2.4 GHz ath9k mesh is rock-solid. 5 GHz is ath10k (non-CT for mesh). Smallest/cheapest "just works" option. **US store closing Jun 2026 → use Amazon.** |
| 2 | **TP-Link Archer C7 v5 / A7 v5** | QCA9563 (2.4 ath9k) + QCA9880/QCA9882 (5 ath10k) | Dual (2.4+5) | **Yes — 3 ext.** (5 GHz integrated on v4/v5) | EOL new; ~$30-60 used | eBay, Amazon used, FB Marketplace | Best cheap dual-radio. 2.4 ath9k mesh excellent. 5 GHz ath10k needs **non-CT firmware**; ath10k-ct historically breaks mesh on this board. Huge OpenWrt community. |
| 3 | **Netgear R7800 (Nighthawk X4S)** | IPQ8065 + QCA9984 (ath10k, both radios) | Dual (2.4+5) | **Yes — 4 ext.** | ~$50-90 used | eBay (many listings) | Top-tier OpenWrt target, fast (1.7 GHz dual-core, AC2600). **Both radios ath10k** → mesh needs non-CT firmware; CT fw has documented mesh/compat issues. Best throughput-per-dollar. |
| 4 | **Linksys EA8300 / MR8300** | IPQ4019 (2.4 + 5 ath10k) + QCA9888 (2nd 5 GHz, ath10k) | **Tri-radio** | No (internal) | ~$30-60 used | eBay, Amazon used | Tri-band: dedicate the 3rd (QCA9888) radio to mesh backhaul, keep others for clients. All ath10k → non-CT/firmware care. MR8300 mesh confirmed working but some report "crippled"/flaky configs. No ext. antennas. |
| 5 | **TP-Link Archer C2600 / C5400** | IPQ8064 + QCA9980 (ath10k, both radios) | Dual (2.4+5) | **Yes — 4 ext.** (C2600) | ~$25-50 used | eBay (many), Amazon used | Cheap, fast AC2600, 4 ext. antennas. Both radios ath10k → non-CT firmware for mesh. Good OpenWrt support (ipq806x). Strong value if found cheap. |
| 6 | **Netgear R7500v2 (Nighthawk X4)** | IPQ8064 + QCA9980 (5) + QCA9880 (2.4) | Dual (2.4+5) | **Yes — 4 ext.** | ~$30-60 used | eBay, Amazon used | ipq806x, all ath10k. Historically had wifi stability bugs on OpenWrt (FS#1197); less polished than R7800. Non-CT fw for mesh. Buy R7800 instead if you can. |
| 7 | **GL.iNet GL-AR300M16-Ext (Shadow)** | QCA9531 (ath9k) | **Single (2.4 only)** | **Yes — 2 ext.** | ~$30-40 new; ~$13-25 refurb | Amazon, GL.iNet stores | Pure ath9k, gold-standard mesh stability, but **2.4 GHz only** (single radio) — no dedicated backhaul band. Great cheap mesh node if 2.4-only is acceptable. Tiny, low-power. |
| 8 | **Ubiquiti UniFi AP AC (Lite / LR / Pro / Mesh)** | QCA9563 (2.4 ath9k) + QCA988x (5 ath10k) | Dual (2.4+5) | Internal (AC-Mesh has ext.) | ~$20-50 used | eBay (abundant) | Cheap and plentiful used. OpenWrt support exists but **install can be painful** on newer factory firmware; PoE-only powering. ath10k 5 GHz mesh caveats apply. AC-Mesh/AC-Mesh-Pro have external antennas. |

---

## Driver reality check: ath9k vs ath10k for 802.11s

### ath9k (WiFi 4 / 802.11n) — the gold standard
- Fully **mainline mphline driver, no firmware blob**, mature 802.11s support.
- Encryption (SAE/authsae) works; mesh is stable.
- **Config gotcha:** disable hardware crypto for mesh stability — add `nohwcrypt=1` to `/etc/modules.d/ath9k`. HW crypto + mesh is known-unstable.
- **Throughput tradeoff:** WiFi 4 only. ath9k mesh is notably **slow at HT40**; expect well under 100 Mbps per hop real-world. Fine for control/IoT/low-bandwidth mesh; weak for video/bulk.

### ath10k (WiFi 5 / 802.11ac) — more bandwidth, more caveats
- Two firmware families: **CT (Candela)** — OpenWrt's default — and **upstream non-CT**.
- **CT firmware historically does NOT support 802.11s properly** (broken on wave-1/QCA988x; ath10k-ct fails mesh-VIF + AP-VIF combos that routers commonly need). **For mesh, install the non-CT firmware** (e.g. `ath10k-firmware-qca9984` instead of `…-ct`).
- ath10k mesh is generally limited to **VHT20** and often **no encryption** — so the "WiFi 5 = faster" win is partially clawed back on the mesh link.
- Practical pattern on dual/tri-band boards: run **ath9k 2.4 GHz as the reliable mesh backhaul**, or dedicate a spare ath10k 5 GHz radio (tri-band boards) to backhaul and keep the rest for clients.

### Required OpenWrt packages (either driver)
- Replace `wpad-basic*` with **`wpad-mesh-wolfssl`** (or `wpad-mesh-openssl`) — basic wpad cannot do mesh/SAE.
- For ath10k mesh, swap to the **non-CT** firmware package for your chip.

---

## Best cheap mesh picks (recommendations)

### If you want it to "just work" with external antennas → **GL.iNet GL-AR750S-Ext (Slate)**
Dual-band, **2 external antennas**, OpenWrt preinstalled, cheap (refurb seen $16-30, new ~$50-70). The 2.4 GHz ath9k radio gives you gold-standard mesh out of the box; 5 GHz ath10k available for client AP or (with non-CT fw) a faster backhaul. Lowest friction of any option. **Buy via Amazon/Walmart** — GL.iNet's US store was closing June 2026.

### Best dual-radio value with antennas → **TP-Link Archer C7 v5 / A7 v5**
~$30-60 used, **3 external antennas**, massive OpenWrt community, current releases (25.12.x) install cleanly. ath9k 2.4 GHz mesh is excellent; 5 GHz ath10k needs non-CT firmware. The canonical cheap OpenWrt router. Note: EOL/new stock dwindling — buy used.

### Best throughput-per-dollar → **Netgear R7800**
~$50-90 used, **4 external antennas**, fast IPQ8065 + QCA9984, premier OpenWrt target with a huge dev community. Both radios are ath10k, so mesh requires non-CT firmware and accepting ath10k's VHT20/encryption limits — but you get real AC throughput and the best radios in this list. Abundant on eBay.

### Cheapest dedicated-backhaul-band option → **Linksys EA8300 / MR8300**
Tri-radio for ~$30-60 used: keep two radios for clients, dedicate the third (QCA9888 5 GHz) to mesh backhaul. All ath10k (firmware care needed), no external antennas, and some users report flaky mesh — but the dedicated backhaul band is architecturally nice for a mesh. Good if internal antennas are acceptable.

### Pure-ath9k stability, single band → **GL.iNet GL-AR300M16-Ext (Shadow)**
~$13-40, **2 external antennas**, 100% ath9k = most robust mesh. Downside: **2.4 GHz only** (single radio), so no separate backhaul band and limited throughput. Great as a cheap, reliable, low-power mesh node where bandwidth is not critical.

---

## Honesty notes on availability (June 2026)

- These are **older chips**, so most are **used-market** buys (eBay, Amazon-used, FB Marketplace). New retail stock is thin and many models (Archer C7, C2600) are **End-of-Life**. Prices above are estimates from eBay trending/used ranges; verify the live listing — condition, box, and accessories swing prices a lot.
- **GL.iNet** units are the main reliably-NEW-in-stock option (ath9k-based), but the **US GL.iNet store was closing ~June 16 2026** — purchase via **Amazon or Walmart** for U.S. delivery.
- **R7800** used supply is healthy and it's the strongest all-around pick; **Archer C7/A7** used supply is abundant and cheapest.
- For all ath10k boards, budget time for the **non-CT firmware swap** before mesh will behave.

---

## Sources

- [Netgear R7800 OpenWrt exploration (IPQ8065, QCA9984)](https://forum.openwrt.org/t/netgear-r7800-exploration-ipq8065-qca9984/285)
- [Netgear R7800 listings — eBay](https://www.ebay.com/sch/i.html?_nkw=netgear+r7800&_sop=12)
- [OpenWrt ToH — TP-Link Archer C7 AC1750](https://openwrt.org/toh/tp-link/archer_c7)
- [OpenWrt forum — Archer C7 v3/v5 install (25.12.x confirmed)](https://forum.openwrt.org/t/openwrt-on-tp-link-archer-c7-v3-and-v5/233873)
- [Linksys EA8300 // Tri-Band OpenWrt — OpenWrt forum](https://forum.openwrt.org/t/linksys-ea8300-tri-band-router-openwrt/37353)
- [Linksys MR8300 — OpenWrt forum](https://forum.openwrt.org/t/linksys-mr8300/205006)
- [Linksys MR8300 crippled mesh connection — OpenWrt forum](https://forum.openwrt.org/t/linksys-mr8300-crippled-mesh-connection/169464)
- [GL.iNet GL-AR750S-Ext (Slate) — Amazon](https://www.amazon.com/GL-iNet-GL-AR750S-Ext-pre-Installed-Cloudflare-Included/dp/B07GBXMBQF)
- [GL.iNet GL-AR300M16-Ext (Shadow) — Amazon](https://www.amazon.com/GL-iNet-GL-AR300M16-Ext-Pre-Installed-Performance-Programmable/dp/B07794JRC5)
- [GL.iNet travel routers refurb pricing — Slickdeals](https://slickdeals.net/f/17298205-gl-inet-travel-routers-refurb-gl-ar750s-ext-slate-16-gl-mt1300-beryl-19-90-more)
- [ath10k-ct and 802.11s — not working on Archer C7 — OpenWrt forum](https://forum.openwrt.org/t/ath10k-ct-and-802-11s-mesh-not-working-on-archer-c7/13877)
- [ath10k-ct wifi-driver does not support 802.11s — OpenWrt forum](https://forum.openwrt.org/t/ath10k-ct-wifi-driver-does-not-support-802-11s/125423)
- [No mesh (802.11s) on ath10k-firmware-qca9888? — OpenWrt forum](https://forum.openwrt.org/t/no-mesh-802-11s-on-ath10k-firmware-qca9888/82730)
- [802.11s: ath9k slow at HT40; ath10k VHT20/no-encryption — OpenWrt forum](https://forum.openwrt.org/t/802-11s-ath9k-slow-speeds-on-ht40-ath10k-doesnt-support-mesh-above-vht20-and-no-encryption-support/2978)
- [Setup OpenWRT Mesh Network (ath9k nohwcrypt, wpad-mesh) — PHB Crystal Ball](https://phb-crystal-ball.org/setup-openwrt-mesh-network/)
- [Difference between QCA9980 and QCA9984 in ath10k — OpenWrt forum](https://forum.openwrt.org/t/difference-between-qualcomm-qca9980-and-qca9984-in-ath10k-driver-in-openwrt/93321)
- [Netgear R7500v2 — TechInfoDepot](https://techinfodepot.shoutwiki.com/wiki/Netgear_R7500v2)
- [TP-Link Archer C2600 listings — eBay](https://www.ebay.com/p/2255422679)
- [UniFi AP AC Lite OpenWrt — OpenWrt forum](https://forum.openwrt.org/t/unifi-ap-ac-lite-openwrt-or-stay-with-stock-firmware/72723)
