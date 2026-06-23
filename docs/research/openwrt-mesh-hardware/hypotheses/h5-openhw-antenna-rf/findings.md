# Hypothesis: H5 — Open-source-hardware options & antenna/RF/form-factor considerations

## Summary
**Supported (with one important caveat).** Genuinely open-hardware boards that meet the mesh requirements exist, but they cluster at two ends: the **OpenWrt One** (modern, MT7981/mt76, WiFi 6, schematics published, but only 1 WiFi chip = effectively 1 dual-band radio) and the **LibreRouter LR1** (purpose-built 3-radio ath9k mesh router, EAGLE files open, but old QCA9558/ath9k silicon and chronically hard to buy). No board today combines *fully open design files* + *≥2 independent mt76/ath radios* + *current silicon* in one package — so the user will likely trade either radio count, silicon age, or design-file completeness. The antenna/RF/packaging guidance is well-established and unambiguous (use U.FL→RP-SMA pigtails to durable bulkheads; directional/sector antennas for backhaul).

## Evidence

### Part A — Open-hardware boards

**OpenWrt One (OpenWrt project + Banana Pi / WayPonDEV)** — strongest "modern + open" candidate
- SoC: MediaTek **MT7981B (Filogic 820)**; WiFi via **MT7976C dual-band WiFi 6 (AX3000)** chipset — this is **mt76-driver** silicon, exactly the user's preference [1][5].
- Radios/antennas: dual-band WiFi 6, advertised "3×3 / 2×2" with **detachable antennas**; community first-impressions and Reddit report **three MMCX antenna connectors** on the metal case (MMCX is notable — ~500 mating cycles, far more robust than U.FL) [2][7]. Note: this is **one WiFi chip** providing 2.4 + 5 GHz concurrent, not two independent radios on separate channels.
- 1 GB DDR4 RAM, 256 MiB NAND + 16 MiB NOR (dual-flash "unbrickable" recovery), **M.2 SSD/NVMe slot**, 1× 2.5 GbE WAN + 1× 1 GbE LAN, USB-C serial console + USB 2.0 [2][4][7].
- Power: **802.3af/at PoE via the 2.5 GbE WAN port**, plus USB-C [2].
- **Openness: schematics and datasheets are published at `https://one.openwrt.org/hardware/`** (the open MT7981B platform datasheet is hosted there) [2][3]. It is the first board *co-designed with the OpenWrt project*; mainline OpenWrt support is first-class [1][4].
- Availability/price: widely available via WayPonDEV/Amazon and the OpenWrt store; commonly ~US$89 (a portion donated to OpenWrt/SPI). Price not confirmed on the official wiki in this pass — flag as open question [4].

**LibreRouter LR1 (libremesh.org / LibreRouter project)** — the purpose-built open-hardware *mesh* router the context hints at
- SoC: **Qualcomm Atheros QCA9558** (MIPS, 750 MHz) — **ath9k** [6].
- Radios: **3 radios** — on-chip 2.4 GHz 802.11n 2×2, plus **two power-amplified 5 GHz 802.11an 2×2 radios based on the AR9582 chip** in two mPCIe slots. This is the genuine multi-radio mesh design (dedicated radios let one face uplink, one downlink, one client) [6][9].
- 128 MB DDR RAM, 16 MB flash, 2× Gigabit Ethernet (QCA8337N switch), **PoE + PoE passthrough, 12–36 V input, ~16 W** [6].
- **Openness: MEGA board design files published as EAGLE schematics on GitHub**; built on **OpenWrt + LibreMesh**; passed ENACOM (Argentina) certification 2019 [6]. Spec sheet describes a **weatherproof enclosure** with 3 radios (2.4/5/5 GHz) [9].
- **Availability is the Achilles heel**: long-standing community complaint (Hacker News thread, "no buy-it-now button") that it is hard to actually purchase; production has been intermittent/limited and tied to community-network funding [8].
- Silicon is **old (ath9k, 802.11n)** — no WiFi 5/6, lower throughput than mt76 boards.

**Banana Pi BPI-R3 / BPI-R4 (MediaTek Filogic, mt76)** — "open project" but not fully open *design files*
- BPI-R3: MT7986 (Filogic 830), mt76, dual-band WiFi 6, 2 GB RAM, multiple 2.5 GbE/SFP; BPI-R4: MT7988, WiFi 7 via BE14 module [4][owprice]. These are marketed as "open source router development boards" and have strong OpenWrt support, **but Banana Pi's "open source" generally means published docs/firmware, not always complete licensed schematics+Gerbers** — and forum threads note driver/blob dependence on MediaTek (e.g. the BPI-R4 "disappointment" thread about vendor-supplied drivers) [osh-boards]. Treat as "more open than typical, less open than OpenWrt One/LibreRouter."

