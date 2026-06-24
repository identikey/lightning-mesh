# Cheap MT7981 (Filogic 820) Routers for OpenWrt 802.11s Mesh

**Research pass 3 — budget hardware sourcing**
**Date:** 2026-06-24
**Goal:** Find cheap (<$150, ideally $30–100) consumer routers on the **MediaTek MT7981B / Filogic 820** SoC that flash to **mainline OpenWrt** and support **802.11s mesh**, with external antennas a plus, and that are realistically **in stock**.

---

## TL;DR

- **All MT7981B devices use the same `mediatek/filogic` OpenWrt target with the mt76 driver** (`kmod-mt7915e` + `kmod-mt7981-firmware`). 802.11s mesh works on this silicon family. The radio capability is identical across every model below; differences are PCB, antennas, flash chip, and flashing pain.
- **Cheapest + easiest in-stock pick: Cudy WR3000 (v1)** — ~$30–40, official OpenWrt, web-UI flash, widely stocked on Amazon US. Antennas are **fixed (non-removable)**.
- **Best external-antenna + cheap pick: CMCC RAX3000M** — ~$25–40 on AliExpress, official OpenWrt, 4× external 5 dBi antennas (some variants non-removable; emmc/nand variants exist).
- **Avoid for beginners: Xiaomi AX3000T** — cheapest sticker price (~$40) but flashing requires exploit/SSH + a **flash-chip lottery (Winbond vs ESMT)** that soft-bricks units. Antennas fixed.
- **Most plug-and-play (premium): GL.iNet GL-MT3000 (Beryl AX)** — ~$80–90, ships with GL's OpenWrt, mainline supported, but antennas are tiny retractable internal and 802.11s needs manual config.

> ⚠️ **External antennas caveat:** Almost every cheap MT7981 consumer router has **fixed (soldered/glued) external whip antennas**, NOT removable RP-SMA. True RP-SMA removable antennas in this class are rare — mostly found on industrial Zbtlink 5G/LTE CPE units (more expensive, $90+). If high-gain external antennas are a hard requirement, plan to either (a) buy a Zbtlink industrial unit, or (b) modify a fixed-antenna unit, or (c) accept fixed whips.

---

## Ranked comparison table

