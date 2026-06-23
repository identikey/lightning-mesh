# mjolnir-mesh — LAN-backhaul container networking for RouterOS (OFFLINE mesh).
#
# Use THIS script instead of container-net.rsc for the same-site, WiFi/wired,
# no-internet mesh (the `mesh` daemon's default `--lan` mode). The difference:
#
#   container-net.rsc      isolates the container on its own 172.20.0.0/24 and
#                          masquerades it out to the internet. Good when the
#                          container needs internet egress (relay/pkarr mode),
#                          but it DOUBLE-NATs the container, which stops same-LAN
#                          nodes from forming direct iroh paths (mjolnir-mesh-67h).
#
#   container-net-lan.rsc  (this file) bridges the container directly onto the
#                          shared L2 segment that the other mesh nodes are on, so
#                          every node's container is mutually reachable. meshd then
#                          self-assigns each node a stable IPv4 backhaul address
#                          (10.254.0.0/16, host derived from node id) and the
#                          nodes discover + connect DIRECTLY over the LAN via mDNS
#                          — no relay, no DHCP, no internet (mjolnir-mesh-4pk).
#
# Apply on EVERY node (after device-mode=container is enabled). Idempotent.
# Upload this file, then:  /import file-name=container-net-lan.rsc
#
# ─────────────────────────────────────────────────────────────────────────────
# REQUIRED: set $meshLink to the interface on the shared segment that reaches the
# OTHER mesh nodes — i.e. the port into the common switch (bench) or the WiFi
# backhaul interface (deployment). This is the ONE value you must get right; the
# container is bridged onto this L2 so peers can see its backhaul address. There is no safe
# default (bridging the wrong port can disrupt the node's own connectivity), so
# the script refuses to run until you set it.
# ─────────────────────────────────────────────────────────────────────────────
:local meshLink "CHANGE-ME"

:if ($meshLink = "CHANGE-ME") do={
    :put "ERROR: edit container-net-lan.rsc and set \$meshLink to the interface facing the other mesh nodes (e.g. ether1 or the WiFi backhaul iface). Aborting."
    :error "meshLink not set"
}

# veth — the container's virtual NIC. The container side comes up as eth0; meshd
# adds the derived 10.254.0.0/16 backhaul address to it itself. The address here
# is only a placeholder so the interface is valid/up; the backhaul rides on the
# 10.254 address meshd assigns, so this needn't match the LAN. (If you also want
# the container reachable/managed on the LAN, give it a real LAN address + gateway.)
:if ([:len [/interface/veth/find where name="veth-mesh"]] = 0) do={
    /interface/veth/add name=veth-mesh address=172.20.0.2/24 gateway=172.20.0.1
}

# bridge — spans the container veth AND the shared-segment interface, so the
# container shares one L2 broadcast domain with every other node's container.
:if ([:len [/interface/bridge/find where name="br-mesh"]] = 0) do={
    /interface/bridge/add name=br-mesh
}

# plug the container veth into the bridge
:if ([:len [/interface/bridge/port/find where interface="veth-mesh"]] = 0) do={
    /interface/bridge/port/add bridge=br-mesh interface=veth-mesh
}

# plug the shared-segment interface into the SAME bridge — this is what puts the
# container on the LAN with the other nodes. (Bridges forward the multicast that
# IPv6 neighbour discovery uses, so the containers ND each other directly; the
# router does no L3 for them.)
:if ([:len [/interface/bridge/port/find where interface=$meshLink]] = 0) do={
    /interface/bridge/port/add bridge=br-mesh interface=$meshLink
}

# NOTE: no NAT/masquerade is needed for the offline backhaul — the nodes talk
# peer-to-peer over the bridged L2 on their 10.254.0.0/16 backhaul addresses, and
# the `mesh` daemon runs in --lan mode (no relay, no pkarr, no internet). If you
# later need internet egress for relay/pkarr mode, use container-net.rsc instead
# (or add masquerade + a real LAN address to the veth here).

:put ("mjolnir container-net-lan: done. veth-mesh + " . $meshLink . " bridged on br-mesh. meshd will self-assign the 10.254.0.0/16 backhaul address on eth0.")
