#!/bin/sh
# Configure the radio layer on an MT7981 OpenWrt node for the mjolnir mesh:
#   - 2.4 GHz radio  -> 802.11s mesh-point backhaul (bridged into br-mesh)
#                       + a concurrent client AP on the same channel (for 2.4-only
#                         IoT/ESP32), STAGED DISABLED — flip per-node with
#                         CLIENT_AP_2G=1 or `uci set wireless.clientap2g.disabled=0`
#   - 5 GHz radio    -> client AP on br-lan
# Band-detecting, so it's robust to radio0/radio1 ordering across units.
#
# TEMPLATE — run this ON a flashed node, then VERIFY, then we lock the real config
# from the node's generated /etc/config/wireless. Same MESH_ID + MESH_KEY +
# MESH_CHANNEL_2G on EVERY node or they won't form one island.
#
# Decisions (mjolnir-mesh-w1l): 2.4 GHz backhaul (range/foliage), 5 GHz clients.
# Override any value via env, e.g.:  MESH_KEY='<mesh-passphrase>' CLIENT_KEY='<client-passphrase>' sh setup-wireless.sh
set -e

MESH_ID="${MESH_ID:-mjolnir-mesh}"
MESH_KEY="${MESH_KEY:-}"                 # empty => OPEN mesh (recommended for first bring-up); set => SAE
MESH_CHANNEL_2G="${MESH_CHANNEL_2G:-6}"  # one shared 2.4 GHz channel mesh-wide
CLIENT_SSID="${CLIENT_SSID:-Lightning Mesh}"
CLIENT_KEY="${CLIENT_KEY:-lightning!}"
CLIENT_CHANNEL_5G="${CLIENT_CHANNEL_5G:-36}"
CLIENT_AP_2G="${CLIENT_AP_2G:-0}"            # ENABLE flag for the 2.4 GHz client AP (concurrent with the mesh-point, for 2.4-only
                                             # IoT/ESP32). The section is ALWAYS rendered so the SSID/key/FT config is staged; the
                                             # flag only sets wireless.clientap2g.disabled. DEFAULT DISABLED (mjolnir-mesh-oaq): an
                                             # ENABLED concurrent AP doesn't just come up start_disabled — on WR3000S it blocks the
                                             # MESH JOIN itself (wpad holds phy0; needs AP removal + reboot to recover). Keep the
                                             # backhaul clean; enable per-node deliberately (leaf nodes, IoT-only — it shares the
                                             # backhaul channel's airtime) once oaq is solved (mjolnir-mesh-ab4):
                                             #   uci set wireless.clientap2g.disabled=0; uci commit wireless; wifi reload
CLIENT_AP_2G_ENC="${CLIENT_AP_2G_ENC:-psk2}" # WPA2-PSK by default: most ESP32/cheap IoT lack WPA3-SAE. Set to 'sae-mixed' to match 5 GHz, or 'none' for open.
COUNTRY="${COUNTRY:-DE}"                  # regulatory domain — REQUIRED, or the radios won't initiate (vifs never appear)
DISTANCE="${DISTANCE:-}"                  # metres to the farthest mesh peer; sets ACK timeout for long/foliage links. empty = driver default
# 802.11r fast transition (mjolnir-mesh-bnd): empty FT_KEY (default) => FT left off,
# same "off means untouched" convention as MESH_KEY above. Set FT_KEY to turn it on for
# EVERY client AP this script configures (5 GHz SAE + 2.4 GHz PSK, when present) — same
# key + mobility domain on every node, or roaming clients reject the handoff as a
# different mobility domain / can't validate the pushed key.
FT_KEY="${FT_KEY:-}"                         # 256-bit hex string (64 hex chars), shared mesh-wide — the r0kh/r1kh key-holder push secret
FT_MOBILITY_DOMAIN="${FT_MOBILITY_DOMAIN:-a1b2}" # 2-octet hex MDID, shared mesh-wide (must match on every node/band the client roams across)

# Discover which radio is 2.4 vs 5 GHz by its 'band' option.
radio_2g=""; radio_5g=""
for r in $(uci show wireless | sed -n 's/^wireless\.\([^.]*\)=wifi-device/\1/p'); do
	case "$(uci -q get wireless.$r.band)" in
		2g) radio_2g="$r" ;;
		5g) radio_5g="$r" ;;
	esac