**Community mesh-firmware ecosystem (not hardware, but defines the software target)**
- **LibreMesh** (libremesh.org) and **Freifunk Gluon** (github.com/freifunk-gluon/gluon) are the two mature OpenWrt-based mesh firmware frameworks; LibreMesh uses BATMAN-adv layering and is the LibreRouter's native firmware [community results]. These run on commodity ath9k/ath10k/mt76 OpenWrt hardware, so the user is not locked to a single board.

**Olimex** — open-hardware vendor, but a gap for this use case
- Olimex genuinely publishes **KiCad/EAGLE design files under GPL on GitHub** (OLINUXINO, DIY-LAPTOP HARDWARE dirs) and is a recognized OSHW vendor [olimex results]. **However, no Olimex product matches a 2-radio mt76/ath OpenWrt WiFi-router board** — their open boards are SBCs/industrial Linux boards, not multi-radio routers. So Olimex is a real OSHW vendor but **not a direct candidate** here.

### Part B — Antenna / RF / form-factor (well-corroborated, multiple sources)

**Connector durability & strategy** [10, oscarliang, tejte]:
- **U.FL / IPEX-1, MHF4, H.FL: ~30 mating cycles**, fragile, "not intended for repeated disconnects" — semi-permanent only.
- **MMCX: ~500 mating cycles** — much more robust (this is what OpenWrt One uses).
- **SMA / RP-SMA: ~500 mating cycles**, threaded, "robust and reliable in both indoor and outdoor applications."
- **Best practice for a custom enclosure:** route each board U.FL/IPEX jack through a short **U.FL-to-RP-SMA (or SMA) bulkhead pigtail** to the case wall. This isolates the fragile PCB jack from mechanical stress and gives a weatherproof, field-serviceable external connection [10].

**Number of antenna connectors:**
- One per spatial stream per radio: **2×2 = 2 connectors, 3×3 = 3, 4×4 = 4**, **per radio**. A 3-radio LibreRouter-style build with 2×2 radios = 6 connectors; OpenWrt One's single dual-band chip = 3 connectors (matches its 3 MMCX jacks).

**Antenna selection — omni vs directional** [11, 12, fs.com]:
- **Omni (360°)**: ~2–8 dBi, for local client coverage / nodes that mesh with neighbors in many directions.
- **Sector**: typically **12–18 dBi**, covers a wedge (e.g. 60/90/120°).
- **Directional panel/dish**: **20+ dBi**, narrow beam, long range.
- **Mesh backhaul benefits strongly from directional/high-gain**: focusing energy raises gain and effective range and **reduces interference/noise from off-axis** — ideal for fixed point-to-point or point-to-multipoint backhaul links. External antennas give "more control over the energy radiated" and let you tailor the pattern, vs fixed internal antennas [12]. Practical pattern: **dedicate one radio + directional antenna to backhaul, one radio + omni to local clients** (this is exactly why 2–3 independent radios matter for mesh).

**Power input options** [2, 6, Transtector]:
- **PoE 802.3af (~13 W) / 802.3at (~25 W)** is the cleanest single-cable outdoor option (OpenWrt One supports it on the 2.5 GbE WAN; LibreRouter has PoE + passthrough). Use an outdoor-rated/industrial PoE injector for field installs.
- **12–24/36 V DC barrel** (LibreRouter accepts 12–36 V) for solar/battery sites.
- **USB-C PD** present on OpenWrt One (console + power capable on some revisions) — convenient for bench/indoor, less so outdoors.

**Enclosure / weatherproofing / thermal** [13, 14, TRENDnet]:
- Use a **polycarbonate enclosure rated NEMA 4X/6P or IP66–IP68** (Polycase ML series rated to NEMA 6P/IP68) [13].
- **Thermal caution**: do **not** simply seal an indoor board in a box — "enclosing an indoor router defeats its thermal design, trapping heat and creating condensation." Plan a heat path: metal enclosure/heatsink contact, vents with hydrophobic membranes (Gore vents), or generous internal air volume [14].
- **Conformal coating** the PCB protects against humidity/condensation for outdoor deployment — indoor boards typically lack it [14].
- **Cable entry**: use **IP-rated glands** for power/Ethernet; SMA/RP-SMA bulkheads with O-rings at the wall; add **gas-discharge/PoE surge protection** for outdoor Ethernet runs.