| Rank | Model | SoC | Price (USD) | Where to buy / in-stock | Ext. antennas | OpenWrt / flashing | 802.11s |
|------|-------|-----|-------------|--------------------------|----------------|---------------------|---------|
| 1 | **Cudy WR3000 (v1)** | MT7981B | **~$30–40** | Amazon US (B0BRK3CYY3), Cudy store, AliExpress — **widely in stock** | 4×5 dBi **fixed** | Official ToH, web-UI flash. ⚠️ check S/N: ≥2543 = "new flash" revision, different procedure | ✅ |
| 2 | **CMCC RAX3000M** | MT7981B | **~$25–40** | AliExpress, Bunjang — **in stock** (Chinese ISP surplus, plentiful) | 4×5 dBi **external** (non-removable) | Official ToH. NAND & eMMC variants; uboot procedure | ✅ |
| 3 | **Cudy TR3000** | MT7981B | **~$45–55** | Amazon US, Cudy store, AliExpress — **in stock** | Internal (compact "mini VPN router") | Official ToH, web-UI flash, easy | ✅ |
| 4 | **Routerich AX3000 (v1, ZR-3020)** | MT7981B | **~$35–55** | AliExpress, routerich.ru — **in stock** (RU/CIS seller) | Internal (none specified / non-removable) | OpenWrt 24.10 runs well; community/snapshot | ✅ |
| 5 | **Xiaomi AX3000T** | MT7981B | **~$40–45** | AliExpress, Amazon (gray-market), microless — **in stock, cheapest sticker** | 4 **fixed** external whips | ⚠️ **Hardest**: needs exploit/SSH unlock; **Winbond-flash units soft-brick** on stock OpenWrt (use ImmortalWrt/X-Wrt or wait for patch). UART/JTAG recovery common | ✅ |
| 6 | **Cudy AP3000 (Wall / Outdoor)** | MT7981B | **~$42–50** | Cudy store, AliExpress | Internal (AP form factor) | Official ToH, web-UI flash | ✅ |
| 7 | **Cudy WR3000H / WR3000S / WR3000P** | MT7981B | **~$40–60** | Amazon US, Cudy store | 4×5 dBi **fixed** | Official ToH (WR3000H added PR #17458); WR3000S/P 2.5G & PoE variants | ✅ |
| 8 | **GL.iNet GL-MT3000 (Beryl AX)** | MT7981B | **~$80–90** | Amazon US (B0BPSGJN7T), Walmart, GL.iNet store — **in stock** | 2× tiny retractable (internal-ish) | Ships with GL OpenWrt; **mainline supported**. Easiest flash, but 802.11s needs manual CLI config (no LuCI mode menu by default) | ✅ (manual) |
| 9 | **Netgear EAX15 v2 / v3** | MT7981 | ~$30–60 (used/refurb) | eBay, Amazon refurb — **spotty stock** | Internal | EAX15 **v2 official in 25.12**; v3 community WIP | ✅ |
| 10 | **TP-Link Archer AX55 Pro v2 / EX520** | MT7981B | ~$50–80 | Amazon (AX55 Pro), ISP-branded (EX520) | Internal/fixed | **Community WIP, not yet mainline-merged** — avoid for now | ✅ (once supported) |
| 11 | **Zbtlink Z8103AX / Z8102AX / Z8105AX** | MT7981B | **~$60–120+** | zbtwifi.com, Amazon, 524wifi, AliExpress — **in stock** | **7–8× external 5 dBi (best for high-gain), some removable** | OpenWrt (vendor + community); industrial 5G/LTE CPE | ✅ |
| — | **Acelink EW-7811 / JCG** | — | n/a | — | — | **Not confirmed MT7981 / no clear ToH entry** — could not verify; skip | ? |

---

## Per-model detail

### 1. Cudy WR3000 (v1) — best cheap + easy
- **SoC:** MT7981B + MT7976 RF. 256 MB RAM, 128 MB SPI-NAND.
- **OpenWrt:** Official. Firmware selector: `https://firmware-selector.openwrt.org/?target=mediatek%2Ffilogic&id=cudy_wr3000` (and `cudy_wr3000s-v1`). Flash via Cudy's OpenWrt factory image through the stock web UI — one of the easiest in this list.
- **⚠️ Flash-revision warning:** Units with serial number **≥ 2543** (mfg Nov 2025+) ship a **"New Flash" hardware revision**; do NOT use the old install guide or you can brick it. Check the S/N sticker first. See: sergiogimenez.com OpenWrt-Cudy-WR3000 guide.
- **Antennas:** 4× 5 dBi **fixed** (non-removable).
- **Price/stock:** ~$30–40. Amazon US `B0BRK3CYY3` (WR3000 V2.0), Cudy official store, AliExpress. Reliably in stock.
- **802.11s:** ✅ standard mt76.

### 2. CMCC RAX3000M — cheapest external-antenna pick
- **SoC:** MT7981B, 512 MB RAM, **128 MB SPI-NAND or 64 GB eMMC** variant. 4× GbE.
- **OpenWrt:** Official (PR #13513, merged Oct 2023). Selector: `https://firmware-selector.openwrt.org/?target=mediatek%2Ffilogic&id=cmcc_rax3000m`. uboot-mediatek added a DDR3 build (Sept 2025) — confirm your RAM/flash variant before flashing.
- **Antennas:** 4× external 5 dBi (high-gain) — but **non-removable** on standard units.
- **Price/stock:** ~$25–40 on AliExpress (Chinese ISP surplus, very plentiful). Often the cheapest real MT7981 with external antennas. Watch for "used/refurb" listings.
- **802.11s:** ✅.

### 3. Cudy TR3000 — compact, very easy flash
- **SoC:** MT7981B, 2.5G port, "mini VPN router" form factor.
- **OpenWrt:** Official (added May 2024, lede-commits). Web-UI flash, easy.
- **Antennas:** Internal (compact). No external option.
- **Price/stock:** ~$45–55 (seen at €49 in EU). Amazon US, Cudy store, AliExpress.
- **802.11s:** ✅.

### 4. Routerich AX3000 (v1 / ZR-3020)
- **SoC:** MT7981BA + MT7976CN, 512 MB DDR3, 128 MB SPI-NAND (Winbond), MT7531AE switch. 4×LAN+1×WAN GbE, 1× USB 2.0, 3.3 V UART console.
- **OpenWrt:** Runs OpenWrt 24.10.2 (6.6.x) well; overclockable. Community/snapshot support, good forum activity.
- **Antennas:** "none specified" connector = **internal, not removable**.
- **Price/stock:** ~$35–55 on AliExpress; vendor routerich.ru. In stock via RU/CIS sellers (shipping varies by region).
- **802.11s:** ✅.

### 5. Xiaomi AX3000T — cheapest sticker, hardest flash
- **SoC:** MT7981B, 256 MB RAM, **128 MB NAND (ESMT F50L1G41LB *or* Winbond W25N01KV)**, MT7976C RF.
- **OpenWrt:** Official ToH entry exists: `https://firmware-selector.openwrt.org/?target=mediatek%2Ffilogic&id=xiaomi_mi-router-ax3000t`.
- **⚠️ Flashing is the catch:**
  - Requires unlocking (exploit / SSH via known CVE flow or UART) — not a simple web-UI flash.
  - **Flash-chip lottery:** units with the **Winbond W25N01KV** chip **soft-brick** on stock OpenWrt until a patch; **ImmortalWrt / X-Wrt work fine**. Identify the chip by shining a torch through the case underside (reads "Winbond" or "ESMT"). See openwrt/openwrt issue #16002.
  - Bricked units are common in forums; UART/JTAG recovery threads abound.
- **Antennas:** 4 **fixed** external whips (1×2.4+5, 2×5, 1×2.4).
- **Price/stock:** ~$40–45, very widely available on AliExpress (cheapest sticker price of all). Amazon listings are gray-market.
- **802.11s:** ✅ once flashed.
- **Verdict:** Only pick this if you want absolute lowest cost and are comfortable with exploit-flashing + chip verification. Otherwise the WR3000/RAX3000M are far less hassle for similar money.

### 6–7. Cudy AP3000 / WR3000H / WR3000S / WR3000P
- All MT7981B, all **official OpenWrt** (Cudy is one of the most OpenWrt-friendly vendors; they publish factory OpenWrt images). WR3000H/S add a 2.5G port; WR3000P adds PoE-in. AP3000 is an AP/ceiling/outdoor form factor.
- Antennas fixed (WR3000H/S: 4×5 dBi fixed) or internal (AP3000).
- Prices ~$40–60. Amazon US + Cudy store, generally in stock.

### 8. GL.iNet GL-MT3000 (Beryl AX) — easiest but pricier, weak antennas
- **SoC:** MT7981B @ 1.3 GHz, 2.5G WAN + 1G LAN. Travel-router form factor.
- **OpenWrt:** Ships with GL.iNet's OpenWrt; **mainline OpenWrt supported** (forum install threads exist). Easiest "it just works" device here.
- **Antennas:** 2× small **retractable** external + 1 internal — **low gain**, not suited to high-gain upgrades.
- **802.11s:** ✅ supported by silicon, but GL firmware/LuCI lacks a ready mesh-mode menu — **needs manual CLI config** (`/etc/config/wireless` mode=mesh). Confirmed on GL.iNet forum.
- **Price/stock:** ~$80–90. Amazon US `B0BPSGJN7T`, Walmart, GL.iNet store. Always in stock.
- **Verdict:** Great if you value zero flashing risk; poor if you want external high-gain antennas or cheapest cost.

### 9. Netgear EAX15 (v2/v3)
- MT7981-based (per GPL tarball). **EAX15 v2 got official OpenWrt in the 25.12 branch**; v3 still community WIP. Internal antennas. Stock is spotty (mostly used/refurb on eBay/Amazon). Lower priority unless you find one cheap.

### 10. TP-Link Archer AX55 Pro v2 / EX520
- AX55 Pro v2 (US) = MT7981B, 512 MB/128 MB, dual 2.5GE. EX520 = MT7981 (ISP-branded EX220 sibling). **Both are community WIP, not mainline-merged yet** (active forum threads). TP-Link also tends to lock bootloaders / require exploit flashing. **Skip until merged.**

### 11. Zbtlink Z8103AX / Z8102AX / Z8105AX / Z8106AX
- MT7981B industrial routers, many with **5G/4G LTE CPE** + dual-SIM. These are the **only ones in this list with genuinely many external (7–8×) high-gain antennas, some removable**. 1 GB DDR4 on some SKUs.
- OpenWrt: vendor + community support. Best choice **if external high-gain antennas are mandatory**, but pricier (~$60–120+) and overkill if you don't need cellular.
- Stock: zbtwifi.com, Amazon, 524wifi, AliExpress — in stock.

### Acelink / JCG
- **Could not confirm** an MT7981 model with a clear OpenWrt ToH entry for "Acelink EW-7811" or JCG in this pass. The EW-7811 string is more associated with Edimax USB adapters. **Recommend skipping** unless a specific verified model surfaces.

---

## Best value picks

1. **Cheapest reliably-in-stock + easy flash → Cudy WR3000 (v1).** ~$30–40, official OpenWrt, web-UI flash, everywhere on Amazon. The default recommendation for a mesh node fleet. (Just verify S/N < 2543 or follow the new-flash guide.) Antennas fixed.

2. **Cheapest WITH external antennas → CMCC RAX3000M.** ~$25–40 on AliExpress, 4× external 5 dBi, official OpenWrt. Best $/node if you're buying from AliExpress and want external (even if non-removable) antennas. Confirm NAND vs eMMC/DDR3 variant before flashing.

3. **If you want zero flashing risk and don't care about antennas → GL.iNet GL-MT3000.** ~$85, ships OpenWrt-friendly, mainline supported. Costs ~2–3× the Cudy though, and 802.11s needs manual config.

4. **If removable high-gain external antennas are a hard requirement → Zbtlink Z810x series.** ~$60–120, the only class here with 7–8 external (some removable) antennas. Overkill unless you also want cellular.

**Avoid for a fleet:** Xiaomi AX3000T (flash-chip brick lottery + exploit flashing) and TP-Link AX55 Pro/EX520 (not mainline-merged). Xiaomi is only worth it if you specifically want the absolute lowest sticker price and accept the flashing risk.

---

## Notes on 802.11s across the family

Every model above is the **same `mediatek/filogic` MT7981B target** using the **mt76 driver** (`kmod-mt7915e` + `kmod-mt7981-firmware` + `mt7981-wo-firmware`). 802.11s mesh-point mode works on this silicon. Known caveats from mt76 issue tracker (apply to the whole mt7915/mt7981/mt7916 family, not specific to any model here):
- Historically some 5 GHz 802.11s + WPA3-SAE mesh combos were flaky; 2.4 GHz mesh was more reliable. Status improves with newer OpenWrt (24.10 / 25.x).
- Use a recent OpenWrt release for best mesh behavior; pin and test 5 GHz mesh + encryption early in your bring-up.

---

## Sources

- [OpenWrt support for Xiaomi AX3000T — forum](https://forum.openwrt.org/t/openwrt-support-for-xiaomi-ax3000t/180490)
- [Xiaomi AX3000T softbricks (Winbond flash) — issue #16002](https://github.com/openwrt/openwrt/issues/16002)
- [Xiaomi AX3000T — OpenWrt firmware selector](https://firmware-selector.openwrt.org/?target=mediatek%2Ffilogic&id=xiaomi_mi-router-ax3000t)
- [Cudy OpenWrt collection](https://www.cudy.com/en-us/collections/openwrt)
- [Support for Cudy WR3000 — forum](https://forum.openwrt.org/t/support-for-cudy-wr3000/155463)
- [Cudy WR3000S-v1 — OpenWrt firmware selector](https://firmware-selector.openwrt.org/?version=24.10.0&target=mediatek%2Ffilogic&id=cudy_wr3000s-v1)
- [How to Install OpenWrt on the Cudy WR3000 (v1 retail) — Sergio Giménez](https://sergiogimenez.com/posts/2026/openwrt-cudy-w3000-v1/)
- [Cudy WR3000H OpenWrt support — PR #17458](https://github.com/openwrt/openwrt/pull/17458)
- [Supporting the Cudy TR3000 — forum](https://forum.openwrt.org/t/supporting-the-cudy-tr3000-in-openwrt/184912)
- [Cudy AX3000 WR3000 V2.0 — Amazon](https://www.amazon.com/Cudy-AX3000-WiFi-Router-Compatible/dp/B0BRK3CYY3)
- [CMCC RAX3000M — OpenWrt firmware selector](https://firmware-selector.openwrt.org/?target=mediatek%2Ffilogic&id=cmcc_rax3000m)
- [mediatek: add CMCC RAX3000M support — PR #13513](https://github.com/openwrt/openwrt/pull/13513)
- [CMCC RAX3000M DDR3 uboot build — lede-commits Sept 2025](https://lists.infradead.org/pipermail/lede-commits/2025-September/026955.html)
- [CMCC RAX3000M — TechInfoDepot](https://techinfodepot.shoutwiki.com/wiki/CMCC_RAX3000M)
- [Routerich AX3000 v1 — TechInfoDepot](https://techinfodepot.shoutwiki.com/wiki/Routerich_AX3000_v1)
- [Overclocking Routerich AX3000 mt7981 — forum](https://forum.openwrt.org/t/overclocking-routerich-ax3000-mt7981/238075)
- [GL.iNet GL-MT3000 (Beryl AX) product page](https://www.gl-inet.com/en-us/products/gl-mt3000)
- [GL-MT3000 — Amazon](https://www.amazon.com/GL-iNet-GL-MT3000-Pocket-Sized-Wireless-Gigabit/dp/B0BPSGJN7T)
- [Beryl AX 802.11s / 802.11r mode — GL.iNet forum](https://forum.gl-inet.com/t/beryl-ax-mt3000-802-11s-or-802-11r-mode-capability/28001)
- [Netgear EAX15 v3 (MT7981) OpenWrt — forum](https://forum.openwrt.org/t/openwrt-support-for-netgear-eax15-v3-mt7981/240492)
- [TP-Link Archer AX55 Pro V2 (MT7981B) OpenWrt — forum](https://forum.openwrt.org/t/adding-openwrt-support-for-tp-link-archer-ax55-pro-v2-us-mt7981b-2-5ge-x2/247116)
- [TP-Link EX520 (MT7981) OpenWrt — forum](https://forum.openwrt.org/t/tp-link-ex520-mediatek-mt7981-openwrt-support/241815)
- [Zbtlink AX3000 Z8103AX-C (MT7981) — zbtwifi](https://www.zbtwifi.com/products/ax3000-dual-band-wifi-6-mesh-router)
- [Zbtlink MT7981 5G CPE Z8105AX-C — zbtwifi](https://www.zbtwifi.com/products/mt7981-dual-sim-5g-cpe-ax3000-cpe-router-wifi6)
- [mt76 802.11s mesh broken (23.05) — issue #12905](https://github.com/openwrt/openwrt/issues/12905)
- [openwrt mt76 Makefile (kmod-mt7915e / mt7981 firmware)](https://github.com/openwrt/openwrt/blob/main/package/kernel/mt76/Makefile)