done
[ -n "$radio_2g" ] || { echo "FATAL: no 2.4 GHz (band=2g) radio found in /etc/config/wireless"; exit 1; }
[ -n "$radio_5g" ] || { echo "FATAL: no 5 GHz (band=5g) radio found in /etc/config/wireless"; exit 1; }
echo ">> 2.4 GHz 802.11s backhaul -> $radio_2g   |   5 GHz client AP -> $radio_5g"

# --- br-mesh: the bridge that carries the 802.11s backhaul L2 (meshd binds 10.254.x here) ---
# NB: `uci -q delete` of a not-yet-existing section returns non-zero; guard each with
# `|| true` so the script's `set -e` doesn't abort on a first (clean) run.
uci -q delete network.mesh || true
uci set network.mesh='interface'
uci set network.mesh.proto='none'         # unmanaged L3: meshd assigns the 10.254.x address
uci set network.mesh.device='br-mesh'
uci -q delete network.br_mesh || true
uci set network.br_mesh='device'
uci set network.br_mesh.name='br-mesh'
uci set network.br_mesh.type='bridge'

# --- radios on, channels + country pinned (country is mandatory or vifs never come up) ---
uci set wireless.$radio_2g.channel="$MESH_CHANNEL_2G"
uci set wireless.$radio_2g.country="$COUNTRY"
uci set wireless.$radio_2g.disabled='0'
# Long/foliage links: widen the ACK timeout so distant peers aren't dropped (if=guard keeps set -e happy when unset).
if [ -n "$DISTANCE" ]; then uci set wireless.$radio_2g.distance="$DISTANCE"; fi
uci set wireless.$radio_5g.channel="$CLIENT_CHANNEL_5G"
uci set wireless.$radio_5g.country="$COUNTRY"
uci set wireless.$radio_5g.disabled='0'

# --- 802.11s backhaul on 2.4 GHz ---
uci -q delete wireless.meshbh || true
uci set wireless.meshbh='wifi-iface'
uci set wireless.meshbh.device="$radio_2g"
uci set wireless.meshbh.mode='mesh'
uci set wireless.meshbh.mesh_id="$MESH_ID"
uci set wireless.meshbh.network='mesh'
# mesh_fwding=1: 802.11s HWMP gives a flat L2 island (mDNS floods, babel sees one
# wired segment). The L3-routed model (mesh_fwding=0, babel does every hop) is the
# spread-out / multi-hop-discovery future — mjolnir-mesh-0yb.
uci set wireless.meshbh.mesh_fwding='1'
if [ -n "$MESH_KEY" ]; then
	# NOTE: SAE on an 802.11s mesh needs wpad-mesh-mbedtls (the stock wpad-basic-mbedtls
	# lacks mesh). Install it first:  apk add wpad-mesh-mbedtls  (replaces wpad-basic).
	# Open mesh (no MESH_KEY) needs nothing extra — it comes up on the stock image.
	uci set wireless.meshbh.encryption='sae'
	uci set wireless.meshbh.key="$MESH_KEY"
else
	uci set wireless.meshbh.encryption='none'
fi

# 802.11r fast transition for a wifi-iface UCI section, keyed by a shared secret instead
# of a per-node peer list: wildcard r0kh (accept a key-holder pull from ANY node) +
# wildcard r1kh (accept a key-holder push FROM any node) let every AP in the mesh push/
# pull PMK-R1 to/from every other AP without this template script knowing peer BSSIDs
# up front (nodes are provisioned one at a time — see the file header). Needed for
# FT-SAE specifically: unlike FT-PSK there's no local-derivation shortcut (a SAE PMK
# isn't a shared secret the way a PSK is), so cross-node roaming needs this key-holder
# push wired up or the fast handoff will only ever work on the ORIGINAL AP the client
# associated to. VERIFY on hardware: the exact r0kh/r1kh field separator UCI expects
# (space vs comma) isn't confirmable without a live wpad — check the rendered
# hostapd-*.conf (`ubus call network.wireless status` / /var/run/hostapd-phy*.conf) for
# literal `r0kh=`/`r1kh=` lines after `wifi reload`.
add_ft_wildcard_rxkh() {
	uci -q delete wireless.$1.r0kh || true
	uci -q delete wireless.$1.r1kh || true
	uci add_list wireless.$1.r0kh="ff:ff:ff:ff:ff:ff * $FT_KEY"
	uci add_list wireless.$1.r1kh="00:00:00:00:00:00 00:00:00:00:00:00 $FT_KEY"
}