### Antenna / RF / Packaging checklist (actionable)
1. Plan connector count = (streams per radio) × (number of radios). Order that many bulkhead pigtails.
2. Internal jacks are U.FL/IPEX → use **U.FL-to-RP-SMA pigtails to panel-mount bulkheads**; never expose U.FL to repeated handling or weather.
3. Prefer **RP-SMA/SMA at the enclosure wall** (~500 cycles, threaded, weatherproof with O-ring); MMCX acceptable if board already uses it.
4. **Backhaul radio → directional/sector (12–20+ dBi); client radio → omni (2–8 dBi).** Keep backhaul on a separate radio/channel from client.
5. Power: PoE 802.3at as default outdoor single-cable; 12–24 V DC for solar; add **Ethernet surge protector** on outdoor runs.
6. Enclosure: polycarbonate **IP66–IP68 / NEMA 4X**; add **Gore vent** for pressure/condensation; ensure a **conductive thermal path** for the SoC.
7. **Conformal-coat** the PCB for outdoor humidity; use **IP-rated cable glands** for all penetrations.
8. Keep antenna pigtails short (RG178/RG316) to minimize loss; mount bulkheads away from metal that detunes the antenna.

## Confidence
**Level**: medium-high

Part A board facts (OpenWrt One MT7981/mt76, schematics at one.openwrt.org; LibreRouter QCA9558/AR9582 3-radio, EAGLE files) are corroborated by primary/official sources (OpenWrt wiki, Banana Pi docs, CNX Software). Part B is high-confidence (multiple independent vendor/engineering sources agree on mating-cycle numbers and antenna gain ranges). Confidence is held back from "high" because (a) OpenWrt One pricing and exact hardware *license* (CERN-OHL vs "schematics published") were not directly confirmed this pass, and (b) LibreRouter current purchasability/production status in 2026 is uncertain.

## Sources
- [1] **url**: https://forum.banana-pi.org/t/openwrt-one-opensource-wifi6-router-with-mediatek-mt7981b-chip/18168 — "OpenWrt One/AP-24.XY board based on MT7981B (Filogic 820) SoC and MT7976C dual-band WiFi 6 chipset; first official OpenWrt community dev board."
- [2] **url**: https://openwrt.org/toh/openwrt/one — "Filogic 820; WiFi 6 dual-band 3×3/2×2 detachable antennas; 1GB DDR4; 256MiB NAND+16MiB NOR; M.2 SSD; 1×2.5GbE WAN+1×1GbE LAN; USB-C console; 802.3af/at PoE via WAN; schematics/datasheets at one.openwrt.org/hardware/."
- [3] **url**: https://one.openwrt.org/hardware/MT7981B_Wi-Fi6_Platform_Datasheet_Open_V1.0.pdf — open MT7981B Wi-Fi 6 platform datasheet (AX3000), confirms published open hardware docs.
- [4] **url**: https://www.amazon.com/Banana-OpenWrt-One-Router-Development/dp/B0GTTNZPG2 — "Official OpenWrt Community Edition – first hardware co-designed with the OpenWrt open-source project; MT7981B; 2.5GbE; M.2 NVMe; unbrickable dual flash."
- [5] **url**: https://docs.banana-pi.org/en/OpenWRT-One/BananaPi_OpenWRT-One — "OpenWrt One based on MT7981B (Filogic 820) SoC and MT7976C dual-band WiFi 6 chipset."
- [6] **url**: https://www.cnx-software.com/2020/01/29/librerouter-is-an-open-source-hardware-router-for-community-networks/ — "QCA9558 750MHz; on-chip 2.4GHz 2×2 + two AR9582-based 5GHz 2×2 mPCIe radios (3 radios); 128MB RAM/16MB flash; 2× GbE; PoE + passthrough 12–36V; MEGA board EAGLE schematics on GitHub; OpenWrt + LibreMesh; ENACOM certified 2019."
- [7] **url**: https://www.reddit.com/r/openwrt/comments/1hx9432/first_impressions_of_the_openwrt_one_official/ — "all-metal casing for heat dissipation; three MMCX antenna connectors allow users to extend wireless coverage."
- [8] **url**: https://news.ycombinator.com/item?id=18715230 — community concern that LibreRouter is not easily purchasable ("no buy-it-now button"), availability constraint.
- [9] **url**: https://www.scribd.com/document/488490194/LibreRouter-Specifications-v7-5 — "LibreRouter LR1 open-source hardware WiFi router with 3 radios (2.4/5/5 GHz) and 2 Ethernet ports; LibreMesh firmware; weatherproof enclosure."
- [10] **url**: https://www.data-alliance.net/blog/antenna-jacks-ufl-mhf4-rpsma-sma-add-upgrade-antenna-to-vastly-increase-range — "U.FL/MHF4 ~30 mating cycles, fragile; MMCX ~500; SMA/RP-SMA ~500, robust indoor/outdoor; use U.FL-to-SMA/RP-SMA pigtail to isolate fragile jack."
- [11] **url**: https://www.bbtantennas.com/article/omni-vs-directional-antenna-which-one-actually-works-better-in-real-projects-omni-fiberglass-antennas.html — "directional antennas achieve higher gain (sector 12–18 dBi, dish 20+ dBi) and greater range than omni."
- [12] **url**: https://www.ekahau.com/blog/when-to-use-a-wi-fi-access-point-with-an-external-antenna — "external antennas give more control over radiated energy; tailor coverage shape vs fixed internal antennas."
- [13] **url**: https://www.polycase.com/techtalk/outdoor-electronic-enclosures/best-options-for-outdoor-network-enclosures.html — "ML series polycarbonate enclosures rated to NEMA 6P/IP68, weatherproof/waterproof."
- [14] **url**: https://www.alibaba.com/product-insights/how-to-choose-the-best-outdoor-router-for-reliable-weatherproof-connectivity.html — "enclosing an indoor router defeats its thermal design—trapping heat and condensation; indoor units lack conformal coating."
- [15] **url**: https://github.com/OLIMEX/OLINUXINO (and OLIMEX/DIY-LAPTOP/HARDWARE) — "OLINUXINO is Open Source / Open Hardware; KiCad design files under GPLv3+." Confirmed OSHW vendor, but no matching multi-radio WiFi router board.

