# mjolnir-mesh — DRAFT wireless backhaul for RouterOS v7 (wifi-qcom). mjolnir-mesh-b1d.
#
# NOT hardware-tested. Replaces the wired switch with a WiFi station-bridge so the
# 4 nodes are self-contained (no home router, no switch). Hardware: L23UGSR /
# IPQ-5010, RouterOS v7 "wifi" package (/interface/wifi), dual-band. Do the wired
# 4-node validation (mjolnir-mesh-2j6) FIRST — this only swaps the L1/L2 backhaul;
# the container/software layer is unchanged.
#
# TOPOLOGY: star. ONE node = root AP on 5 GHz; the other THREE = station-bridge.
# All four put their wifi backhaul + container veth on br-mesh -> one L2 domain.
# (802.11s/HWMP+ is NOT available in wifi-qcom — legacy "wireless" only. And
#  station-bridge requires every node on wifi-qcom, which they are.)
#
# Why mDNS still works across WiFi: RouterOS bridges ALWAYS flood link-local
# multicast (224.0.0.0/24 incl. mDNS 224.0.0.251, and ff02:: for IPv6 ND),
# regardless of igmp-snooping. NOTE: `multicast-enhance` is broken on wifi-qcom
# (as of 7.23) — leave it OFF; multicast still forwards, just at low 802.11 rate,
# which is fine for our small discovery packets.
#
# Composition: this script builds br-mesh and bridges the wifi backhaul into it,
# so it SUPERSEDES container-net.rsc / container-net-lan.rsc for the wireless case.
# Add the container veth (veth-mesh) to br-mesh here. Run meshd with the default
# --backhaul-iface eth0 (unchanged); meshd self-assigns its 10.254/16 addr on eth0.
#
# EDIT THESE, then /import file-name=wifi-backhaul.rsc :
:local role        "station"            ;# "root" on ONE node, "station" on the other 3
:local backhaulIf  "wifi1"              ;# 5 GHz radio (check /interface/wifi print)
:local ssid        "MJOLNIR-BACKHAUL"
:local passphrase  "CHANGE-ME-STRONG"
:local country     "United States"      ;# MUST match your regulatory domain
:local channel     "5180"               ;# pin a clean 5 GHz freq on the ROOT (e.g. ch36); stations follow

# --- bridge (RSTP; always-flood broadcast/multicast for ARP/ND/mDNS) ----------
:if ([:len [/interface/bridge/find where name="br-mesh"]] = 0) do={
    /interface/bridge/add name=br-mesh protocol-mode=rstp igmp-snooping=no
}

# --- datapath that drops the wifi backhaul straight into br-mesh ---------------
:if ([:len [/interface/wifi/datapath/find where name="dp-backhaul"]] = 0) do={
    /interface/wifi/datapath/add name=dp-backhaul bridge=br-mesh
}

# --- the 5 GHz backhaul radio: AP on the root, station-bridge on the others ----
:if ($role = "root") do={
    /interface/wifi/set [find default-name=$backhaulIf] \
        configuration.mode=ap configuration.ssid=$ssid configuration.country=$country \
        channel.frequency=$channel \
        datapath=dp-backhaul \
        security.authentication-types=wpa2-psk security.passphrase=$passphrase \
        disabled=no
} else={
    /interface/wifi/set [find default-name=$backhaulIf] \
        configuration.mode=station-bridge configuration.ssid=$ssid configuration.country=$country \
        datapath=dp-backhaul \
        security.authentication-types=wpa2-psk security.passphrase=$passphrase \
        disabled=no
}

# --- put the container veth on br-mesh; ensure flooding on its ports -----------
:if ([:len [/interface/bridge/port/find where interface="veth-mesh"]] = 0) do={
    /interface/bridge/port/add bridge=br-mesh interface=veth-mesh
}
:foreach p in=[/interface/bridge/port/find where bridge="br-mesh"] do={
    /interface/bridge/port/set $p broadcast-flood=yes unknown-multicast-flood=yes unknown-unicast-flood=yes
}

:put ("mjolnir wifi-backhaul: done. role=" . $role . " on " . $backhaulIf . " (ssid " . $ssid . "). br-mesh = wifi + veth-mesh.")

# Client access (optional): use the OTHER radio (e.g. wifi2 / 2.4 GHz) as a client
# AP — configuration.mode=ap on its own datapath/bridge. The two radios run
# independently (documented "repeater" pattern). Left out here to keep backhaul
# bring-up isolated.
