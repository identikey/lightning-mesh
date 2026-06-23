# Hypothesis: H4 — Driver/mesh-mode reality check: which chipsets ACTUALLY support 802.11s/IBSS in current OpenWrt, and WiFi-6/6GHz caveats

## Summary
**Supported with high confidence.** In current OpenWrt (23.05 / 24.10 / 25.x), the **mt76 family (mt7915/mt7916/mt7986 Filogic)** and **ath9k** both genuinely support 802.11s mesh point mode and IBSS, including SAE-encrypted mesh via `wpad-mesh-mbedtls`. The historical mt7915 802.11s breakage (mt76 GitHub issues #522/#622/#675) was real but is **closed/fixed as of 2022-2024**. **ath9k remains the flawless reference** for both IBSS and 802.11s. **ath10k works only with caveats** — the OpenWrt wiki explicitly recommends swapping the `-ct` (Candela) driver/firmware for the **non-CT stock** versions to get reliable mesh. The user's prior pain on the **ath11k / wifi-qcom** stack is corroborated: that target is plagued by STA/mode crashes and instability; while ath11k hardware advertises `mesh point` in `iw list`, it is the least battle-tested for mesh and not recommended. **Recommendation: buy mt76 (mt7915/mt7916) or ath9k/ath10k hardware; avoid ath11k/ath12k for mesh.**

## Evidence

### OpenWrt 802.11s Wiki (the authoritative primary source)
The official wiki page is unambiguous on two points the user cares about:
- **Encryption/SAE works on all current drivers with hardware support:** "802.11s works reliably with all current OpenWrt versions, including over the air encryption, assuming that there is hardware/driver support. The package `wpad-mesh-mbedtls` (or equivalent) is required for this." This means encrypted (SAE) mesh is a packaging choice (`wpad-mesh-*`), not a per-driver gap — provided the driver supports `mesh point`.
- **ath10k-ct is explicitly flagged as problematic:** "as of Dec 2023, the `ath10k-ct` wireless driver used in NETGEAR R7800 and other devices with Qualcomm Atheros QCA988x chips doesn't support mesh very well, resulting in errors and random dropping of wireless interfaces... It is recommended that you use [firmware selector] to... remove the -ct module... and replace them with the non-ct versions... to get reliable mesh support."
- **Verification method:** Run `iw list | grep "Supported interface modes" -A 9` and confirm `* mesh point` (and `* IBSS`) appear.
- Note on stock wpad: default `wpad-basic-mbedtls` does NOT include mesh SAE; you need `wpad-mesh-mbedtls` (or full `wpad`) — a forum thread confirms LuCI will offer mesh encryption but silently fail to peer without the mesh-capable wpad.

### mt76 family (mt7915/mt7916/mt7986 Filogic) — historical bugs now fixed
- mt76 GitHub issue **#675 "Mt7915 5ghz 802.11s mesh is not working"** — **closed as completed (Apr 2, 2024)**, with "fixed per this commit `openwrt/openwrt@2126325`" and a user reporting "works now."
- Companion historical reports #522 ("802.11s broken in latest version", 2021) and #622 ("802.11s MESH BROKEN MT7915E (RT3200)", 2021) are from the same early-driver era. The pattern: mt7915 802.11s was broken ~2021-2022, fixed during 2022-2024.
- The Jan 2024 OpenWrt forum interop thread states it directly: "all ath9k (≥ar9160), ath10k (at least wave2...), ath11k, mt76 devices are supposed to work, also in a mesh setup — including interoperability between them. But mt76 has made rather significant improvements since mt7615." This corroborates that current mt76 (mt7915/mt7916) is in good shape.
- **Open caveat (not mesh-specific):** mt76 issue **#912 "serious regression: mt7916 firmware: 5ghz AP not working"** is OPEN (created Sep 2024, updated Feb 2026). This is a firmware regression affecting 5 GHz AP on some mt7916 — worth flagging because it touches the same band you'd use for backhaul. Pin to a known-good firmware/snapshot.

### ath9k — the reference, still flawless
- Multiple sources treat ath9k as the gold standard for IBSS + 802.11s. Academic/Linux-wireless material notes the stock ath9k mac80211 path supports both IBSS and mesh modes natively (older work even had to patch only for handoff features, not basic mesh).
- Forum guidance: ath9k "works well for older Atheros cards and avoids hardware encryption issues on some boards"; some boards benefit from `option nohwcrypt 1` for stability. The tradeoff: ath9k is 802.11n only (2.4/5 GHz, no WiFi-6, no 6 GHz).

### ath10k — works only with the right firmware
- OpenWrt wiki (above) + a guifi.net wiki note: ath10k 802.11s works but the firmware story matters. The community consensus ("ath10k commonly needs non-ct firmware and kmod replacement for reliable 802.11s") is consistent across the wiki and independent guides.
- Practical rule: for ath10k mesh, use **stock `kmod-ath10k` + `ath10k-firmware-<chip>` (non-CT)**, not `ath10k-ct`. Wave-2 chips (QCA9984/QCA9888) are the safer ath10k mesh targets; Wave-1 has "footnotes."

### ath11k / ath12k / wifi-qcom — confirms the user's prior pain
- The hardware advertises mesh: a QCN9074 (ath11k) `iw list` dump shows `* mesh point` among supported modes — so the *capability bit* is present.
- BUT the target is unstable in practice: OpenWrt issue **#20702** "ath11k driver crash with STA (client) mode on MX4300 (ipq807x)" (Nov 2025) — STA association instantly crashes the driver; issue **#19367** ipq6018 5GHz roaming disconnects (2025); issue **#22074** ath11k STA connection problems on 25.12.0-rc4 (Feb 2026). Mesh point relies on the same client/peer code paths that are crashing here.
- Phoronix (Apr 2026) notes Linux networking fixes "addressing performance issues within the Qualcomm Ath11k and Ath12k WiFi drivers that have always [had issues]" — i.e., these drivers remained problematic into 2026.
- The reliable mesh use on qualcommax (ipq807x, MX4300) in the wild is happening on the **QSDK/NSS community build** (codelinaro QSDK), not the mainline ath11k driver — confirming mainline ath11k mesh is not the dependable path. This matches exactly the user's "wifi-qcom / ath11k closed stack lacks working 802.11s/IBSS" experience.

### Cross-cutting mesh gotchas
- **802.11s vs IBSS vs proprietary "mesh":** 802.11s is the open L2 standard (HWMP default). IBSS/ad-hoc is the older modeless peer mode (no auth/AP concept). Vendor "Mesh" (Deco/Orbi/etc.) is proprietary roaming, unrelated and not interoperable — the wiki calls this out explicitly.
- **6 GHz / DFS on backhaul:** Strong community consensus to **avoid DFS and 6 GHz for the mesh backhaul.** 802.11s requires all peers on one fixed channel; DFS radar events force channel changes that break the mesh ("DFS interruptions break mesh links"). 160 MHz is DFS-only in most regions, so it's discouraged with 802.11s. 6 GHz has short range and is "generally not suitable for backhaul." Use a fixed non-DFS 5 GHz channel for backhaul.
- **Mesh + AP on the same radio:** Yes, generally works — "they usually can; they have to share the same channel, and there may be odd hardware/driver limitations" (Reddit/OpenWrt). For best throughput the recommended topology is a tri-band device with a dedicated 5 GHz radio for backhaul + separate radios for client AP. A Feb 2025 forum report ("mesh and AP on same radio stopped working on 24.10.0") shows this combo is fragile across versions — so a dedicated backhaul radio is the safer design.
- **Routing layer (L3 — user's case):** 802.11s gives you an L2 backhaul; HWMP handles L2 mesh path selection. batman-adv adds an L2 routing overlay; the user runs **L3 (babeld/static)** on top, which just rides over the 802.11s L2 link as "a switch" — fully supported per the wiki ("Any Layer 3 infrastructure will work on top of this"). No driver dependency here; the driver only needs to bring up the `mesh point` interface.

## Confidence
**Level**: high

The two recommended families (mt76, ath9k) are corroborated by the official OpenWrt wiki, a closed-with-fix-commit mt76 GitHub issue, and an independent forum interop statement; the ath10k-ct caveat and ath11k instability are each backed by the primary wiki text plus multiple dated 2025-2026 GitHub issues.

## Sources
- [1] **url**: https://openwrt.org/docs/guide-user/network/wifi/mesh/802-11s — Official wiki: SAE/encryption works on all current drivers with `wpad-mesh-mbedtls`; ath10k-ct flagged as unreliable for mesh (Dec 2023), recommends non-CT; `iw list` verification of `mesh point`/`IBSS`. (Scraped 2026-06)
- [2] **url**: https://github.com/openwrt/mt76/issues/675 — "Mt7915 5ghz 802.11s mesh is not working" — closed as completed Apr 2, 2024, "fixed per commit openwrt/openwrt@2126325," user confirms "works now." (Created 2022-05-27)
- [3] **url**: https://github.com/openwrt/mt76/issues/912 — OPEN regression: mt7916 5 GHz AP not working (created 2024-09-04, updated 2026-02-10); flag firmware pinning for mt7916.
- [4] **url**: https://forum.openwrt.org/t/mesh-with-2-different-router-brands/184736 — "all ath9k (≥ar9160), ath10k (wave2), ath11k, mt76... supposed to work, also in a mesh setup, including interoperability... mt76 has made rather significant improvements since mt7615" (Jan 2024).
- [5] **url**: https://github.com/openwrt/openwrt/issues/20702 — ath11k driver crash in STA mode on MX4300/ipq807x (2025-11-08); plus https://github.com/openwrt/openwrt/issues/19367 (ipq6018 5GHz roaming drops, 2025-07-11) and https://github.com/openwrt/openwrt/issues/22074 (ath11k STA broken, 25.12.0-rc4, 2026-02-18).
- [6] **url**: https://www.phoronix.com/news/Linux-7.0-rc7-Networking-Fixes — Linux networking fixes "addressing performance issues within the Qualcomm Ath11k and Ath12k WiFi drivers that have always [had issues]" (2026-04-02).
- [7] **url**: https://forum.openwrt.org/t/qualcommax-nss-build/148529 — Real-world 802.11s mesh on qualcommax (MX4300) runs on QSDK/NSS community build, not mainline ath11k (latest activity 2026-06-17), with reports of backhaul throughput degrading over ~1 week.
- [8] **url**: https://www.reddit.com/r/openwrt/comments/1owrypz/cudy_ax3000_openwrt/ — DFS/160 MHz incompatibility with 802.11s fixed-channel requirement (2025-11-14); + https://forum.openwrt.org/t/mesh-and-ap-on-the-same-radio-stopped-working-on-24-10-0/223821 (mesh+AP same-radio fragility, 2025-02-05).
- [9] **url**: https://deepwiki.com/openwrt/mt76/7-mt7915-device-family — mt76 unified driver covers MT7915/MT7916/MT798x (Filogic) WiFi-6; dual-MCU architecture (2026-04-24 snapshot).
- [10] **url**: https://gitlab.com/guifi-exo/wiki/blob/master/info/atheros-wifi.md — ath10k official driver supports 802.11s mesh mode (corroborating non-CT firmware guidance).
- [11] **url**: https://forum.openwrt.org/t/i-am-somehow-able-to-create-a-802-11s-mesh-with-wpad-basic-mbedtls-according-to-the-wiki-this-shouldnt-work/182317 — `wpad-basic-mbedtls` lacks mesh SAE; `wpad-mesh-mbedtls` required to peer with encryption.

## Driver Support Matrix

| Driver | Chips | 802.11s (mesh point) | IBSS (ad-hoc) | SAE-mesh (encrypted) | WiFi gen | Notes / verdict |
|---|---|---|---|---|---|---|
| **ath9k** | AR9xxx | Yes (reference) | Yes (reference) | Yes (`wpad-mesh-mbedtls`); `nohwcrypt=1` helps some boards | 802.11n | **Safest, flawless** — but 2.4/5 GHz only, no WiFi-6/6 GHz. Best for low-rate reliable backhaul. |
| **mt76 (mt7915/mt7916/mt7986)** | Filogic WiFi-6/6E | Yes (historical bugs fixed 2022-2024) | Yes | Yes (`wpad-mesh-mbedtls`) | WiFi-6 / 6E | **Recommended for WiFi-6.** Use 24.10+; pin firmware (watch open mt7916 5 GHz AP regression #912). 6 GHz mesh possible but avoid for backhaul (range/DFS). |
| **ath10k** | QCA988x / QCA9984 / QCA9888 | Yes, **only with non-CT** firmware/driver | Yes | Yes | 802.11ac (WiFi-5) | Workable: install stock `kmod-ath10k` + non-CT firmware; **avoid `ath10k-ct`** for mesh. Wave-2 safer than Wave-1. |
| **ath11k / ath12k / wifi-qcom** | IPQ60xx/80xx, QCN9074, IPQ53xx | Capability advertised, but **unreliable in mainline** | Limited/unstable | In principle, but moot given instability | WiFi-6/6E/7 | **Avoid for mesh.** STA/peer code paths crash (2025-2026 issues). Real deployments use QSDK fork, not mainline. Matches user's prior pain. |

## Open Questions
- **mt7916 5 GHz AP regression (#912):** Is it fully resolved in the snapshot/firmware the user would actually flash, and does it touch `mesh point` interfaces or only AP? Needs a check of the specific firmware blob version at purchase/flash time.
- **mt7915 vs mt7916 mesh+AP concurrency on one radio:** Whether a single 5 GHz mt7916 radio can simultaneously run a `mesh point` backhaul and a client AP reliably on 24.10/25.x, or whether a dedicated backhaul radio (tri-band board) is required. The Feb 2025 "same-radio stopped working on 24.10.0" report suggests caution but isn't mt7916-specific.
- **Specific mt76 board recommendation:** Filogic boards vary (Banana Pi R3 / R4, GL-MT6000 Flint 2, etc.); per-board DTS/EEPROM and tri-band layout determine whether a dedicated backhaul radio exists. A board-level shortlist (does it have 2×5 GHz?) would de-risk the buy — outside this hypothesis's scope, flagging for the synthesizer.
- **6 GHz mesh on mt76 specifically:** Whether mt7916/mt7986 will even *bring up* a 6 GHz `mesh point` under current regdb (some report 6 GHz mesh blocked by regulatory/AFC), vs. it being merely inadvisable. Not fully resolved here.

## Sub-Hypotheses
None generated (DEPTH_REMAINING was 1; findings converged without needing deeper branching).