# --- 5 GHz client AP -> br-lan ---
uci -q delete wireless.clientap || true
uci set wireless.clientap='wifi-iface'
uci set wireless.clientap.device="$radio_5g"
uci set wireless.clientap.mode='ap'
uci set wireless.clientap.ssid="$CLIENT_SSID"
uci set wireless.clientap.network='lan'
uci set wireless.clientap.encryption='sae-mixed'
uci set wireless.clientap.key="$CLIENT_KEY"
if [ -n "$FT_KEY" ]; then
	# FT-SAE: nasid/r1_key_holder are left unset — hostapd's own default (the AP's own
	# BSSID) is already unique per node, which is exactly what's needed here.
	uci set wireless.clientap.ieee80211r='1'
	uci set wireless.clientap.mobility_domain="$FT_MOBILITY_DOMAIN"
	uci set wireless.clientap.ft_over_ds='0'
	add_ft_wildcard_rxkh clientap
fi

# --- 2.4 GHz client AP, concurrent with the mesh-point on the SAME radio/channel ---
# Most ESP32s (classic/S2/S3/C3/C6) and a lot of cheap IoT are 2.4-GHz-only; the
# 5 GHz AP alone locks them out. mt76 runs a mesh-point + AP concurrently on one
# radio — they share channel $MESH_CHANNEL_2G and its airtime (fine for low-bandwidth
# IoT; steer heavy clients to 5 GHz). Same SSID/key as the 5 GHz AP so a device roams
# across bands on one L2. Default WPA2-PSK for max IoT compatibility. (mjolnir-mesh-ab4)
# The section is rendered UNCONDITIONALLY but disabled unless CLIENT_AP_2G=1 — a
# disabled wifi-iface creates no vif, so it can't trigger the oaq mesh-join breakage,
# and enabling in the field is one uci flip instead of a setup-wireless re-run.
uci -q delete wireless.clientap2g || true
uci set wireless.clientap2g='wifi-iface'
if [ "$CLIENT_AP_2G" = 1 ]; then
	uci set wireless.clientap2g.disabled='0'
else
	uci set wireless.clientap2g.disabled='1'
fi
uci set wireless.clientap2g.device="$radio_2g"
uci set wireless.clientap2g.mode='ap'
uci set wireless.clientap2g.ssid="$CLIENT_SSID"
uci set wireless.clientap2g.network='lan'
uci set wireless.clientap2g.encryption="$CLIENT_AP_2G_ENC"
[ "$CLIENT_AP_2G_ENC" = none ] || uci set wireless.clientap2g.key="$CLIENT_KEY"
if [ -n "$FT_KEY" ] && [ "$CLIENT_AP_2G_ENC" != none ]; then
	uci set wireless.clientap2g.ieee80211r='1'
	uci set wireless.clientap2g.mobility_domain="$FT_MOBILITY_DOMAIN"
	uci set wireless.clientap2g.ft_over_ds='0'
	case "$CLIENT_AP_2G_ENC" in
	*sae*)
		# FT-SAE, same as the 5 GHz AP above — no local-derivation shortcut.
		add_ft_wildcard_rxkh clientap2g
		;;
	*)
		# FT-PSK: hostapd derives PMK-R0/R1 locally from the shared PSK, so no
		# r0kh/r1kh key-holder push is needed between nodes at all.
		uci set wireless.clientap2g.ft_psk_generate_local='1'
		;;
	esac
fi

