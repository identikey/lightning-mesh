# OpenWrt mt76 node deploy — mjolnir-mesh

For the open WiFi-6 mesh nodes (OpenWrt on mt76 hardware: MT7981 / MT7986,
aarch64). Unlike the MikroTik target there is **no container** — OpenWrt is real
Linux, so the overlay runs as a native static binary supervised by procd,
alongside babeld. See `mjolnir-mesh-0xu` / `mjolnir-mesh-w1l` (Cudy MT7981 fleet)
and `docs/network-coordination/radio-backhaul-and-discovery.md`.

## Build

```sh
deploy/openwrt/build.sh            # -> deploy/openwrt/mjolnir-meshd-aarch64
```

Static `aarch64-unknown-linux-musl` binary (no shared-lib deps), cross-built in
the `messense/rust-musl-cross:aarch64-musl` container (the repo is mounted, so
`target/` is reused and rebuilds are incremental). The artifact is git-ignored —
rebuild as needed. The startup banner stamps the git short-SHA (`MJOLNIR_BUILD`,
`-dirty` if the tree is dirty); see "Verify identity" below.

## Install on a node

One command — idempotent, safe to re-run:

```sh
deploy/openwrt/install-node.sh root@<node-ip>
```

It pushes the binary to `/usr/bin/mjolnir-meshd`, installs the procd init scripts,
installs `babeld` + `kmod-tun` (via `apk` on OpenWrt 25.12+, else `opkg`), hands
babeld supervision to procd (see below), swaps `wpad-basic`→`wpad-mesh` (802.11s
SAE), and enables the meshd service. On a **fresh** node it also drops the UCI
config template; on an existing node it leaves `/etc/config/mjolnir` untouched
(your peers survive). It does **not** start meshd — you set peers first.

`kmod-tun` is **required** whenever per-peer iroh tunnels run (`lan_tunnels=1` or
`mode internet`): without it `/dev/net/tun` is absent and a tunnel fails to come
up with `No such file or directory (os error 2)`.

What lands on the node:

| path | role |
|------|------|
| `/usr/bin/mjolnir-meshd`          | the static daemon |
| `/etc/init.d/mjolnir-meshd`       | procd service (START=95) |
| `/etc/init.d/mjolnir-babeld`      | procd service for babeld (START=96) |
| `/etc/config/mjolnir`             | UCI config (peers, backhaul_iface, mode, …) |
| `/root/setup-wireless.sh`         | 802.11s backhaul + client-AP helper |

### babeld is supervised by procd, not meshd (mjolnir-mesh-m8t)

Split of concerns: **meshd renders the config** (`/etc/mjolnir/babeld.conf`) and
**procd owns the process *and* the restarts**. `mjolnir-babeld` declares
`procd_set_param file /etc/mjolnir/babeld.conf`, so procd restarts babeld whenever
meshd rewrites it — meshd starts babeld once and otherwise stays out of the
restart loop. (Driving those restarts synchronously from meshd wedged the daemon
under rapid config churn — `mjolnir-mesh-qz9`.) meshd never `fork()`s babeld
itself; that chain orphaned babelds on `SIGKILL`. `install-node.sh` disables the
stock `babeld` service so the two don't both run.

## Reaching & operating nodes (runbook)

The gotchas that otherwise get re-discovered every time:

**Install over Ethernet / out-of-band.** `install-node.sh` swaps
`wpad-basic`→`wpad-mesh`, which **bounces wifi** — SSHing in over the 802.11s mesh
you're reconfiguring will cut your own session. Use a wired LAN port.

**Reaching a node — three paths:**
- **Wired / direct** — plug the build machine into the node's LAN port; the node
  answers at OpenWrt's default `192.168.1.1`. Simplest, and survives a wifi/meshd
  bounce.
- **Over the mesh (jump host)** — a node that's only on the 802.11s backhaul is
  reachable *through* a wired peer with SSH `ProxyJump`, at its derived `10.254.x`
  backhaul address (SSH is allowed because the mesh iface sits in the `lan` zone).
  Find the address with `mjolnir-meshd id` → it's `10.254.<blake3(node_id)[0..2]>`,
  or just read it off a wired peer (`ip -4 route | grep 10.254`). Example
  `~/.ssh/config`:
  ```
  Host gw            # the wired peer
      HostName 192.168.1.1
  Host leaf          # mesh-only peer, reached through gw
      HostName 10.254.x.y
      ProxyJump gw
  ```
- **NOT the WAN** — OpenWrt firewalls SSH on the `wan` zone, so a node's upstream
  IP refuses `:22`. Use the LAN port or the mesh jump.

