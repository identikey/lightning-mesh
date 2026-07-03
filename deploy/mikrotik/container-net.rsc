# mjolnir-mesh — container network setup for RouterOS.
#
# Apply on EVERY router (after device-mode=container is enabled). Idempotent
# (safe to re-run) and config-independent: works whether the router has the
# default firewall (forward-drop + LAN/WAN lists) or a blank one, because it
# accepts the container subnet explicitly by src-address rather than relying on
# any interface-list name.
#
# Upload this file, then:  /import file-name=container-net.rsc
#
# Subnet: container 172.20.0.2  <->  router 172.20.0.1 (br-mesh). See
# docs/archive/mikrotik-container/container-networking.md for what each piece does and why.

:local sub "172.20.0.0/24"

# veth — virtual cable; container side = 172.20.0.2, default gw = 172.20.0.1
:if ([:len [/interface/veth/find where name="veth-mesh"]] = 0) do={
    /interface/veth/add name=veth-mesh address=172.20.0.2/24 gateway=172.20.0.1
}

# bridge — virtual switch for the container
:if ([:len [/interface/bridge/find where name="br-mesh"]] = 0) do={
    /interface/bridge/add name=br-mesh
}

# plug the veth into the bridge
:if ([:len [/interface/bridge/port/find where interface="veth-mesh"]] = 0) do={
    /interface/bridge/port/add bridge=br-mesh interface=veth-mesh
}

# router's IP on the bridge = the container's gateway
:if ([:len [/ip/address/find where interface="br-mesh"]] = 0) do={
    /ip/address/add address=172.20.0.1/24 interface=br-mesh
}

# NAT — masquerade the container subnet out to the internet
:if ([:len [/ip/firewall/nat/find where chain="srcnat" src-address=$sub]] = 0) do={
    /ip/firewall/nat/add chain=srcnat action=masquerade src-address=$sub \
        comment="mjolnir container egress"
}

# Firewall — accept the container's forwarded traffic explicitly. On a router
# with a default forward-drop this is REQUIRED for egress; on a blank firewall
# it is a harmless no-op. Placed at the top so a later drop can't pre-empt it.
:if ([:len [/ip/firewall/filter/find where comment="mjolnir container egress"]] = 0) do={
    :if ([:len [/ip/firewall/filter/find]] > 0) do={
        /ip/firewall/filter/add chain=forward action=accept src-address=$sub \
            comment="mjolnir container egress" place-before=0
    } else={
        /ip/firewall/filter/add chain=forward action=accept src-address=$sub \
            comment="mjolnir container egress"
    }
}

:put "mjolnir container-net: done. veth-mesh + br-mesh + NAT + forward-accept in place."