Supporting/secondary URLs consulted:
- https://www.fs.com/blog/directional-vs-omnidirectional-antennas-for-outdoor-wifi-b47414.html — omni vs directional coverage comparison.
- https://oscarliang.com/fpv-antenna-connectors/ — "U.FL connectors are way more fragile than standard SMA/RP-SMA and have much fewer mating cycles (30+)."
- https://tejte.com/blog/ufl-to-sma-adapter-cable-guide/ — U.FL-to-SMA pigtail selection guide.
- https://www.transtector.com/poeod1gat-tt — outdoor IP67 802.3at PoE injector / surge protection.
- https://libremesh.org/ and https://github.com/freifunk-gluon/gluon — OpenWrt-based mesh firmware frameworks.

## Open Questions
- **OpenWrt One hardware license**: confirmed schematics+datasheets are *published*, but I did not verify the exact license (CERN-OHL? OSHWA-certified?) or whether full Gerbers/CAD (not just PDFs) are released. Matters for the user's "genuinely open" bar.
- **OpenWrt One radio count for mesh**: it has one WiFi chip (dual-band concurrent), not two independent radios on separate channels — need to confirm whether that satisfies the project's "≥2 radios" requirement, or whether a second radio (M.2/mPCIe or USB) is needed for dedicated backhaul + client.
- **LibreRouter 2026 purchasability and price**: production has historically been intermittent; current lead time, price, and whether a successor/v2 exists is unconfirmed.
- **Banana Pi openness depth**: unclear whether BPI-R3/R4 release complete, licensed schematics + Gerbers, or only docs/firmware + vendor-dependent driver blobs — directly affects whether they clear the "open hardware" bar.
- **OpenWrt One pricing** not confirmed from a primary OpenWrt/SPI source this pass (commonly cited ~US$89).

## Sub-Hypotheses (if any)
DEPTH_REMAINING is 1, so I note rather than spawn:
- [openwrt-one-second-radio]: Can the OpenWrt One gain a true second independent radio (M.2 WiFi card or mPCIe/USB mt76/ath card) to meet a dedicated-backhaul mesh topology? — cannot resolve from current findings because I did not confirm whether the M.2 slot is PCIe-capable for a WiFi NIC vs SSD-only.
- [openwrt-one-hw-license]: Exact open-hardware license and completeness of OpenWrt One design files (Gerbers/CAD vs PDF schematics, OSHWA cert) — unresolved because one.openwrt.org/hardware/ contents were not enumerated this pass.