**`scp` to OpenWrt needs `-O`.** Dropbear has no SFTP subsystem, so a modern
`scp` (SFTP by default) fails with `sftp-server: not found`. Use `scp -O` (legacy
protocol) for any manual copy. `install-node.sh` already does, and lands the
binary via `.new`+`mv` so replacing the *running* binary doesn't hit `ETXTBSY`.

**Shared direct-link IP / host-key churn.** Every node uses `192.168.1.1` on its
LAN port, so swapping which node is wired changes the SSH host key. Clear it
first: `ssh-keygen -R 192.168.1.1`.

**Update an in-place node.** Re-run `install-node.sh root@<ip>` (config is
preserved), then `service mjolnir-meshd restart` to exec the new binary. Verify:
`sha256sum /usr/bin/mjolnir-meshd` matches `deploy/openwrt/mjolnir-meshd-aarch64`.

**Recover / un-stick a node.** If a node hangs after enabling an experimental
flag (e.g. `lan_tunnels=1` hitting `mjolnir-mesh-qz9`), disable it and restart:
```sh
ssh root@<node> 'uci set mjolnir.meshd.lan_tunnels=0; uci commit mjolnir; service mjolnir-meshd restart'
```
`install-node.sh` does not keep a backup of the previous binary — a bad binary
is recovered by re-running `install-node.sh` with a known-good build.

**`lan_tunnels` (experimental, default `0`).** `lan_tunnels=1` re-enables per-peer
iroh tunnels in LAN mode (the `mjolnir-mesh-auu` retest); needs `kmod-tun`. It
currently triggers a daemon hang shortly after a tunnel forms
(`mjolnir-mesh-qz9`) — keep it `0` for production until that's fixed.

## Configure & run

Edit `/etc/config/mjolnir`, then start the service:

```sh
# this node's id (stable, derived from the persistent secret):
ssh root@<node> 'mjolnir-meshd id --secret-file /etc/mjolnir/secret'

# add the OTHER nodes' ids to /etc/config/mjolnir   (list peer '<64-hex-id>')
# set backhaul_iface: 'br-lan' for the wired-switch bench, or run
#   /root/setup-wireless.sh  then set it to 'br-mesh' for the 802.11s backhaul.

ssh root@<node> 'service mjolnir-meshd start && logread -e mjolnir_meshd'
```

The daemon defaults to `--lan` (offline: mDNS, no relay). meshd self-assigns its
`10.254.0.0/16` backhaul address on `backhaul_iface`, peers discover each other
over the flat L2 via mDNS, babel routes over it as `type wired`, and meshd assigns
the claimed /24's `.1` on `client_iface` as a connected route babel redistributes
(`mjolnir-mesh-e4r`).

Notes:
- Runs as root (needs `CAP_NET_ADMIN` for the backhaul address + TUNs); fine on OpenWrt.
- Persistent `--secret-file` (default `/etc/mjolnir/secret`) → stable node id across reboots.
- For an internet/relay node, set `option mode 'internet'` (and optionally `option relay <url>`).

## Verify identity (mjolnir-mesh-0xu / mjolnir-mesh-auu)

Two routers in one mesh must run the **same binary**. `CARGO_PKG_VERSION` is
`0.1.0` for every build, so the startup banner also logs `MJOLNIR_BUILD` (git
short-SHA). Compare it across nodes before suspecting a transport bug:

```sh
ssh root@<node> 'logread -e "mjolnir-meshd starting"'   # version= build= must match every node
```

A clean SHA (no `-dirty`) means the deployed binary is traceable to a committed
source tree. Pair with a `sha256sum /usr/bin/mjolnir-meshd` check at deploy time.

## Diagnostics

`service mjolnir-meshd diag` (or `mjolnir-meshd status --secret-file <path>`) is a
read-only, daemon-free dump of ground truth: build stamp, node id, the derived
`10.254.x` backhaul address, every interface's IPv4 addresses (it flags a
dual-addressed backhaul interface — the `auu` failure mode where an extra address
leaks as a bogus next-hop), and the installed mesh-space kernel routes with their
next-hops. Use it to answer "is the backhaul addr up, did babel install routes and
via what next-hop" without grepping logs.

```sh
ssh root@<node> 'service mjolnir-meshd diag'
```

## Radio side (separate)

The 802.11s mesh + client AP config lives at the OpenWrt/wifi layer — see
`setup-wireless.sh` and the design note. meshd only needs the resulting `br-mesh`
L2 to exist.