# --- firewall: put the mesh backhaul in the 'lan' zone so IP *input* (babel hellos,
# iroh, ping) and client<->mesh *forward* (transit) aren't dropped by OpenWrt's
# default input=REJECT / forward=REJECT. Without this, the radios associate at L2
# (ARP resolves) but no IP crosses the mesh and babel never peers. ---
fw_lan_zone=$(uci show firewall | sed -n 's/^firewall\.\(@zone\[[0-9]*\]\)\.name=.lan./\1/p' | head -1)
if [ -n "$fw_lan_zone" ]; then
	uci -q get firewall.$fw_lan_zone.network | grep -qw mesh || uci add_list firewall.$fw_lan_zone.network='mesh'
fi

# --- client DNS on non-egress nodes (a8o): dnsmasq forwards to public
# resolvers, reachable once a gateway node advertises the mesh default route.
# Without this a client on a WAN-less node gets a lease but no name
# resolution (the node's own resolv.conf has no upstream). Harmless on the
# gateway itself. del_list-then-add_list keeps re-runs from duplicating.
uci -q del_list dhcp.@dnsmasq[0].server='9.9.9.9' 2>/dev/null || true
uci -q del_list dhcp.@dnsmasq[0].server='1.1.1.1' 2>/dev/null || true
uci add_list dhcp.@dnsmasq[0].server='9.9.9.9'
uci add_list dhcp.@dnsmasq[0].server='1.1.1.1'
uci commit dhcp
/etc/init.d/dnsmasq restart >/dev/null 2>&1 || true

uci commit network
uci commit wireless
uci commit firewall
fw4 reload >/dev/null 2>&1 || /etc/init.d/firewall reload >/dev/null 2>&1

# --- persist: kill WiFi power-save on the 802.11s backhaul iface (mt76 mesh+PS = peering/latency flaps) ---
# Hotplug fires when `wifi reload` brings the mesh-point iface up, so it survives reboots/reloads.
mkdir -p /etc/hotplug.d/net
cat > /etc/hotplug.d/net/30-mesh-powersave <<'HOTPLUG'
#!/bin/sh
[ "$ACTION" = add ] || exit 0
case "$(iw dev "$DEVICENAME" info 2>/dev/null | sed -n 's/^[[:space:]]*type //p')" in
	"mesh point") iw dev "$DEVICENAME" set power_save off ;;
esac
HOTPLUG
chmod +x /etc/hotplug.d/net/30-mesh-powersave

# --- USB wifi dongles: configure any supported dongle already plugged in ------
# mjolnir-dongle owns the supported-hardware table (vid:pid -> kmods -> role);
# hotplug covers dongles plugged in LATER. --no-reload: the operator's (or
# mjolnir-apply's) wifi reload below activates it together with everything
# else. Silently skipped on a bare template run where the helper isn't
# installed yet. `|| true`: exit 2 = no dongle/no-op, and even a dongle
# failure shouldn't abort the radio setup this script exists for.
if [ -x /usr/sbin/mjolnir-dongle ]; then
	/usr/sbin/mjolnir-dongle apply --no-reload || true
fi

cat <<EOF
>> committed. Now:
     wifi reload                              # brings up mesh0; hotplug auto-disables power-save on it
   Verify the island + bridge:
     iw dev                                  # find the mesh ifname (mode 'mesh point')
     iw dev <mesh-ifname> station dump       # peers appear once another node is up
     ip link show br-mesh                    # must be UP (if DOWN: ip link set br-mesh up)
   Then point meshd at it:  uci set mjolnir.meshd.backhaul_iface='br-mesh'; service mjolnir-meshd restart
EOF

if [ -n "$FT_KEY" ]; then
	cat <<EOF
>> 802.11r (FT_KEY set) — VERIFY before trusting it, this is the untested part:
     grep -E 'ieee80211r|mobility_domain|r0kh|r1kh|ft_psk_generate_local' /var/run/hostapd-phy*.conf
       confirm r0kh=/r1kh= actually rendered (not silently dropped by a uci field-format mismatch)
     iw dev <ap-ifname> info | grep -i 'ft\|mobility'   # or: hostapd_cli -i <ap-ifname> get_config
   Then roam-test: associate a client to one node's AP, walk it to another node, and on
   the client run its own re-assoc timer (or watch \`logread -f | grep -i 'FT\|reassoc'\`
   on both nodes) — should be single-digit milliseconds, not a full handshake.
EOF
fi
