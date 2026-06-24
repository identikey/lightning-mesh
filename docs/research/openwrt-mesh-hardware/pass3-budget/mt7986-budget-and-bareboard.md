# MT7986 (Filogic 830) Budget & Bare-Board Reality Check

**Research date:** June 2026
**Goal:** Find cheap MT7986 hardware for an 802.11s mesh (kmod-mt7915e + kmod-mt7986-firmware), either (a) a sub-$150 router with a case, or (b) the real story on BPI-R3 Mini bare-board stock/pricing.
**TL;DR verdict:** Stop waiting for a sub-$200 BPI-R3 Mini. It is genuinely scarce and out of stock at nearly every Western reseller. The **GL.iNet Flint 2 (GL-MT6000)** is the right buy: same MT7986AV + MT7976 radios, mature OpenWrt support, confirmed 802.11s, 4 external antennas, ~$170 retail (often $139-160 on sale). If you want the cheapest MT7986 with external antennas, the **Xiaomi Redmi AX6000** (~$90-100, 6 external antennas) is the budget king, with the caveat that 802.11s on MT76 has known bandwidth bugs that affect *all* these chips equally.

---

## Important caveat that applies to ALL MT7986 hardware

802.11s mesh on the MediaTek MT76 driver has documented OpenWrt bugs that are **chip-level, not board-level** — they affect the BPI-R3 Mini exactly as much as the Flint 2 or Redmi AX6000:

- 5 GHz throughput can drop to <1/3 when an 802.11s node is enabled ([openwrt#12905](https://github.com/openwrt/openwrt/issues/12905)).
- 2.4 GHz 802.11s links negotiate only 20 MHz even when 40 MHz is configured ([openwrt#13112](https://github.com/openwrt/openwrt/issues/13112)).

People work around this by running 802.11s at HE80 on 5 GHz and/or layering batman-adv on top. **Picking a more expensive board does not buy you out of these bugs** — so optimize for price, antennas, and OpenWrt maturity, not for the "premium" bare board.

---

## Price / Stock Table (June 2026)

| Device | SoC | Case? | Ext. ant. | OpenWrt ToH | 802.11s | Price (USD) | Where / Stock |
|---|---|---|---|---|---|---|---|
| **GL.iNet Flint 2 (GL-MT6000)** | MT7986AV + MT7976 | Yes | 4 (non-detach) | [supported, 23.05.3+](https://forum.openwrt.org/t/gl-inet-flint-2-gl-mt6000-discussions/173524) | Yes (confirmed working) | **$169.99 MSRP** (~$139-160 on sale) | GL.iNet store / Amazon — **in stock** |
| **Xiaomi Redmi AX6000** | MT7986AV + MT7976 | Yes | 6 (non-detach) | [supported 23.05.0+ / ImmortalWrt](https://forum.openwrt.org/t/add-openwrt-support-for-xiaomi-redmi-ax6000/125008) | Yes (with MT76 caveats) | **~$90-100** | AliExpress — in stock (CN import, needs U-Boot mod flash) |
| **TP-Link EX820v** | MT7986 | Yes | internal | [PR #13900, in dev](https://github.com/openwrt/openwrt/pull/13900) | via OpenWrt once merged | ISP-supplied (not retail) | Not a clean retail buy; ISP/used only |
| **TP-Link VX830v** | MT7986 | Yes | internal | [in dev, no clean dump yet](https://forum.openwrt.org/t/support-for-tp-link-vx830v-mt7986/246657) | Not yet | ISP-supplied (IT) | Avoid — no stable OpenWrt |
| **Banana Pi BPI-R3 Mini** | MT7986A + MT7976C | Bare (case extra) | Yes (u.FL/SMA) | [supported](https://docs.banana-pi.org/en/BPI-R3_Mini/BananaPi_BPI-R3_Mini) | Yes | List ~$82-110, **street ~$180-200+** | **Out of stock** at ameriDroid, eBay, TME; scarce/marked up where listed |
| **Banana Pi BPI-R3 (full)** | MT7986A + MT7976 | Bare (case extra) | Yes (SMA) | [supported](https://docs.banana-pi.org/en/BPI-R3/BananaPi_BPI-R3) | Yes | List ~$110 (board), bundles higher | **Out of stock** at ameriDroid ($109.95); limited at youyeetoo/52Pi |
| **Zyxel (MT7986)** | — | — | — | Only APs (NWA series) get OpenWrt; no MT7986 router | n/a | n/a | No suitable retail SKU |
| **Acelink (MT7986)** | — | — | — | Not in ToH | n/a | n/a | Not found / not OpenWrt-supported |

> **Note on quoted prices:** Some reseller pages (youyeetoo) rendered prices in local currency (HKD ≈ "$1,647"/"$1,796") that are *not* real USD — those are conversion artifacts, ignore them. The reliable USD anchors are ameriDroid ($81.95 Mini-bundle / $109.95 full board, both currently **sold out**), GL.iNet ($169.99), and the AX6000 import price (~$90-100).

---

## Job 1 — Cheap MT7986 routers under $150 (case OK)

### GL.iNet GL-MT6000 (Flint 2) — RECOMMENDED
- **SoC:** MT7986AV + MT7976GN/AN, 4x4:4 on both bands. Same silicon family as the BPI-R3.
- **OpenWrt:** Supported since 23.05.3, current through 25.12.x. Both GL's vendor OpenWrt and upstream OpenWrt run. This is the most-documented MT7986 consumer router for OpenWrt.
- **802.11s:** Confirmed working by multiple forum users (HE80 @ 5 GHz mesh between MT6000 units; pairs well with batman-adv). Subject to the MT76 caveats above.
- **External antennas:** Yes — 4 antennas (4x 2.4 GHz + 4x 5 GHz wires). Non-detachable but external/positionable, plus internal u.FL.
- **Ports:** 2x 2.5 GbE + 4x GbE. 1 GB RAM. Strong for a mesh node / gateway.
- **Price:** $169.99 MSRP at GL.iNet; frequently discounted to ~$139-160 on Amazon/sales. **In stock.**
- **Verdict:** Slightly over the $150 line at MSRP but routinely dips under on sale. Best balance of price, OpenWrt maturity, 802.11s reality, and external antennas.

### Xiaomi Redmi AX6000 — CHEAPEST with most antennas
- **SoC:** MT7986AV + MT7976, 4x4:4 both bands, but only **512 MB RAM** (vs 1 GB on Flint 2).
- **OpenWrt:** Supported from 23.05.0; very active ImmortalWrt/hanwckf builds. Requires a U-Boot/exploit flash procedure (more fiddly than the Flint 2's one-click).
- **802.11s:** Works, but this is exactly the device the [openwrt#12905](https://github.com/openwrt/openwrt/issues/12905) / [#13112](https://github.com/openwrt/openwrt/issues/13112) mesh-bandwidth bugs were filed against.
- **Antennas:** **6 external** (non-detachable). 160 MHz capable.
- **Price:** ~$90-100 on AliExpress (CN import; e-catalog lists ~$95.99). **In stock.**
- **ToH/firmware:** [ImmortalWrt selector](https://firmware-selector.immortalwrt.org/?version=23.05.4&target=mediatek/filogic&id=xiaomi_redmi-router-ax6000).
- **Verdict:** Best raw $/MT7986 with external antennas. Trade-offs: 512 MB RAM, harder flash, and it's the poster child for the MT76 mesh bugs. Good if budget is the hard constraint.

### TP-Link EX820v / VX830v (MT7986)
- ISP-supplied AX6000-class boxes (EX820v: MT7986, 512 MB RAM, 2x 2.5 GbE + 3x GbE, USB 3.0). **Not retail products** — you get them from an ISP or used.
- **EX820v** OpenWrt support is in an open PR ([#13900](https://github.com/openwrt/openwrt/pull/13900)) — not yet a stable release target.
- **VX830v** is still stuck in dev (no clean NAND dump / DTS as of early 2026, [forum](https://forum.openwrt.org/t/support-for-tp-link-vx830v-mt7986/246657)).
- **Verdict:** Skip unless you already have one. No clean buy path, immature OpenWrt.

### Zyxel / Acelink
- Zyxel's OpenWrt program only covers **access points (NWA series)**, not an MT7986 router. No suitable SKU.
- "Acelink MT7986" does not appear in the OpenWrt ToH or as an OpenWrt-supported retail device. Dead end.

### AliExpress OEM bare boards (SXK80 / Sieem / CMCC type)
- Searches for non-Banana MT7986 "SXK80"/Sieem OEM bare boards returned **no credible OpenWrt-supported listings** — the MT7986 bare-board market is essentially Banana Pi (BPI-R3 / R3 Mini) plus the BPI-R4 (MT7988, WiFi 7). There is no cheaper drop-in MT7986 clone with real OpenWrt support.
- **The relevant cheaper OEM path is one chip down:** the **CMCC RAX3000M** (MT7981B, the dual-stream Filogic 820 sibling). It's a fully OpenWrt-supported ([firmware selector](https://firmware-selector.openwrt.org/?target=mediatek%2Ffilogic&id=cmcc_rax3000m)) WiFi 6 board widely sold cheap on AliExpress (~$30-50, eMMC versions higher). It uses the *same MT7976 radio family* and same `kmod-mt7915e`/`kmod-mt7986-firmware` mesh stack, so 802.11s behaves the same. It's 2x2 instead of 4x4 (lower throughput, fewer spatial streams) but is the genuine budget MT798x mesh node.

---

## Job 2 — Bare-board reality check

**Is the BPI-R3 Mini genuinely scarce/overpriced? Yes.**

Live stock check across Western resellers (June 2026):

| Reseller | BPI-R3 Mini | Full BPI-R3 |
|---|---|---|
| ameriDroid | **Out of stock** (Mini+case+ant+PSU listed $81.95) | **Out of stock** ($109.95 board) |
| eBay | **Out of stock** | — |
| TME (Sinovoip BPI-R3-MINI) | **Withdrawn from offer** | — |
| youyeetoo | **Sold out** | "In stock, 50 left" (full board, local-currency price) |
| 52Pi | listed | listed (case/kit) |
| Amazon (multiple 3P listings) | listed, no reliable sub-$200 price; historically marked up | listed |

- **Confirmed:** The Mini is out of stock or withdrawn at the most reputable Western channels (ameriDroid, eBay, TME). Where it *is* listed (Amazon 3P, AliExpress resellers), the street price runs **~$180-200+** — matching the user's experience of not finding it under ~$200. Its nominal/list price (~$82 bundle, ~$60-70 bare in China) is far below what you can actually buy it for in the West right now. This is a classic scarcity markup, not a real $80 board you can get.
- **Is the full BPI-R3 cheaper/more available? Marginally better, but also constrained.** The full BPI-R3 board lists around $109-110 (ameriDroid) and youyeetoo shows the full board in stock while the Mini is sold out. So the *full* BPI-R3 is somewhat easier to source than the Mini — but it's a larger board (5x GbE + 2x SFP, needs a case + antennas + PSU), and ameriDroid still shows it out of stock. It is not a clearly cheaper or reliably-stocked win.
- **Cheaper MT7986 bare-board clones on AliExpress:** none with real OpenWrt support. The Banana Pi line is effectively the only OpenWrt-supported MT7986 bare board. Going cheaper means dropping to **MT7981 (CMCC RAX3000M)** boards, which are plentiful and inexpensive on AliExpress.

---

## Recommendation

1. **Don't wait for a sub-$200 BPI-R3 Mini.** It's genuinely supply-constrained — out of stock at the trustworthy resellers and marked up to ~$180-200+ where available. The list price you see (~$80) is not an attainable purchase price right now.
2. **Buy the GL.iNet Flint 2 (GL-MT6000)** as the default: same MT7986AV/MT7976 silicon, the most mature OpenWrt MT7986 target, confirmed 802.11s, 4 external antennas, 1 GB RAM, 2.5 GbE, in stock at ~$170 (watch for ~$140-160 sales). It's a packaged router (case + PSU included) so it's also less assembly than any bare board.
3. **If budget is the hard limit**, the **Xiaomi Redmi AX6000** (~$90-100, 6 external antennas, same SoC) is the cheapest true MT7986 — accept 512 MB RAM and a fiddlier flash.
4. **If you want bare-board / cheapest-per-node mesh**, drop one tier to **CMCC RAX3000M (MT7981B)** — same MT76 mesh stack and `kmod-mt7986-firmware`, fully OpenWrt-supported, ~$30-50 on AliExpress (2x2 instead of 4x4).
5. Remember the **MT76 802.11s bandwidth bugs are chip-wide** — they are not a reason to pay the BPI-R3 Mini premium, since the premium board has the exact same limitation.

---

## Sources

- GL.iNet Flint 2 product page — https://www.gl-inet.com/products/gl-mt6000
- Flint 2 OpenWrt forum (support, 802.11s, antennas) — https://forum.openwrt.org/t/gl-inet-flint-2-gl-mt6000-discussions/173524
- ServeTheHome Flint 2 review — https://www.servethehome.com/gl-inet-gl-mt6000-flint-2-wifi-router-review-mediatek-openwrt/
- Redmi AX6000 OpenWrt support thread — https://forum.openwrt.org/t/add-openwrt-support-for-xiaomi-redmi-ax6000/125008
- Redmi AX6000 ImmortalWrt firmware selector — https://firmware-selector.immortalwrt.org/?version=23.05.4&target=mediatek/filogic&id=xiaomi_redmi-router-ax6000
- MT76 802.11s mesh bugs — https://github.com/openwrt/openwrt/issues/12905 and https://github.com/openwrt/openwrt/issues/13112
- TP-Link EX820v OpenWrt PR — https://github.com/openwrt/openwrt/pull/13900
- TP-Link VX830v OpenWrt thread — https://forum.openwrt.org/t/support-for-tp-link-vx830v-mt7986/246657
- BPI-R3 Mini docs — https://docs.banana-pi.org/en/BPI-R3_Mini/BananaPi_BPI-R3_Mini
- BPI-R3 docs — https://docs.banana-pi.org/en/BPI-R3/BananaPi_BPI-R3
- ameriDroid BPI-R3 Mini (out of stock, $81.95) — https://ameridroid.com/products/banana-pi-bpi-r3-mini-w-case-antennas-and-power-supply
- ameriDroid BPI-R3 full (out of stock, $109.95) — https://ameridroid.com/products/banana-pi-bpi-r3-open-source-router
- TME BPI-R3 Mini (withdrawn) — https://www.tme.com/us/en-us/details/bpi-r3-mini/single-board-computers/sinovoip/banana-pi-bpi-r3-mini/
- eBay BPI-R3 Mini (out of stock) — https://www.ebay.com/itm/315612208984
- youyeetoo BPI-R3 / BPI-R3 Mini — https://youyeetoo.com/products/banana-pi-bpi-r3 and https://youyeetoo.com/products/bpi-r3-mini
- CMCC RAX3000M (MT7981) OpenWrt firmware selector — https://firmware-selector.openwrt.org/?target=mediatek%2Ffilogic&id=cmcc_rax3000m
- CMCC RAX3000M OpenWrt support PR — https://github.com/openwrt/openwrt/pull/13513
