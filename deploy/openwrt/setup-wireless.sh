#!/bin/sh
# Configure the radio layer on an MT7981 OpenWrt node for the mjolnir mesh:
#   - 2.4 GHz radio  -> 802.11s mesh-point backhaul (bridged into br-mesh)
#                       + a concurrent client AP on the same channel (for 2.4-only
#                         IoT/ESP32) unless CLIENT_AP_2G=0
#   - 5 GHz radio    -> client AP on br-lan
# Band-detecting, so it's robust to radio0/radio1 ordering across units.
#
# TEMPLATE — run this ON a flashed node, then VERIFY, then we lock the real config
# from the node's generated /etc/config/wireless. Same MESH_ID + MESH_KEY +
# MESH_CHANNEL_2G on EVERY node or they won't form one island.
#
# Decisions (mjolnir-mesh-w1l): 2.4 GHz backhaul (range/foliage), 5 GHz clients.
# Override any value via env, e.g.:  MESH_KEY='s3cret' CLIENT_KEY='hunter2' sh setup-wireless.sh
set -e

MESH_ID="${MESH_ID:-mjolnir-mesh}"
MESH_KEY="${MESH_KEY:-}"                 # empty => OPEN mesh (recommended for first bring-up); set => SAE
MESH_CHANNEL_2G="${MESH_CHANNEL_2G:-6}"  # one shared 2.4 GHz channel mesh-wide
CLIENT_SSID="${CLIENT_SSID:-mjolnir}"
CLIENT_KEY="${CLIENT_KEY:-changeme-client}"
CLIENT_CHANNEL_5G="${CLIENT_CHANNEL_5G:-36}"
CLIENT_AP_2G="${CLIENT_AP_2G:-1}"            # 1 => also run a client AP on 2.4 GHz (concurrent with the mesh-point) for 2.4-only IoT/ESP32; 0 => backhaul-only
CLIENT_AP_2G_ENC="${CLIENT_AP_2G_ENC:-psk2}" # WPA2-PSK by default: most ESP32/cheap IoT lack WPA3-SAE. Set to 'sae-mixed' to match 5 GHz, or 'none' for open.
COUNTRY="${COUNTRY:-DE}"                  # regulatory domain — REQUIRED, or the radios won't initiate (vifs never appear)
DISTANCE="${DISTANCE:-}"                  # metres to the farthest mesh peer; sets ACK timeout for long/foliage links. empty = driver default

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

# --- 5 GHz client AP -> br-lan ---
uci -q delete wireless.clientap || true
uci set wireless.clientap='wifi-iface'
uci set wireless.clientap.device="$radio_5g"
uci set wireless.clientap.mode='ap'
uci set wireless.clientap.ssid="$CLIENT_SSID"
uci set wireless.clientap.network='lan'
uci set wireless.clientap.encryption='sae-mixed'
uci set wireless.clientap.key="$CLIENT_KEY"

# --- 2.4 GHz client AP, concurrent with the mesh-point on the SAME radio/channel ---
# Most ESP32s (classic/S2/S3/C3/C6) and a lot of cheap IoT are 2.4-GHz-only; the
# 5 GHz AP alone locks them out. mt76 runs a mesh-point + AP concurrently on one
# radio — they share channel $MESH_CHANNEL_2G and its airtime (fine for low-bandwidth
# IoT; steer heavy clients to 5 GHz). Same SSID/key as the 5 GHz AP so a device roams
# across bands on one L2. Default WPA2-PSK for max IoT compatibility. CLIENT_AP_2G=0
# restores the old backhaul-only behaviour. (mjolnir-mesh-ab4)
uci -q delete wireless.clientap2g || true
if [ "$CLIENT_AP_2G" = 1 ]; then
	uci set wireless.clientap2g='wifi-iface'
	uci set wireless.clientap2g.device="$radio_2g"
	uci set wireless.clientap2g.mode='ap'
	uci set wireless.clientap2g.ssid="$CLIENT_SSID"
	uci set wireless.clientap2g.network='lan'
	uci set wireless.clientap2g.encryption="$CLIENT_AP_2G_ENC"
	[ "$CLIENT_AP_2G_ENC" = none ] || uci set wireless.clientap2g.key="$CLIENT_KEY"
fi

# --- firewall: put the mesh backhaul in the 'lan' zone so IP *input* (babel hellos,
# iroh, ping) and client<->mesh *forward* (transit) aren't dropped by OpenWrt's
# default input=REJECT / forward=REJECT. Without this, the radios associate at L2
# (ARP resolves) but no IP crosses the mesh and babel never peers. ---
fw_lan_zone=$(uci show firewall | sed -n 's/^firewall\.\(@zone\[[0-9]*\]\)\.name=.lan./\1/p' | head -1)
if [ -n "$fw_lan_zone" ]; then
	uci -q get firewall.$fw_lan_zone.network | grep -qw mesh || uci add_list firewall.$fw_lan_zone.network='mesh'
fi

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

cat <<EOF
>> committed. Now:
     wifi reload                              # brings up mesh0; hotplug auto-disables power-save on it
   Verify the island + bridge:
     iw dev                                  # find the mesh ifname (mode 'mesh point')
     iw dev <mesh-ifname> station dump       # peers appear once another node is up
     ip link show br-mesh                    # must be UP (if DOWN: ip link set br-mesh up)
   Then point meshd at it:  uci set mjolnir.meshd.backhaul_iface='br-mesh'; service mjolnir-meshd restart
EOF
