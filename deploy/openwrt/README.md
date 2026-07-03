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

One command — idempotent, safe to re-run, and safe to run **in-band** (over the
802.11s mesh or the very wifi being reconfigured — mjolnir-mesh-6e5):

```sh
deploy/openwrt/install-node.sh root@<node-ip>                      # binary/deps only
deploy/openwrt/install-node.sh --wireless node.env root@<node-ip>  # + (re)run setup-wireless.sh
```

It works in two phases. **Stage** (non-disruptive): pushes the binary, init
scripts, `setup-wireless.sh`, and the applier to `/root/mjolnir-stage`, and
prefetches the packages (`babeld`, `kmod-tun`, both `wpad` variants) as local
files — via the node's feeds when it has internet, else from the local
`deploy/openwrt/pkg-cache/` (auto-filled from any node that *can* fetch; a
fresh box with no WAN installs entirely from the cache). **Apply** (detached):
`mjolnir-apply` runs on the node under `setsid`, so the SSH session dying
mid-wifi-bounce doesn't matter. It snapshots configs/binary/wpad, applies
idempotently (the `wpad-basic`→`wpad-mesh` swap is **skipped once done** — 
re-runs never bounce wifi), then health-gates: if the apply touched wifi or
restarted a live meshd, a mesh peer or a pre-apply `10.254.x` neighbour must
answer within `--health-timeout` (default 120 s) or **everything rolls back**
(configs, previous binary, previous wpad from the prefetched package) and wifi
comes back up on the old config. `install-node.sh` polls
`/root/mjolnir-stage/result` over reconnecting SSH and reports
`OK` / `ROLLED_BACK` / `FAILED` with the log tail. `--stage-only` stages
without applying.

On a **fresh** node it also drops the UCI config template; on an existing node
it leaves `/etc/config/mjolnir` untouched (your peers survive). It does **not**
start meshd — you set peers first. Fresh nodes have no health baseline (no
peers, no `10.254.x` neighbours), so the gate is skipped — there is nothing to
regress.

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
| `/usr/sbin/mjolnir-apply`         | detached applier (snapshot → apply → health gate → rollback) |
| `/usr/sbin/mjolnir-dongle`        | plug-and-play USB wifi (supported-hardware table + auto-config) |
| `/etc/hotplug.d/usb/70-mjolnir-dongle` | configures a supported dongle the moment it's plugged in |
| `/root/mjolnir-stage/`            | staged payload, prefetched packages, apply log + result |

### USB wifi dongles are plug-and-play (`mjolnir-dongle`)

Plug a **supported** USB wifi dongle into any fleet node and it configures
itself — via hotplug on a running node, and during `setup-wireless.sh` /
`mjolnir-apply` when one is already present. No per-node forethought needed:
drivers for *every* device in the supported-hardware table are preinstalled on
*every* node (and prefetched with dependencies into `pkg-cache/`), so a dongle
works in the field even on a node with no internet.

The table lives at the top of `files/usr/sbin/mjolnir-dongle`
(`vid:pid → kmods → role`) — **adding a validated device there is the whole
procedure**. Supported today:

| device | vid:pid | role |
|--------|---------|------|
| Ralink RT5370 2.4 GHz | `148f:5370` | `ap2g` — dedicated 2.4 GHz client AP |

The `ap2g` role brings the dongle up as a 2.4 GHz client AP bridged into
`br-lan`, mirroring the staged `clientap2g` SSID/key (Lightning Mesh, WPA2 for
IoT), on the far end of the band from the mesh backhaul channel. This is the
practical answer to the `oaq` quirk: the internal radio can't safely run
mesh-point + AP concurrently, so the dongle carries the 2.4 GHz clients.
Manual runs/debug: `mjolnir-dongle apply`, `logread -e mjolnir-dongle`.

### babeld is supervised by procd, not meshd (mjolnir-mesh-m8t)

Split of concerns: **meshd renders the config** (`/etc/mjolnir/babeld.conf`) and
**procd owns the process *and* the restarts**. `mjolnir-babeld` declares
`procd_set_param file /etc/mjolnir/babeld.conf`, so procd restarts babeld whenever
meshd rewrites it — meshd starts babeld once and otherwise stays out of the
restart loop. (Driving those restarts synchronously from meshd wedged the daemon
under rapid config churn — `mjolnir-mesh-qz9`.) meshd never `fork()`s babeld
itself; that chain orphaned babelds on `SIGKILL`. `install-node.sh` disables the
stock `babeld` service so the two don't both run. Note babeld 1.13 exits on
`SIGHUP` rather than reloading, so config reloads are clean restarts
(`mjolnir-mesh-2zz` tracks adding procd respawn).

