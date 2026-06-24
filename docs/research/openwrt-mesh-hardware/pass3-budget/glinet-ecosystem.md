# GL.iNet Ecosystem as an OpenWrt 802.11s Mesh Source (Pass 3 — Budget)

_Research date: June 2026. Prices in USD, US market (gl-inet.com US store + Amazon.com)._

## TL;DR

GL.iNet is a genuinely practical answer to the Banana Pi stock problem. The whole
line ships **OpenWrt-based firmware out of the box**, sells **direct AND on Amazon
with good stock**, and every relevant SoC (MediaTek mt76 + Qualcomm ath9k/ath10k/
ath11k) is **802.11s-capable**. Several models are well under $150 with external
antennas.

**The one big caveat (answers the KEY question):** GL.iNet's *own* GUI mesh feature
is **NOT 802.11s** — it is GL.iNet's proprietary mesh (their marketing-name mesh,
built on their own sync/roaming stack). The GL.iNet stock GUI does **not** expose an
802.11s "mesh point" radio mode on its simplified Wireless page. **However**, stock
GL.iNet firmware keeps the full **LuCI** interface (Advanced Settings), and LuCI *does*
expose 802.11s mesh-point mode. So you have two viable paths on GL.iNet hardware:

1. **Keep stock GL.iNet firmware, configure 802.11s via LuCI** (community-recommended
   workflow; requires SSH'ing in once to swap `wpad` for a mesh-capable build, e.g.
   `wpad-openssl`/`wpad-mesh-openssl`). Do NOT touch the GL.iNet Wireless GUI page after
   that — drive Wi-Fi only from LuCI.
2. **Flash vanilla OpenWrt** (clean, standard `wpad-mesh-*`, no proprietary cruft).
   Best for models with mature mainline support (GL-MT6000 is in **stable** OpenWrt).

Bottom line: you do **not** strictly need to flash vanilla OpenWrt to get 802.11s on
GL.iNet — LuCI on stock firmware works — but vanilla is cleaner where support is mature.

---

## Model table

