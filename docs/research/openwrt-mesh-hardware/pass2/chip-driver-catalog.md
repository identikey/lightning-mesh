# OpenWrt WiFi Chip × Driver × 802.11s Mesh Catalog (Pass 2)

**Research date: 2026-06-24.** Sources favor 2023–2026; foundational kernel-doc / driver-source / forum references included where load-bearing.

## Purpose & context

Goal: build a **true 802.11s mesh** on **OpenWrt** (mainline drivers only). 802.11s mesh-point support is **MANDATORY**; WiFi 6 (802.11ax) ideal; driver quality matters. The user previously **failed to get working mesh on the Qualcomm `wifi-qcom`/`ath11k` closed stack** — this catalog explains precisely why, and what to buy instead.

### The single decisive test

`iw list` → "Supported interface modes" reflects the driver's `wiphy->interface_modes` bitmap. **mac80211 rejects any vif type not in that bitmap.** So "does this chip do 802.11s?" reduces to: *does the mainline driver register `NL80211_IFTYPE_MESH_POINT`?* This was verified by reading driver source directly for every family below, then corroborated with real-world `iw list` dumps from forums.

> **Caveat that bit the user:** advertising `mesh point` in `iw list` is *necessary but not sufficient*. ath11k advertises mesh point yet has real-world data-path / band-downgrade bugs (see §Qualcomm). "Hard and flaky" is not the same as "unsupported."

---

## Chip × Driver × 802.11s Matrix

