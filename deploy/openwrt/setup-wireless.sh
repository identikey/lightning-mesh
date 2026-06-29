#!/bin/sh
# Configure the radio layer on an MT7981 OpenWrt node for the mjolnir mesh:
#   - 2.4 GHz radio  -> 802.11s mesh-point backhaul, bridged into br-mesh
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
COUNTRY="${COUNTRY:-DE}"                  # regulatory domain — REQUIRED, or the radios won't initiate (vifs never appear)

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

uci commit network
uci commit wireless

cat <<EOF
>> committed. Now:
     wifi reload
   Verify the island + bridge:
     iw dev                                  # find the mesh ifname (mode 'mesh point')
     iw dev <mesh-ifname> station dump       # peers appear once another node is up
     ip link show br-mesh                    # must be UP (if DOWN: ip link set br-mesh up)
   Then point meshd at it:  uci set mjolnir.meshd.backhaul_iface='br-mesh'; service mjolnir-meshd restart
EOF