| Model (name) | SoC / WLAN driver | Wi-Fi | Ext. antennas | Price (USD, Jun 2026) | Stock (gl-inet / Amazon) | 802.11s: stock fw (LuCI) vs vanilla |
|---|---|---|---|---|---|---|
| **GL-MT6000** (Flint 2) | MediaTek MT7986A + MT7976 / **mt76** | WiFi6 AX6000 | 4x, **non-detachable** | ~$130–160 (sale lows ~$113) | In stock both; strong Amazon stock | **Stable vanilla OpenWrt** (since 23.05.3, current 25.12.x). 802.11s via LuCI on stock OR vanilla. **Best supported.** |
| **GL-MT3000** (Beryl AX) | MediaTek MT7981B (Filogic 820) / **mt76** | WiFi6 AX3000 | **Internal only** (pocket router) | ~$70–90 | In stock both; excellent stock | 802.11s via LuCI on stock; vanilla in stable/snapshot. No ext antennas. |
| **GL-B3000** (Marble) | MediaTek MT7981B (Filogic 820) / **mt76** | WiFi6 AX3000 | **Internal only** (photo-frame design) | ~$70–90 | In stock both | 802.11s via LuCI on stock; vanilla in snapshot/24.10. No ext antennas. |
| **GL-AXT1800** (Slate AX) | Qualcomm **IPQ6000** / **ath11k** | WiFi6 AX1800 | 2x, **detachable/removable** | ~$100–110 | In stock both | 802.11s via LuCI on stock (this is the model the GL.iNet forum 802.11s guide targets). ath11k 802.11s historically finicky — verify. Ext antennas = plus. |
| **GL-A1300** (Slate Plus) | Qualcomm **IPQ4018** / **ath10k** | WiFi5 AC1300 | 2x, **detachable/removable** | ~$60–70 | In stock both | 802.11s via LuCI on stock; vanilla in snapshot. ath10k 802.11s is mature. **Cheap + ext antennas.** |
| **GL-AR750S-Ext** (Slate) | Qualcomm **QCA9563** (2.4G, ath9k) + **QCA9887** (5G, ath10k) | WiFi5 AC750 | 2x, **detachable**, fold | ~$60–70 | In stock both | 802.11s via LuCI on stock; vanilla mature (ath79). ath9k+ath10k both 802.11s-solid. **Legacy but rock-solid mesh.** |
| **GL-AR300M(16)-Ext** (Shadow) | Qualcomm **QCA9531** / **ath9k** | 2.4GHz only N300 | 2x, **detachable** | ~$30–40 | In stock both | 802.11s via LuCI/vanilla; ath9k 802.11s very mature. 2.4GHz-only, low throughput. Cheapest node. |
| **GL-MT2500** (Brume 2) | MediaTek MT7981 (no radio — wired gateway) | none (no Wi-Fi) | n/a | ~$70 | In stock | **Not a mesh radio node** — wired-only security gateway. Skip for 802.11s. |
| **GL-BE3600** (Slate 7) | Qualcomm **IPQ5312** / ath12k | WiFi7 | internal (travel) | ~$100–130 | In stock both | WiFi7; OpenWrt mainline WIP. ath12k 802.11s immature in mid-2026 — not recommended yet. |
| **GL-BE9300** (Flint 3) | Qualcomm quad-core / ath12k | WiFi7 tri-band | 4x | ~$180–230 | In stock | Over budget; WiFi7 ath12k 802.11s immature. Skip for now. |
| **GL-MT3600BE** (Beryl 7) | MediaTek **MT7987A + MT7990** / mt76 | WiFi7 | internal (travel) | ~$100+ | In stock both | mt76 WiFi7; mainline support WIP (PR #22476). Promising later, not yet mature for 802.11s. |

Notes:
- "802.11s via LuCI on stock" = keep GL.iNet firmware, use Advanced > LuCI > Network >
  Wireless, set radio mode to **802.11s / Mesh Point**, after `wpad` swap. GL.iNet's
  own GUI "Mesh" is a different, proprietary feature — ignore it for 802.11s.
- Antenna "detachable/removable" generally means RP-SMA on the QCA travel routers
  (AR750S, AR300M, A1300, AXT1800). The MediaTek home/desktop units (MT6000) have
  large but **fixed** antennas; the pocket units (MT3000, B3000) have **internal** only.
- Prices fluctuate; GL-MT6000 in particular sees frequent Amazon sales (seen as low
  as ~$113 Nov 2025; coupon to ~$134; list ~$159–189 depending on store/region).

---

## Answering the KEY question

> Does GL.iNet stock OpenWrt firmware expose 802.11s mesh-point mode in LuCI, or must
> the user flash vanilla OpenWrt?

- **GL.iNet's simplified GUI does NOT expose 802.11s.** Its "Mesh" feature is GL.iNet's
  **proprietary** mesh (their own roaming/sync tech), not the IEEE 802.11s mesh-point
  mode you want for an OpenWrt 802.11s + (optionally) batman-adv/babel backhaul.
- **But stock GL.iNet firmware ships full LuCI** (under Advanced Settings), and **LuCI
  DOES expose 802.11s mesh-point mode.** The GL.iNet official forum's own guide
  ("[Guide] Slate AX / Flint — 802.11k/v/r/s & Mesh Support") walks through exactly this:
  - SSH in, swap the hostapd/wpad build for a mesh-capable one:
    `opkg update && opkg remove wpad-openssl && opkg install wpad-openssl --force-overwrite`
    (later posts in the thread suggest `wpad-mesh-openssl` is preferable).
  - Configure the mesh interface in **LuCI**, mode **802.11s**.
  - **Rule from the guide: never touch Wi-Fi from the GL.iNet Wireless GUI page after
    this — only use LuCI**, or the GUI will clobber your mesh config.
- **So: you can run 802.11s on stock GL.iNet firmware without flashing vanilla.** Flash
  vanilla only if you want a clean standard image (recommended where mainline support
  is mature, i.e. GL-MT6000 stable; MT3000/B3000/A1300 snapshot; AR750S/AR300M ath79).

---

## Best GL.iNet picks for our mesh

Ranked for an OpenWrt **802.11s** mesh, weighting: in-stock, ≤$150, external antennas,
mature 802.11s driver, good throughput.

1. **GL-MT6000 (Flint 2) — best overall node.** MT7986A/mt76, **stable mainline
   OpenWrt** (the killer feature: no snapshot roulette), AX6000, 2x 2.5GbE, huge stock,
   ~$113–160. Caveat: 4 antennas are **fixed**, not removable. If you want a strong,
   well-supported, high-throughput mesh node and don't need swappable antennas, this is
   the pick. Flash vanilla OpenWrt (it's in the stable ToH) or run 802.11s via LuCI on
   stock.

2. **GL-A1300 (Slate Plus) — best value node WITH removable external antennas.**
   IPQ4018/**ath10k** (mature, reliable 802.11s), WiFi5 AC1300, **2x detachable
   RP-SMA antennas**, ~$60–70, always in stock. This is the sweet spot if external/
   upgradeable antennas matter: cheap, antennas you can swap, and ath10k 802.11s is
   battle-tested. Lower throughput than the MT6000 but plenty for backhaul-grade mesh.

3. **GL-AR750S-Ext (Slate) — rock-solid legacy mesh node, cheap, ext antennas.**
   QCA9563 (ath9k 2.4G) + QCA9887 (ath10k 5G), **2x detachable** folding antennas,
   ~$60–70. Both drivers have the most mature 802.11s support in OpenWrt. AC750 is slow
   for a backhaul but extremely dependable; great for edge/leaf nodes or testing.

4. **GL-MT3000 (Beryl AX) / GL-B3000 (Marble) — best cheap WiFi6 nodes, but NO external
   antennas.** MT7981B/mt76, AX3000, ~$70–90, excellent stock. Pick these if you want
   modern WiFi6 throughput on a budget and don't need external antennas (MT3000 is a
   pocket router, B3000 a photo-frame desktop). 802.11s via LuCI/vanilla snapshot.

5. **GL-AXT1800 (Slate AX) — WiFi6 + removable antennas, but verify ath11k 802.11s.**
   IPQ6000/**ath11k**, **2x detachable** antennas, ~$100–110. Attractive combo (WiFi6 +
   ext antennas + the model the GL.iNet 802.11s forum guide targets), but ath11k 802.11s
   has historically been less mature than ath10k/ath9k — **test mesh-point mode before
   committing to a fleet.**

**Avoid for now:** GL-BE3600 / GL-BE9300 / GL-MT3600BE (WiFi7, ath12k/mt76 WiFi7 —
802.11s support immature in mid-2026, and BE9300 is over budget). GL-MT2500 (Brume 2)
has **no Wi-Fi radio** — wired gateway only, not a mesh node.

### Practical recommendation
- For **backbone/high-throughput nodes**: GL-MT6000 (vanilla OpenWrt, stable) — accept
  fixed antennas.
- For **external-antenna nodes / range flexibility / lowest cost**: GL-A1300 or
  GL-AR750S-Ext (ath10k/ath9k, removable antennas, mature 802.11s).
- A **mixed fleet works** — 802.11s interoperates across mt76/ath9k/ath10k as long as
  channel, mesh ID, encryption (SAE) and band match. Keep wpad = `wpad-mesh-*` on all.

---

## Sources

- [GL.iNet Firmware Versions](https://www.gl-inet.com/support/firmware-versions/)
- [GL.iNet forum: Guide — Slate AX / Flint 802.11k/v/r/s & Mesh Support](https://forum.gl-inet.com/t/guide-slate-ax-flint-802-11k-v-r-s-mesh-support/27821)
- [GL.iNet forum: Differences between firmware, GUI & OpenWrt](https://forum.gl-inet.com/t/differences-between-firmware-gui-openwrt/43531)
- [tekovic.com: OpenWRT and the 802.11s mesh network](https://www.tekovic.com/blog/openwrt-80211s-mesh-networking/)
- [Flint 2 (GL-MT6000) product page](https://store-us.gl-inet.com/products/flint-2-gl-mt6000-wi-fi-6-high-performance-home-router)
- [GL-MT6000 on Amazon](https://www.amazon.com/GL-iNet-GL-MT6000-Multi-Gig-Connectivity-WireGuard/dp/B0CP7S3117)
- [GL-MT6000 OpenWrt Firmware Selector (mediatek/filogic)](https://firmware-selector.openwrt.org/?target=mediatek%2Ffilogic&id=glinet_gl-mt6000)
- [TechInfoDepot: GL-MT6000 (Flint 2)](https://techinfodepot.shoutwiki.com/wiki/GL.iNet_GL-MT6000_(Flint_2))
- [OpenWrt forum: GL-MT6000 discussions](https://forum.openwrt.org/t/gl-inet-flint-2-gl-mt6000-discussions/173524)
- [Beryl AX (GL-MT3000) product page](https://www.gl-inet.com/en-us/products/gl-mt3000)
- [GL-MT3000 OpenWrt docs](https://docs.gl-inet.com/router/en/4/user_guide/gl-mt3000/)
- [Slate AX (GL-AXT1800) product page](https://store-us.gl-inet.com/products/slate-ax-gl-axt1800-gigabit-wireless-router)
- [Slate Plus (GL-A1300) product page](https://www.gl-inet.com/en-us/products/gl-a1300)
- [GL.iNet forum: A1300 has official OpenWrt support in snapshot](https://forum.gl-inet.com/t/a1300-has-official-openwrt-support-in-snapshot/25705)
- [OpenWrt Wiki: GL.iNet GL-AR750S](https://openwrt.org/toh/gl.inet/gl-ar750s)
- [Shadow (GL-AR300M16-Ext) product page](https://store-us.gl-inet.com/products/gl-ar300m16-mini-smart-router)
- [GL-B3000 (Marble) on Amazon](https://www.amazon.com/GL-iNet-GL-B3000-Wall-Mountable-Dual-Band-WireGuard/dp/B0D7PTFZZM)
- [OpenWrt forum: GL-B3000 (Marble) support](https://forum.openwrt.org/t/gl-b3000-marble-support-coming/238258)
- [Slate 7 (GL-BE3600) product page](https://www.gl-inet.com/en-us/products/gl-be3600)
- [Flint 3 (GL-BE9300) product page](https://www.gl-inet.com/en-us/products/gl-be9300)
- [Beryl 7 (GL-MT3600BE) product page](https://www.gl-inet.com/en-us/products/gl-mt3600be)
- [OpenWrt Table of Hardware](https://openwrt.github.io/toh-openwrt-org/)