## Fleet rollout

The fleet is inventoried in `fleet-nodes.conf` (name → derived `10.254.x` →
node id → model; leaf-first order, wired jump node last). Roll an update to
every reachable node, one at a time, with:

```sh
deploy/openwrt/update-fleet.sh                       # binary/deps everywhere
deploy/openwrt/update-fleet.sh --wireless node.env   # + radio config everywhere
deploy/openwrt/update-fleet.sh m3000                 # a single named node
```

It halts on the first failure (that node has rolled itself back; re-runs are
idempotent) and skips-and-reports unreachable nodes. After a rollout, sweep the
fleet with `deploy/openwrt/validate-fleet.sh` (read-only: build stamp, effective
backhaul vs inventory, pt9 claim convergence, babel routes).

Reaching `10.254.x` depends on where the workstation sits:

- **On the mesh** (client Wi-Fi or a node's LAN port): `10.254/16` is
  babel-routed directly from the client subnet — **no jump host**. A stale
  `ProxyJump` line actively breaks this; keep the stanza jump-free:

  ```
  Host 10.254.*
      User root
      StrictHostKeyChecking accept-new
  ```

- **On the upstream LAN** (outside the mesh, e.g. the network feeding the
  gateway's WAN): the mesh correctly refuses SSH on WAN, so you need one wired
  node as jump host — add `ProxyJump root@192.168.1.1` to the stanza above.

New box? Provision it over ethernet first (`install-node.sh root@192.168.1.1`,
then `--wireless`), get its id (`mjolnir-meshd id`), and add its line to
`fleet-nodes.conf`. Design rationale: `docs/deploy/node-operations.md`.
### Secrets

Fleet-wide wireless secrets (`MESH_KEY`, `CLIENT_KEY`, `FT_KEY`, …) live in
`fleet-secrets/wireless.env` — **gitignored**; copy the checked-in
`wireless.env.example` and fill it in, then pass it with
`--wireless fleet-secrets/wireless.env`. The applier wipes the staged copy
from the node after use (the values persist in `/etc/config/wireless`, which
is their job). Secrets that deliberately do NOT live in the repo or this dir:
each node's iroh identity (`/etc/mjolnir/secret`, generated on-node, never
leaves it — a dead node means a new identity + inventory line, by design),
root passwords (set at flash time, keep them in your password manager), and
the workstation SSH key that dropbear trusts (`~/.ssh`).

## Reaching & operating nodes (runbook)

The gotchas that otherwise get re-discovered every time:

**Installs/updates run in-band.** `install-node.sh` stages first and applies
detached with health-gated rollback, so the wifi bounce (wpad swap,
`setup-wireless.sh`) cutting your SSH session is expected and harmless — the
script reconnects and reports the result. Ethernet at `192.168.1.1` is the
**recovery of last resort** (e.g. a rollback that still didn't restore
reachability), not a requirement.

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
preserved, the wpad swap is skipped, and a running meshd is restarted onto the
new binary automatically — with rollback to the previous binary if the mesh
doesn't come back). Verify: `sha256sum /usr/bin/mjolnir-meshd` matches
`deploy/openwrt/mjolnir-meshd-aarch64`.

**Recover / un-stick a node.** If a node hangs after enabling an experimental
flag (e.g. `lan_tunnels=1` hitting `mjolnir-mesh-qz9`), disable it and restart:
```sh
ssh root@<node> 'uci set mjolnir.meshd.lan_tunnels=0; uci commit mjolnir; service mjolnir-meshd restart'
```
During an apply the previous binary/configs/wpad live in
`/root/mjolnir-stage/backup/` and are restored automatically if the health gate
fails; a bad binary that *passes* the gate is still recovered by re-running
`install-node.sh` with a known-good build. Post-mortem: `cat
/root/mjolnir-stage/apply.log` (and `result`).

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
# run /root/setup-wireless.sh and set backhaul_iface: 'br-mesh' for the
#   802.11s backhaul (backhaul_iface: 'br-lan' is bench-only, superseded —
#   the fleet runs 802.11s 'br-mesh').

ssh root@<node> 'service mjolnir-meshd start && logread -e mjolnir_meshd'
```

The daemon defaults to `--lan` (offline, no relay). meshd self-assigns its
`10.254.0.0/16` backhaul address on `backhaul_iface` and pins iroh's socket to
it, and seeds iroh's address book with every roster peer's fully derived
address — gossip dials need no discovery lookup at all (`mjolnir-mesh-0yb.1`;
mDNS remains a link-local bootstrap fallback only). Babel routes over the
backhaul as `type wired`, and meshd assigns
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