| Chip | Vendor | Driver | WiFi gen | Bands (2.4/5/6) | Max SS | **802.11s mesh point** | IBSS | OpenWrt mainline | One-line verdict |
|---|---|---|---|---|---|---|---|---|---|
| AR9280 | Qualcomm Atheros | ath9k | 11n (W4) | 2.4 **or** 5 | 2x2 | **YES** | YES | Mature mainline | Rock-solid mesh, but 11n only |
| AR9380 | Qualcomm Atheros | ath9k | 11n (W4) | 2.4/5 | 3x3 | **YES** | YES | Mature mainline | The 802.11s reference platform |
| AR9580 | Qualcomm Atheros | ath9k | 11n (W4) | 2.4/5 | 3x3 | **YES** | YES | Mature mainline | Reference-grade mesh, dated speed |
| QCA988x | Qualcomm Atheros | ath10k | 11ac (W5) | 2.4/5 | 2x2/3x3 | **CONDITIONAL** (stock fw only) | cond. | Mainline (CT default) | Mesh only on non-CT firmware |
| QCA9984 | Qualcomm Atheros | ath10k | 11ac (W5) | 2.4/5 | 4x4 | **CONDITIONAL** (stock fw only) | cond. | Mainline (CT default) | Best ath10k mesh, swap off CT fw |
| QCA9888 | Qualcomm Atheros | ath10k | 11ac (W5) | 5 (often) | 2x2 | **CONDITIONAL** (stock fw only) | cond. | Mainline (CT default) | Discrete 5 GHz radio, non-CT for mesh |
| IPQ8074 | Qualcomm | ath11k / wifi-qcom | 11ax (W6) | 2.4/5 | 4x4 | **CONDITIONAL / FRAGILE** | no | Mainline (qualcommax) | Advertises mesh; ax link silently drops to VHT80 (bug #19805) |
| IPQ6018 | Qualcomm | ath11k / wifi-qcom | 11ax (W6) | 2.4/5 | 2x2 | **CONDITIONAL / FRAGILE** | no | Mainline (ipq60xx) | Same ath11k mesh fragility; this is what bit the user |
| QCN9074 | Qualcomm | ath11k | 11ax (W6E) | 5/6 | 2x2/4x4 | **CONDITIONAL / FRAGILE** | no | Mainline (discrete) | 6 GHz add-in card; closed fw, no swap escape |
| IPQ5018 | Qualcomm | ath11k | 11ax (W6) | 2.4/5 | 2x2 | **CONDITIONAL / FRAGILE** | no | Mainline (budget) | Budget IPQ; same ath11k caveats |
| IPQ5332/QCN9274/WCN7850 | Qualcomm | ath12k | 11be (W7) | 2.4/5/6 | 2x2 | **YES (experimental)** | no | Snapshot only (since ~Sep 2024) | Mesh advertised w/ HE+EHT; bleeding-edge, no MLO |
| mt7915 | MediaTek | mt76 | 11ax (W6) | 2.4/5 | 4x4 | **YES** | YES | Mainline | Mesh works; most-documented mesh bugs |
| mt7916 | MediaTek | mt76 | 11ax (W6E) | 2.4/5/6 | 2x2 / 3x3:2 | **YES** (6 GHz build-dependent) | YES | Mainline | Mesh incl. 6 GHz where build exposes it |
| mt7921 | MediaTek | mt76 | 11ax (W6) | 2.4/5 | 2x2 | **NO** | **NO** | Mainline | Client silicon — no mesh in hardware |
| mt7922 | MediaTek | mt76 | 11ax (W6E) | 2.4/5/6 | 2x2 | **NO** | **NO** | Mainline | Client silicon (= AMD RZ616) — no mesh |
| mt7925 | MediaTek | mt76 | 11be (W7) | 2.4/5/6 | 2x2 | **NO** | **NO** | Mainline | WiFi 7 client — still no mesh |
| mt7981 (+mt7976) | MediaTek | mt76 | 11ax (W6) | 2.4/5 | 2x2(–4x4) | **YES** | YES | Mainline | Confirmed working mesh (AX3000T pair) |
| mt7986 | MediaTek | mt76 | 11ax (W6) | 2.4/5 | 4x4 | **YES** (2.4/5); 6 GHz impossible | YES | Mainline | Strong mesh AP silicon; "6E" is marketing |
| mt7988 (WiFi via mt7996) | MediaTek | mt76 (mt7996e) | 11be (W7) | 2.4/5/6 | 4x4 | **CONDITIONAL / unverified** | decl. | Mainline (immature) | SoC has NO WiFi; uses mt7996 card; driver immature |
| mt7990 | MediaTek | mt76 (mt7996e) | 11be (W7) | 2.4/5 (no 6) | 2x3:3 | **CONDITIONAL YES** (immature) | YES | Snapshot/main | Mesh declared; throughput-degradation bug #1065 |
| RTL8821CE | Realtek | rtw88 | 11ac (W5) | 2.4/5 | 1x1 | **NO** | YES | Mainline | No mesh; weak AP — avoid |
| RTL8822BE | Realtek | rtw88 | 11ac (W5) | 2.4/5 | 2x2 | **NO** | YES | Mainline | No mesh; weak AP — avoid |
| RTL8822CE | Realtek | rtw88 | 11ac (W5) | 2.4/5 | 2x2 | **NO** | YES | Mainline | No mesh; weak AP — avoid |
| RTL8852AE | Realtek | rtw89 | 11ax (W6) | 2.4/5 | 2x2 | **NO** | **NO** | Mainline (5.18+) | No mesh, no IBSS — avoid |
| RTL8852BE | Realtek | rtw89 | 11ax (W6) | 2.4/5 | 2x2 | **NO** | **NO** | Mainline (6.2+) | No mesh; reboots on OpenWrt (#17025) |
| RTL8852CE | Realtek | rtw89 | 11ax (W6E) | 2.4/5/6 | 2x2 | **NO** | **NO** | Mainline (5.19+) | No mesh; AP malfunctions under load |
| AX200 | Intel | iwlwifi | 11ax (W6) | 2.4/5 | 2x2 | **NO** | YES | Mainline | Client-only; AP crippled to 2.4 GHz |
| AX210 | Intel | iwlwifi | 11ax (W6E) | 2.4/5/6 | 2x2 | **NO** | YES | Mainline | Client-only; mesh "Operation not supported" |
| AX211 | Intel | iwlwifi | 11ax (W6E) | 2.4/5/6 | 2x2 | **NO** | YES | Mainline (CNVio2) | No mesh; CNVio2 won't fit router M.2-E |
| BE200 | Intel | iwlwifi (mld) | 11be (W7) | 2.4/5/6 | 2x2 | **NO** | YES | Mainline (CNVio2) | WiFi 7 client-only; no mesh; CNVio2-bound |
| BCM43xx (FullMAC) | Broadcom | brcmfmac | varies | varies | varies | **NO** (-EOPNOTSUPP) | yes (buggy) | Host driver open, fw closed | Mesh explicitly rejected; avoid |
| BCM47xx/53xx (legacy) | Broadcom | b43 / brcmsmac | 11g / 11n | 2.4(/5) | 1–2 | **NO** | limited | Open (reverse-eng) | Basic AP/STA only; no mesh |
| Broadcom STA (wl) | Broadcom | broadcom-sta (wl) | varies | varies | varies | **NO** | broken | Proprietary blob, abandoned | Closed, unmaintained — never for mesh |

**Legend:** SS = spatial streams. "CONDITIONAL" = mesh point is advertised/works only under a specific firmware or has a documented degradation. "decl." = declared in driver but unverified on real hardware.

---

# Per-vendor evidence

## MediaTek — mt76 (the recommended family)

**Decisive driver source (github.com/openwrt/mt76, master):**

- `mt7915/init.c` — sub-driver for **mt7915, mt7916, mt7981, mt7986** — registers `BIT(NL80211_IFTYPE_ADHOC)` and, under `#ifdef CONFIG_MAC80211_MESH`, `BIT(NL80211_IFTYPE_MESH_POINT)`, combo `#{ AP, mesh point } <= 16`. → **mesh point + IBSS advertised.**
- `mt7996/init.c` + `mt7996/main.c` — sub-driver for **mt7996, mt7992, mt7990 (WiFi 7)** — declares `ADHOC` unconditionally and `MESH_POINT` under `CONFIG_MAC80211_MESH`. → **mesh point + IBSS advertised.**
- `mt792x_core.c` — shared core for **mt7921, mt7922, mt7925** — iface_limits register ONLY `STATION`, `AP`, P2P types. **No `MESH_POINT`, no `ADHOC`.** The `MESH_POINT` strings in mt7921/mt7925 `main.c` are defensive `-EOPNOTSUPP` returns. → **client chips: no mesh, no IBSS.**

### Per-chip notes

- **mt7915** — WiFi 6, 2.4/5, up to 4x4. Mesh **YES** but the most bug-reported mesh target (see bug list). `kmod-mt7915e` + `kmod-mt7915-firmware`.
- **mt7916** — WiFi 6E, 2.4/5/**6** capable (EEPROM v2 has 6 GHz offsets; DBDC: 2.4 GHz 2x2 + 5/6 GHz 2T3R, 5 and 6 are alternatives on the 2nd radio). Mesh **YES**, including 6 GHz where build/regdomain expose it. Caveat: several OpenWrt 24.x builds drop 6 GHz entirely (openwrt#17632). `kmod-mt7915e` + `kmod-mt7916-firmware`.
- **mt7921 / mt7922 / mt7925** — **NO mesh, NO IBSS.** Hardware limitation (no per-STA RX GTK). Maintainer nbd168: "MT7921 does not support mesh mode." mt7922 runs under the mt7921e path (+`kmod-mt7922-firmware`). mt7925 (WiFi 7 client) still lacks mesh — a raw `iw list` dump shows managed/AP/AP-VLAN/monitor/P2P only.
- **mt7981 (+mt7976 RF)** — WiFi 6 (NOT 6E), 2.4/5 only, products typically 2x2/2x2 DBDC. Mesh **YES** — confirmed working between two Xiaomi AX3000T (openwrt#22308). Bug: 802.11r + mesh on kernel 6.12 kills the 5 GHz phy; workaround = disable 802.11r. `kmod-mt7915e` + `kmod-mt7981-firmware`.
- **mt7986** — WiFi 6 (**NOT 6E** — kernel doc classifies "2.4/5 GHz"; **no 6 GHz radio physically exists**), 2.4/5, 4T4R. Mesh **YES** on 2.4/5 — real SXK80 `iw list` shows "mesh point"; confirmed working between two BPI-R3 with SAE. **6 GHz mesh = impossible (no radio).** `kmod-mt7915e` + `kmod-mt7986-firmware`.
- **mt7988** — router SoC with **NO integrated WiFi**; WiFi on BPI-R4 comes from a separate **mt7996-family PCIe card** → uses `kmod-mt7996e` + `kmod-mt7996-firmware`. Mesh declared but unverified; hw-semaphore lockups / "wifi unusable" reports on BPI-R4. **CONDITIONAL/unverified.**
- **mt7990** — WiFi 7 (BE3600), DBDC 2.4/5 only (**no 6 GHz** — not tri-band), 2x3:3. Uses `mt7996e` sub-driver + `kmod-mt7990-firmware`. Merged into Linux 6.16 (March 2025); OpenWrt snapshot/main only. Mesh declared (if `CONFIG_MAC80211_MESH`), but bug #1065 (Mar 2026): progressive throughput degradation under multi-client load, reboot-only recovery — "not production-ready for sustained multi-client deployments."

### Known mt76 mesh bugs

mt76#259 (nodes not meshing, 2019); mt76#622 (mt7915E/RT3200 mesh unstable, 2021); mt76#707 (mesh kernel warnings, 2022); mt76#1065 (mt7990 degradation, Mar 2026); openwrt#12905 (23.05 mesh throughput/association regression on mt7986/mt7915); openwrt#13153 (mesh locked to 20 MHz with 3+ VAPs); openwrt#13880 (mesh capped 80 MHz vs 160); openwrt#22308 (mt7981 5 GHz phy disappears with 802.11r+mesh).

### Sources (MediaTek)

- https://github.com/openwrt/mt76/blob/master/mt7915/init.c — mesh+IBSS for mt7915/7916/7981/7986
- https://github.com/openwrt/mt76/blob/master/mt792x_core.c — NO mesh/IBSS for mt7921/7922/7925
- https://github.com/openwrt/mt76/blob/master/mt7996/main.c · https://raw.githubusercontent.com/openwrt/mt76/master/mt7996/init.c — mesh+IBSS for mt7996/7992/7990
- https://github.com/openwrt/openwrt/blob/main/package/kernel/mt76/Makefile — kmod package names
- https://wireless.docs.kernel.org/en/latest/en/users/drivers/mediatek.html — band classification (mt7986/7981 = 2.4/5 only)
- https://patches.linaro.org/project/linux-wireless/cover/20250329154731.2113551-1-shayne.chen@mediatek.com/ — mt7990 upstream (Linux 6.16, dual-band)
- https://github.com/openwrt/mt76/issues/653 — "MT7921 does not support mesh mode"
- https://github.com/openwrt/openwrt/issues/22308 — mt7981 AX3000T mesh works; 802.11r+mesh phy bug
- https://github.com/openwrt/openwrt/issues/17632 — mt7916 6 GHz missing in 24.x builds
- https://forum.openwrt.org/t/what-is-my-router-capable-of-sxk80-based-on-iw-list/244241 — mt7986 "mesh point" iw list dump
- https://forum.banana-pi.org/t/bpi-r3-mesh-802-11s-didn-t-get-it-to-work-solved/14056 — mt7986 working mesh (SAE)
- https://github.com/morrownr/USB-WiFi/blob/main/home/iw_list/MediaTek_MT7925_m.2.txt — mt7925 no-mesh dump
- https://github.com/openwrt/mt76/issues/259 · /622 · /707 · /1065 — mesh bugs
- https://github.com/openwrt/openwrt/issues/12905 · /13153 · /13880 · /20188 — mesh regressions / mt7988

---

## Qualcomm / Atheros — ath9k, ath10k, ath11k/wifi-qcom, ath12k

### ath9k — AR9xxx (802.11n) — THE MESH REFERENCE

- AR9280 (2x2 single-band), AR9380 / AR9580 (3x3 dual-band). All support mesh point; this is the de-facto 802.11s reference. Install `wpad-mesh-wolfssl`/`-openssl`; some boards need `nohwcrypt=1` for stability. Known wrinkle: encryption on HT40 can collapse throughput (config/channel issue, not hardware).
- **Verdict: YES.** Most mature mesh driver in OpenWrt. Limitation: 802.11n speeds only.
- Evidence: https://wireless.docs.kernel.org/en/latest/en/users/drivers/ath9k.html · https://forum.openwrt.org/t/802-11s-ath9k-slow-speeds-on-ht40-ath10k-doesnt-support-mesh-above-vht20-and-no-encryption-support/2978

### ath10k — QCA988x / QCA9984 / QCA9888 (802.11ac) — THE CT FIRMWARE CAVEAT

**>>> The single most important ath10k fact: CT vs non-CT firmware <<<**

- OpenWrt ships **ath10k-ct (Candela Technologies)** driver + firmware **by default**.
- **CT firmware does NOT support 802.11s mesh, and does NOT support IBSS.** Maintainer Ben Greear has put CT firmware in maintenance-only mode ("not planning to add any new features").
- Symptom on CT: `must load driver with rawmode=1 to add mesh interfaces`; setting `rawmode=1` then fails with `rawmode = 1 requires support from firmware` / `fatal problem with firmware features: -22` (CT firmware lacks the `raw-mode` feature flag).
- The OpenWrt wiki warns (Dec 2023) that `ath10k-ct` "doesn't support mesh very well, resulting in errors and random dropping of wireless interfaces."

**The fix (required for mesh):** swap CT → stock packages:
- Remove `kmod-ath10k-ct`, `ath10k-firmware-<chip>-ct`
- Install `kmod-ath10k`, `ath10k-firmware-<chip>` (and `iw-full`)
- **Stock/upstream firmware** carries the `raw-mode` flag (MBSS raw-mode since Sep 2015) needed for mesh; the `mfp-support` flag is additionally required for *encrypted* (SAE) mesh, which uses the wpa_supplicant path.

**Nuance (verified independently):** the CT-vs-stock answer is not absolute — it is firmware-version- and scenario-dependent. One forum case (QCA988x on UniFi AC Mesh, May 2020) found the *opposite*: 5 GHz mesh failed on every ath10k-ct variant and only worked after switching to **stock ath10k**. Net: for mesh on ath10k, **use stock/non-CT firmware** and expect possible historical VHT-channel-width fragility (a 2017 QCA9882 report meshed only at VHT20, not VHT40/80).

- **Verdict: CONDITIONAL — NO on CT (OpenWrt default), YES on stock/non-CT firmware.**
- Evidence: https://github.com/openwrt/openwrt/issues/14089 (CT lacks mesh+IBSS; default is CT) · https://forum.openwrt.org/t/changing-from-ath10k-ct-firmware-to-classic-because-must-load-driver-with-rawmode-1-to-add-mesh-interfaces/192256 (rawmode error + swap, Mar 2024) · https://github.com/greearb/ath10k-ct/issues/81 (rawmode "-22") · https://wireless.docs.kernel.org/en/latest/en/users/drivers/ath10k/mesh.html (raw-mode + mfp-support flags) · https://forum.openwrt.org/t/ath10k-ct-with-802-11s/64932 (stock works where CT failed, May 2020) · https://github.com/freifunk-berlin/firmware/issues/696 (CT mesh+AP-station parallel fails)

### ath11k / wifi-qcom — IPQ6018 / IPQ8074 / QCN9074 / IPQ5018 / IPQ53xx (802.11ax) — WHY THE USER FAILED

**>>> The precise current (2025–2026) ath11k mesh limitation <<<**

**Key correction:** ath11k does **NOT** lack mesh point. Mesh point IS implemented and **`iw list` DOES report "mesh point"** on IPQ8074/IPQ6018. HE-mesh support (Sven Eckelmann / Open-Mesh) merged into mainline around when ath11k entered the kernel (v5.6, Mar 2020), including workarounds disabling HE SU PHY caps for buggy firmware. So `iw list | grep mesh` showing "mesh point" is *expected*, not proof of success.

**The real failure modes are bugs/degradation, not absence:**

1. **AX→VHT80 mesh downgrade (the big one) — OpenWrt #19805, OPEN as of Aug 2025** (24.10.2 + SNAPSHOT). On 802.11ax radios, AP interfaces correctly use HE80 but **mesh interfaces silently downgrade to VHT80** (i.e. run as 802.11ac, not ax). Root cause is OpenWrt's `hostapd.sh` / `wpa_supplicant_set_fixed_freq()`; EHT (WiFi 7) handling is also missing there. Tested on Xiaomi AX3600 (IPQ8074) and BPI-R4.
2. **Multicast / data-path issues** — early IPQ8074 reports: "802.11s doesn't support multicast traffic (and/or igmp snooping)"; users worked around by switching to WDS (`frame_mode=1`). Mesh associates but data path can misbehave.
3. **General instability** — STA-mode driver crashes on IPQ807x, board-ID/QMI detection problems, VHT mis-advertisement causing crashes above HT20 on early kernels. Functional but fragile vs ath9k/mt76.
4. **Closed firmware, no escape hatch** — wifi-qcom/ath11k uses proprietary firmware loaded via QMI (board-2.bin). Unlike ath10k there is **no CT-style alternative firmware to swap to** if mesh behaves badly.
5. **Encrypted (SAE) mesh** on ax/6 GHz combines #1's config-script gaps with the wpa_supplicant path — the fragile zone where setups commonly fail.

**Most probable cause of the user's failure:** (a) the HE→VHT80 mesh downgrade bug (#19805) producing mismatched/degraded links on ax radios, and/or (b) hostapd/wpa_supplicant fixed-freq script gaps on ax/6 GHz, and/or (c) driver instability — **not** a missing capability. Actionable check: re-capture `iw list | grep -A12 "Supported interface modes"` on the failing device; mainline ath11k should list "mesh point." If it does, it's the config/bug path, and unlike ath10k there's no firmware swap to route around it.

- **Verdict: CONDITIONAL / FRAGILE — mesh point advertised, real-world ax mesh buggy/degraded; closed firmware blocks workarounds. "Hard and flaky," not cleanly "unsupported."**
- Evidence: https://github.com/openwrt/openwrt/issues/19805 (ax/EHT mesh → VHT80, OPEN Aug 2025) · https://forum.openwrt.org/t/802-11s-mesh-and-igmp-snooping/131915 (multicast/IGMP; iw list shows mesh point; WDS workaround) · https://patchwork.kernel.org/project/linux-wireless/cover/20190724163359.3507-1-sven@narfation.org/ (ath11k HE mesh / MESH_POINT merged) · https://wireless.docs.kernel.org/en/latest/en/users/drivers/ath11k.html · https://github.com/openwrt/openwrt/issues/20702 (ath11k STA crash IPQ807x) · http://lists.infradead.org/pipermail/ath11k/2020-December/000818.html · https://deepwiki.com/openwrt/firmware_qca-wireless/2.1-driver-ecosystems:-ath10k-vs-ath11k

### ath12k — WiFi 7 (IPQ5332 / QCN9274 / WCN7850)

- In OpenWrt snapshots (WCN7850 since ~Sep 2024), `iw list` reports **mesh point with HE+EHT caps** across bands. 6 GHz @160 MHz confirmed on WCN7850. Caveats: **MLO not in vanilla**; QCN9274 has no open firmware; expect the same OpenWrt fixed-freq config gaps (#19805 notes missing EHT handling). Experimental.
- **Verdict: YES (advertised/reported) but BLEEDING-EDGE.**
- Evidence: https://forum.openwrt.org/t/ath12k-qualcomm-wifi7/212355 · https://github.com/openwrt/openwrt/pull/15945 · https://github.com/openwrt/openwrt/issues/19805

---

## Realtek — rtw88 (WiFi 5) and rtw89 (WiFi 6/6E)

**Decisive driver source:**

- `realtek/rtw88/main.c`: `interface_modes = STATION | AP | ADHOC`. **No MESH_POINT** (IBSS present). Covers RTL8821CE (1x1), RTL8822BE/CE (2x2). All WiFi 5, 2.4/5 only.
- `realtek/rtw89/core.c`: `interface_modes = STATION | AP | P2P_CLIENT | P2P_GO`. **No MESH_POINT, no ADHOC.** (Trap: `core.c` has dead `RTW89_TYPE_MAPPING(MESH_POINT)` / ADHOC mapper code, never advertised.) Covers RTL8852AE/BE (2.4/5), RTL8852CE (2.4/5/**6**, WiFi 6E).

- **Real-world confirm:** OpenWrt forum `iw list` for RTL8852AE (kernel 6.6) shows managed/AP/AP-VLAN/monitor/P2P only — no mesh, no IBSS.
- **Driver quality:** rtw88 STA reliable, AP/IBSS weak ("RTL in AP mode is usually a no-go"). rtw89 mediocre/immature — firmware H2C hangs, 8852BE reboots on OpenWrt (#17025), 8852CE AP malfunction under load.
- **Verdict: NO mesh on any Realtek chip here. Unsuitable for a mesh build.**
- Evidence: https://raw.githubusercontent.com/torvalds/linux/master/drivers/net/wireless/realtek/rtw88/main.c · https://raw.githubusercontent.com/torvalds/linux/master/drivers/net/wireless/realtek/rtw89/core.c · https://forum.openwrt.org/t/generic-x86-64-24-10-3-rtl8852ae-rtw89-firmware-loads-but-no-wireless-interface/241369 · https://cateee.net/lkddb/web-lkddb/RTW89.html

---

## Intel — iwlwifi (AX200 / AX210 / AX211 / BE200)

**Decisive driver source:** `intel/iwlwifi/mvm/mac80211.c` (AX200/210/211) and `iwlwifi/mld/mac80211.c` (BE200, WiFi 7) both register `STATION | P2P_CLIENT | AP | P2P_GO | P2P_DEVICE | ADHOC`. **No MESH_POINT anywhere** (grep for `IFTYPE_MESH` returns nothing). IBSS present (incl. IBSS RSN).

- **Confirm on the key question:** mesh point is **NOT** in any Intel card's supported interface modes. A chip-identified AX210 `iw list` dump shows exactly 8 modes (IBSS, managed, AP, AP/VLAN, monitor, P2P-client, P2P-GO, P2P-device) — no mesh. `iw ... type mp` returns "Operation not supported."
- **Why client-oriented:** Intel's official stance (KB 000030429): all adapters are station/client. Where AP is advertised, firmware restricts it to **2.4 GHz only** (LAR blocks 5/6 GHz AP). AX211/BE200 are **CNVio2-only** — they will not work in standard PCIe/M.2-E router hardware regardless.
- **Verdict: NO mesh. Excellent client driver, architecturally client-only. Avoid for infrastructure/mesh roles.**
- Evidence: https://github.com/torvalds/linux/blob/master/drivers/net/wireless/intel/iwlwifi/mvm/mac80211.c · https://github.com/torvalds/linux/blob/master/drivers/net/wireless/intel/iwlwifi/mld/mac80211.c · https://bbs.archlinux.org/viewtopic.php?id=281697 (AX210 iw list) · https://wireless.docs.kernel.org/en/latest/en/users/drivers/iwlwifi.html · https://www.intel.com/content/www/us/en/support/articles/000030429/wireless.html

---

## Broadcom — brcmfmac / b43 / brcmsmac / wl (closed & unsuitable)

**Decisive driver source:** `broadcom/brcm80211/brcmfmac/cfg80211.c` registers `STATION | ADHOC | AP` (+monitor/p2p conditionally), **no MESH_POINT** — and the vif-add switch explicitly returns `case NL80211_IFTYPE_MESH_POINT: return ERR_PTR(-EOPNOTSUPP);`.

- **brcmfmac (FullMAC):** host driver open but **MAC/firmware closed** (where mesh would live). No mesh; AP/IBSS exist but are buggy (STA+AP firmware crashes). Real-world: Raspberry Pi BCM43455/43430 `iw list` lacks mesh point; `iw mesh join` fails `-95`.
- **b43 / brcmsmac (SoftMAC):** open (reverse-engineered), basic 11g / 11n-no-40MHz only, no usable mesh.
- **wl / broadcom-sta:** proprietary binary blob, abandoned (~2010), no AP, broken IBSS — not viable.
- **Verdict: NO mesh in any scenario. OpenWrt community position: "Broadcom's wifis should be avoided."**
- Evidence: https://wireless.docs.kernel.org/en/latest/en/users/drivers/brcm80211.html · https://forums.raspberrypi.com/viewtopic.php?t=145831 (BCM43455 iw list, no mesh) · https://github.com/lll-project/broadcom-sta/blob/master/README.rst · https://wiki.debian.org/wl

---

# Which chips to BUY / AVOID for 802.11s

## BUY — reliable mesh, in priority order

1. **MediaTek mt7986 (WiFi 6, 4x4, 2.4/5)** — best blend of modern speed + proven 802.11s. Confirmed working SAE mesh (BPI-R3 pairs). Mainline mt76. **Top pick for a WiFi-6 802.11s mesh.** (No 6 GHz — "6E" branding is marketing.)
2. **MediaTek mt7981+mt7976 (WiFi 6, 2.4/5)** — cheap, widely available (Xiaomi AX3000T), confirmed mesh (#22308). Great value mesh node. (Disable 802.11r if on kernel 6.12 to avoid the 5 GHz phy bug.)
3. **MediaTek mt7916 (WiFi 6E, 2.4/5/6)** — only solid mainline path to **6 GHz mesh**. Verify your build actually exposes 6 GHz (some 24.x builds drop it, #17632).
4. **MediaTek mt7915 (WiFi 6, 4x4, 2.4/5)** — works, but the most bug-reported mt76 mesh target; prefer mt7986/mt7981 if available.
5. **Qualcomm ath9k AR9380/AR9580 (WiFi 4)** — the gold-standard mesh reference if 802.11n speed is acceptable (IoT/control-plane mesh, max reliability).
6. **Qualcomm ath10k QCA9984 (WiFi 5)** — only with **stock/non-CT firmware** (`kmod-ath10k` + `ath10k-firmware-qca9984`, not the `-ct` packages). Acceptable but firmware-fiddly; mt76 is the easier WiFi-5/6 path.

## CONDITIONAL / EXPERIMENTAL — only if you'll validate on-hardware

- **MediaTek mt7990 / mt7996 (mt7988/BPI-R4) (WiFi 7)** — mesh in the driver but immature (degradation bug #1065, lockups). Snapshot-only. Validate before relying on it.
- **Qualcomm ath12k WCN7850/IPQ5332 (WiFi 7)** — mesh advertised in snapshots; no MLO; bleeding-edge.

## AVOID for 802.11s

- **Qualcomm ath11k / wifi-qcom (IPQ6018/IPQ8074/QCN9074/IPQ5018, WiFi 6)** — *this is what bit the user.* Mesh point is advertised but real-world ax mesh degrades to VHT80 (open bug #19805, Aug 2025), has multicast/data-path issues, and the **closed firmware gives no swap escape hatch**. Do not pick ath11k for a dependable 802.11s mesh.
- **MediaTek mt7921 / mt7922 / mt7925** — client silicon; **no mesh, no IBSS in hardware.** No software fix possible.
- **Realtek rtw88 (8821/8822) and rtw89 (8852A/B/C)** — no mesh point at all; rtw89 also no IBSS; weak/unstable AP.
- **Intel iwlwifi (AX200/AX210/AX211/BE200)** — no mesh, client-only, AP crippled to 2.4 GHz, AX211/BE200 are CNVio2 (won't fit router M.2-E).
- **Broadcom (all)** — closed firmware, mesh explicitly rejected (`-EOPNOTSUPP`); avoid entirely.

## One-line recommendation

**For a WiFi-6 802.11s mesh on OpenWrt mainline, standardize on MediaTek mt76 AP silicon — mt7986 (or mt7981 for budget; mt7916 if you specifically need 6 GHz). Avoid the entire Qualcomm ath11k/wifi-qcom path that previously failed, avoid all MediaTek Connac client chips (mt7921/22/25), and avoid Realtek/Intel/Broadcom for mesh roles.**

---

## Appendix: general 802.11s setup notes (all drivers)

- Requires OpenWrt 19.07+ with `wpad-mesh-openssl` (ample flash/RAM) or `wpad-mesh-wolfssl` (constrained). Min ~8 MB flash / 64 MB RAM for easy setup.
- Encrypted mesh = **WPA3-SAE** between routers (uses the wpa_supplicant path; kernel `iw` alone can't do SAE). WPA2-PSK on client-facing APs.
- Quick capability check on any device: `iw list | grep -A12 "Supported interface modes"` — look for `* mesh point`.
- Sources: https://openwrt.org/docs/guide-user/network/wifi/mesh/802-11s · https://bmaupin.github.io/wiki/other/openwrt/openwrt-80211s.html
