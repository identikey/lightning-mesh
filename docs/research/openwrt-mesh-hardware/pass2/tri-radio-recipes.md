# Tri-Radio OpenWrt Mesh Hardware — Recipes (Pass 2)

**Goal:** A caseless/bare OpenWrt board running **802.11s mesh** with **THREE independent
WiFi radios** (e.g. one fixed non-DFS 5 GHz dedicated to mesh backhaul + a 2.4 GHz client
radio + a second 5 GHz or 6 GHz client radio). Two radios is the floor, three is the target.
Mainline OpenWrt drivers required — prefer MediaTek **mt76** (mt7915/mt7916/mt7986);
**ath9k / ath10k** acceptable.

**Research date:** 2026-06. Sources are linked inline and listed at the end.

---

## The single most important finding: DBDC cards count as TWO radios

A MediaTek **MT7915 / MT7916 DBDC ("Dual-Band Dual-Concurrent")** mPCIe/M.2 card
enumerates in Linux/OpenWrt as **two completely independent mt76 PHYs** — one 2.4 GHz
radio and one 5 GHz radio that operate simultaneously. Vendor and forum confirmation:
"One DR7915 module is detected as two independent MT7915E radios, one for 2.4 GHz
802.11bgnax and second for 5 GHz 802.11nacax, with both radios working at the same time."
([524wifi](https://524wifi.net/qca9882-qca9880-and-mt7915-wifi-cards-for-openwrtwhat-are-the-difference/),
[AsiaRF AW7915-NPD](https://asiarf.com/product/wifi-6-aw7915-npd-11ax-2x2-dbdc-1800mbps-mini-pcie-module-precision-coaxial-cables-and-wi-fi-antenna-dipole-dual-bands-2-4-5ghz/))

**Consequence:** you do NOT need three physical card slots to get three radios.
- 1 onboard dual-band SoC radio set (= 2 PHYs) **+ 1 DBDC mPCIe card** (= 2 PHYs) = **4 radios**.
- 1 DBDC card **+** 1 DBDC card = **4 radios** from two slots.
- A board with a dual-band onboard radio already provides **2 radios** before any card.

This makes the BPI-R3 (onboard dual-band) and the LibreRouter family far more capable than
a naive "one chip = one radio" count suggests, and it means **two slots is plenty** for the
3-radio goal on most boards.

Caveat: the two PHYs on a single DBDC chip share the chip's CPU/DMA and antenna real estate.
For a node where one radio must be a rock-solid dedicated backhaul, prefer putting the
**backhaul on its own physical chip** (separate card or the onboard radio), and let a single
DBDC card cover the two client bands.

---

## Tri-radio Recipes

| # | Recipe | Independent radios | Chips / driver | Bands per radio | Antenna conns | 802.11s | Caseless? | Form factor | Price (board+radios) | Key gotchas |
|---|--------|-------------------|----------------|-----------------|---------------|---------|-----------|-------------|----------------------|-------------|
| **A** | **LibreRouter LR1** (open HW) | **3** (1×2.4 + 2×5 GHz) | AR9558 SoC radio + 2× AR9582 mPCIe / **ath9k** | 2.4 only; 5 GHz; 5 GHz | 8 (3+2+2, +1 spare) | **Yes** (LibreMesh ships 802.11s) | Comes in weatherproof case; board is open HW (build your own) | Custom PCB, QCA9558 750 MHz, 128 MB RAM | Order via email to AlterMundi; ~$? (no public 2026 price); **WiFi 4/5 only**, no WiFi 6 | Ancient SoC (16 MB flash, 128 MB RAM); availability is email-only/batch; ath9k 5 GHz only on AR9582 |
| **B** | **LibreRouter "LRr2"** (next-gen, open HW) | **3** (1×2.4 + 2×5 GHz) | MT7621A SoC + ath9k 2.4 GHz + 2× MT7915 5 GHz / **mt76 + ath9k** | 2.4; 5 GHz ax; 5 GHz ax | ~6–8 | **Yes** (concurrent mesh+AP designed in) | Bare PCB (≈$42/unit batch BOM) | MT7621 router board, 256 MB RAM | ~$300–400 est. retail | **Prototype/active dev — not buyable off-the-shelf in 2026**; MT7621 is modest; switch/RGMII ~500 Mbps cap |
| **C** | **BPI-R4 + 1 DBDC mt7916 card** | **2** (card only; **no onboard radio**) → add 2nd card for **4** | 2× AsiaRF AW7916-NPD (mt7916) / **mt76** | per card: 2.4 + 5/6 GHz | 4–6 per card | **Yes** (mt76 802.11s) | **Bare board** | MT7988A (Filogic 880) A73 quad, 4/8 GB | board ~$95–100 + ~$60–90/card | **R4 has NO onboard WiFi** — all radios from the 2 mPCIe slots; each AsiaRF card wants **3.3 V @ 3 A (9 W)**; thermals; WED offload buggy |
| **D** | **BPI-R4 + BE14 tri-band module** | **3** (2.4 + 5 + 6 GHz) on one module | MT7995AV + MT7976CN + MT7977IAN (WiFi 7) / **mt76 (mt7996)** | 2.4; 5; 6 GHz | 6 | mt76/mt7996 802.11s (newer, verify) | **Bare board** | MT7988A | board ~$95 + BE14 ~$74 | BE14 **occupies BOTH mPCIe slots** (it is a dual-mPCIe card) → no room for a 4th radio; mt7996 802.11s maturity less proven than mt7915 |
| **E** | **BPI-R3 (onboard dual-band) + 1 DBDC mPCIe card** | **4** (onboard 2.4+5, card 2.4+5) — use 3 | MT7986 + MT7975 onboard + AsiaRF mt7915/16 in M.2/mPCIe / **mt76** | onboard 2.4; onboard 5; card 2.4; card 5 | 4 onboard + 2–4 card | **Yes** (mt76 802.11s, widely run on R3) | **Bare board** | MT7986A (Filogic 830) A53 quad, 2 GB | board ~$100 + card ~$60–90 | Only **M.2 Key-B/Key-M** expansion (no true mPCIe) — needs M.2↔mPCIe adapter or M.2 card; verify slot is wired for WiFi card not just NVMe/5G |
| **F** | **x86 mini-PC / SBC + 2–3 mt76 cards** | **3–6** (2 DBDC cards = 4; 3 cards = 6) | 2–3× AsiaRF AW7915-NP1 (4T4R) or AW7916-NPD / **mt76** | each card DBDC 2.4+5 (or 6E) | 4 per 4T4R card (16+ total) | **Yes** (mt76 802.11s) | Bare boards exist; most mini-PCs are cased | varies: N100 board, DFI ADN/ASL-553, RK3588 SBC | ~$150–300 host + ~$70–90/card | **Power is the killer**: each 4T4R card = up to **9 W on 3.3 V (need 3 A/slot)**; few mainboards deliver this on >1 slot; heat ("running hot, sort cooling"); **PC Engines APU2 is EOL** |
| **G** | **PC Engines APU2/4 + 2–3 ath9k/ath10k cards** | **2–3** | Compex WLE600/900 (ath10k) + ath9k / **ath9k+ath10k** | 1 band each (ath9k 2.4 or 5; ath10k 5) | 2–3 per card | **Yes** (ath9k solid; ath10k needs non-CT fw) | Bare board (no case) | AMD GX-412TC, 3× mPCIe (APU2) | **EOL — secondhand only** | APU2 **discontinued**; ath10k 802.11s needs non-ct firmware + rawmode; old WiFi 5 |

---

## Path-by-path detail

### Path 1 — Single boards with 3 radios onboard / via slots

**LibreRouter LR1 (Recipe A).** The reference open-hardware community-mesh router. Confirmed
configuration: **on-SoC 2.4 GHz radio (AR9558) + 2× AR9582 5 GHz mPCIe cards = 3 ath9k
radios**, 2× GbE via QCA8337, ~128 MB RAM / 16 MB flash, 12–36 V PoE.
Ships with LibreMesh (802.11s mesh is the whole point). Sold **assembled in a weatherproof
enclosure**, but it is open hardware so a bare build is possible from the published design
files. **Purchasing is email-only** (librerouter@altermundi.net, batch/quote model); no
public 2026 storefront price was found — treat availability as artisanal/low-volume.
**Biggest drawback: it is WiFi 4/5 (802.11n/an) only — no WiFi 6.**
([CNX](https://www.cnx-software.com/2020/01/29/librerouter-is-an-open-source-hardware-router-for-community-networks/),
[How to Buy/Build](https://foro.librerouter.org/t/how-to-buy-build/167),
[librerouter.org](https://librerouter.org/))

**LibreRouter LRr2 (Recipe B).** The next-gen redesign. **MT7621A SoC + ath9k 2.4 GHz +
2× MT7915 5 GHz (WiFi 6) = 3 radios** on **mt76 + ath9k**, concurrent mesh(802.11s)+AP by
design, bare PCB (~$42/unit at batch BOM cost, ~$300–400 est. retail). This is the closest
thing to a purpose-built tri-radio WiFi-6 open mesh board — **but it is still in
prototyping/active development and not generally purchasable in 2026.** RGMII/switch caps
routed throughput around ~500 Mbps.
([LRr2 hardware notes](https://hackmd.io/@ardcdesarrollo/r1j6_Vo2ye))

**Banana Pi BPI-R4 (Recipes C/D).** MT7988A (Filogic 880), WiFi-7-class router board, **bare**,
~$95–100. **Critical: the R4 has NO onboard WiFi radio.** All radios come from its **2× mPCIe
slots (PCIe 3.0 x2)** plus M.2. Two ways to hit 3 radios:
- **C:** populate slots with DBDC mt7916 cards. One AW7916-NPD = 2 radios; two cards = 4.
- **D:** the **BPI-R4-NIC-BE14** (~$74) is a **tri-band WiFi 7 module = 3 radios in one**
  (MT7995AV + MT7976CN 2.4/5 + MT7977IAN 6 GHz) — but it **spans both mPCIe slots**, so it
  is the whole radio subsystem (no slot left for a 4th).
([CNX BE14](https://www.cnx-software.com/2024/07/09/banana-pi-bpi-r4-nic-be14-wifi-7-dual-mini-pcie-module-for-banana-pi-bpi-r4-sbc/),
[BPI-R4 docs](https://docs.banana-pi.org/en/BPI-R4/BananaPi_BPI-R4),
[BPI-R4 Pro review](https://www.androidpimp.com/embedded/banana-pi-bpi-r4-pro/))
Note: the newer **BPI-R4 Pro (~$165)** adds more 10G/2.5G ports but the WiFi story is the same
(NIC-based).

No mainstream **Filogic board exposes three independent radio card slots**; the dual-mPCIe
pattern (R4) plus DBDC counting is how you reach 3+ on MediaTek.

### Path 2 — SBC / x86 host + multiple radio cards (Recipe F/G)

**Cards.** The workhorses on mt76:
- **AsiaRF AW7915-NP1** — WiFi 6, MT7915, **4T4R**, 2401 Mbps, **4 antenna connectors**;
  power **9 W max / 4–8 W avg, requires 3.3 V @ 3 A (2.5 A min) per slot.**
- **AsiaRF AW7916-NPD** — WiFi 6E, MT7916, DBDC 2.4+5/6, same ~9 W / 3.3 V 3 A envelope.
- **AsiaRF AW7915-NPD** — WiFi 6, MT7915 **2T2R DBDC** (lighter, lower power), good when you
  just want two client bands from one card.
All are mainline **mt76** (work on OpenWrt 21.02+ with no extra driver; WPA3 supported).
([AW7915-NP1](https://asiarf.com/product/wifi-6-11ax-4t4r-mini-pcie-module-mt7915-aw7915-np1/),
[AW7916-NPD](https://asiarf.com/product/wi-fi-6e-mini-pcie-module-mt7916-aw7916-npd/))

**Host boards / slot counts.**
- **PC Engines APU2/APU4** had **3× mPCIe** and was the classic choice — **now EOL**, secondhand only.
  ([PC Engines EOL](https://news.ycombinator.com/item?id=35635900))
- **No clean current x86 replacement** with 3 mPCIe + fanless + <$200 exists; the OpenWrt
  community explicitly notes this gap. Closest: **DFI ADN-553 / ASL-553** (3× i225 + **3× M.2**,
  M.2 not mPCIe), N100 mini-PCs (usually 1× M.2 + 1× mPCIe), or **RK3588 NAS SBCs** with up to
  4 mPCIe.
  ([Seeking APU2-like board](https://forum.openwrt.org/t/seeking-apu2-like-board-dual-mini-pcie-intel-like-ethernet-features-ptp-tc-offload/248674),
  [x86 alternatives](https://forum.openwrt.org/t/x86-nics-and-in-stock-alternatives-to-pc-engines-apu2/110203))

**The hard constraints (OpenWrt forum consensus):**
- **Power:** 4T4R mt76 cards "tend to be off-spec, needing 10+ W on 3.3 V — not all mainboards
  can deliver that and/or will burn out trying." Two or three such cards on one board is exactly
  where mainboards fail to supply 3.3 V rail current.
- **Thermals:** cards run hot; passive + active cooling required.
- **Antennas/pigtails:** 4–8 pigtails + 4–8 antennas per multi-card build (cost + assembly).
- **PCIe enumeration:** generally OK for mt76, but undersized 3.3 V can cause cards to not
  enumerate or to drop under load.
([x86 WiFi card options/limitations](https://forum.openwrt.org/t/x86-64-wifi-card-options-and-limitations/193760))
Because each DBDC card is 2 radios, **2 cards (= 4 radios) on a host with 2 well-powered slots
is the sweet spot** — you usually do NOT need a true 3-slot host to hit 3 radios.

### Path 3 — Mixed onboard + add-in (Recipe E)

**BPI-R3** (MT7986A Filogic 830, ~$100, **bare**) has a **dual-band onboard radio (MT7975N
2.4 GHz + MT7975P 5 GHz) = 2 radios already**, all mt76 with mature 802.11s. Add **one DBDC
mt7915/mt7916 card** → **4 radios total** (use 3: onboard 5 GHz for backhaul, onboard 2.4 GHz
client, card 5 GHz client). Caveat: the R3 expands via **M.2 (Key-B / Key-M)**, not a true
mPCIe slot — you need an M.2-form mt76 card or an M.2↔mPCIe adapter, and must confirm the slot
is wired/powered for a WiFi card (some are intended for NVMe/5G). mt7915 on R3 is a known-working
combo, though some users hit MT7916-on-R3 quirks.
([BPI-R3 wiki](https://wiki.banana-pi.org/Banana_Pi_BPI-R3),
[CNX BPI-R3](https://www.cnx-software.com/2022/09/05/banana-pi-bpi-r3-wifi-6e-router-board-mediatek-filogic830-mt7986-soc/),
[MT7916 on R3 issue](https://forum.openwrt.org/t/issue-utilizing-mt7916-on-bpi-r3/170140))

---

## 802.11s on mt76 — maturity note (read before committing)

802.11s **works** on mt7915/mt7916 with mainline mt76 (no special flags, unlike ath10k which
needs non-CT firmware + rawmode). But there is a real bug-tail you should plan around:
- WPA3-encrypted 802.11s historically flaky on 2.4 GHz; 5 GHz generally better.
- Past reports of 5 GHz mesh restricted to certain channels / 20 MHz BW; channel/DFS handling
  has been the weak spot — which is exactly why a **fixed non-DFS 5 GHz backhaul channel** (your
  plan) is the right call.
- ath9k 802.11s is the most battle-tested of all (LibreMesh runs it at scale).
([mt76 #675 5 GHz mesh](https://github.com/openwrt/mt76/issues/675),
[OpenWrt #12905 mesh/MT76](https://github.com/openwrt/openwrt/issues/12905),
[OpenWrt #13112 20 MHz BW](https://github.com/openwrt/openwrt/issues/13112),
[ath10k mesh/rawmode](https://forum.openwrt.org/t/ath10k-ct-with-802-11s/64932))

**Mesh+AP concurrency:** running a mesh-point and an AP on the **same** PHY works but loads that
radio. With 3+ radios you should **dedicate one PHY to mesh and keep AP service on the others**
— which is the whole point of going tri-radio.

---

## Recommended tri-radio build

**Primary recommendation — BPI-R3 + one DBDC mt76 card (Recipe E).**
Best balance of *buyable today*, *bare board*, *all-mt76 mainline*, *mature 802.11s*, and
*power sanity*:

- **Board:** Banana Pi **BPI-R3** (MT7986A, ~$100, caseless). Onboard MT7975 dual-band gives
  you **2 radios out of the box** with the best-tested MediaTek mesh stack.
- **Add-in:** one **AsiaRF AW7916-NPD** (WiFi 6E DBDC, mt7916) or **AW7915-NPD** (WiFi 6 2T2R
  DBDC, lower power) in the M.2 slot (via M.2 card or adapter) → **+2 radios = 4 total, use 3.**
- **Radio plan:** onboard **5 GHz → dedicated 802.11s backhaul on a fixed non-DFS channel**;
  onboard **2.4 GHz → client AP**; card **5 GHz (or 6 GHz on the 7916) → second client AP**.
- **Why:** only ONE add-in card → only one 3.3 V/3 A power concern, far easier than a 2–3 card
  x86 build; single bare PCB; all radios mt76; proven mesh. Build your own case as planned.

**If you want WiFi-6 backhaul on a separate physical chip and 10G uplink — BPI-R4 + 2 DBDC
cards (Recipe C).** ~$95 board + 2× AW7916-NPD (~$150). Gives **4 radios across two physical
chips** (cleaner backhaul isolation than recipe E), bare board, mt76. Budget for **two 3.3 V/3 A
slots and active cooling**, and avoid the BE14 module if you want to keep a slot per chip.

**If open-hardware / community-mesh provenance matters more than WiFi 6 — LibreRouter LR1
(Recipe A).** True 3-radio, ath9k, 802.11s-native, open HW — but WiFi 4/5 only and email-order.
Watch the **LRr2** (Recipe B): once it ships it is the ideal purpose-built tri-radio WiFi-6 open
mesh board, but it is **not buyable in 2026 yet**.

**Avoid as a starting point:** a fresh 3-card x86 build (Recipe F/G). The APU2 is EOL, no clean
3-mPCIe fanless x86 replacement exists, and the per-slot 3.3 V/3 A power + thermal burden on
multiple 4T4R cards is the most failure-prone path. Use it only if you already own an APU-class
box — and even then, 2 DBDC cards (4 radios) beats 3 single cards on power.

---

## Sources

- LibreRouter: [CNX overview](https://www.cnx-software.com/2020/01/29/librerouter-is-an-open-source-hardware-router-for-community-networks/) · [How to Buy/Build](https://foro.librerouter.org/t/how-to-buy-build/167) · [librerouter.org](https://librerouter.org/) · [LRr2 hardware](https://hackmd.io/@ardcdesarrollo/r1j6_Vo2ye)
- BPI-R4 / BE14: [BPI-R4 docs](https://docs.banana-pi.org/en/BPI-R4/BananaPi_BPI-R4) · [BE14 module (CNX)](https://www.cnx-software.com/2024/07/09/banana-pi-bpi-r4-nic-be14-wifi-7-dual-mini-pcie-module-for-banana-pi-bpi-r4-sbc/) · [BPI-R4 Pro review](https://www.androidpimp.com/embedded/banana-pi-bpi-r4-pro/) · [BPI-R4 + AsiaRF 7916 build](https://forum.banana-pi.org/t/bpi-r4-building-with-asiarf-7916-need-help/17389) · [BPI-R4 WED offload bug](https://forum.openwrt.org/t/bpi-r4-wed-wifi-offloading-with-mt7916an-pcie-wifi-card/246380)
- BPI-R3: [Banana Pi wiki](https://wiki.banana-pi.org/Banana_Pi_BPI-R3) · [CNX BPI-R3](https://www.cnx-software.com/2022/09/05/banana-pi-bpi-r3-wifi-6e-router-board-mediatek-filogic830-mt7986-soc/) · [MT7916-on-R3 issue](https://forum.openwrt.org/t/issue-utilizing-mt7916-on-bpi-r3/170140)
- DBDC = 2 radios: [524wifi card comparison](https://524wifi.net/qca9882-qca9880-and-mt7915-wifi-cards-for-openwrtwhat-are-the-difference/) · [AsiaRF AW7915-NPD](https://asiarf.com/product/wifi-6-aw7915-npd-11ax-2x2-dbdc-1800mbps-mini-pcie-module-precision-coaxial-cables-and-wi-fi-antenna-dipole-dual-bands-2-4-5ghz/)
- Cards/power: [AW7915-NP1 4T4R](https://asiarf.com/product/wifi-6-11ax-4t4r-mini-pcie-module-mt7915-aw7915-np1/) · [AW7916-NPD WiFi 6E](https://asiarf.com/product/wi-fi-6e-mini-pcie-module-mt7916-aw7916-npd/)
- x86 multi-card / APU2 EOL: [x86 WiFi card options/limitations](https://forum.openwrt.org/t/x86-64-wifi-card-options-and-limitations/193760) · [PC Engines EOL](https://news.ycombinator.com/item?id=35635900) · [Seeking APU2-like board](https://forum.openwrt.org/t/seeking-apu2-like-board-dual-mini-pcie-intel-like-ethernet-features-ptp-tc-offload/248674) · [x86 in-stock alternatives](https://forum.openwrt.org/t/x86-nics-and-in-stock-alternatives-to-pc-engines-apu2/110203)
- mt76 802.11s maturity: [mt76 #675 (5 GHz mesh)](https://github.com/openwrt/mt76/issues/675) · [OpenWrt #12905 (MT76 mesh)](https://github.com/openwrt/openwrt/issues/12905) · [OpenWrt #13112 (20 MHz BW)](https://github.com/openwrt/openwrt/issues/13112) · [ath10k mesh/rawmode](https://forum.openwrt.org/t/ath10k-ct-with-802-11s/64932)
