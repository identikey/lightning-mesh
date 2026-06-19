# MikroTik RouterOS Container Deployment Runbook

Deploy `mjolnir-mesh` as a containerized headless daemon on a **MikroTik
L23UGSR-5HaxD2HaxD** (NetMetal ax / L-series).

> **Status:** template runbook. The on-router steps are usable today, but the
> deployable binary (the headless daemon, beads `mjolnir-mesh-tr6`) and its OCI
> image (`mjolnir-mesh-ut9`) do not exist yet. Steps marked **VERIFY** depend on
> your exact RouterOS version — confirm against
> <https://help.mikrotik.com/docs/spaces/ROS/pages/84901929/Container>.

## Board facts that shape everything

| Property | Value | Why it matters |
|----------|-------|----------------|
| CPU | IPQ-5010, **ARM 32-bit (ARMv7)** | Image must be **`linux/arm/v7`**, never arm64 |
| RAM | 256 MB (shared with RouterOS) | Keep the daemon lean; this is the real ceiling |
| Flash | 128 MB NAND | Attach USB storage for the container root |
| OS | RouterOS v7, license level 4 | Container feature available; needs the `container` package |

## Do NOT ship the audio client

The current `mjolnir-mesh` binary (from `crates/mjolnir-node`) is the **audio
example client** — it pulls `cpal`/ALSA + `tract-onnx`, needs audio hardware,
and is heavy. The router runs the **headless substrate daemon** instead. Point
`deploy/mikrotik/build.sh` at that binary (`mjolnir-meshd` by default).

---

## Step 1 — Build the image (on your workstation)

```bash
# from the repo root; pass the headless daemon's bin name if different
deploy/mikrotik/build.sh mjolnir-meshd
# -> deploy/mikrotik/mjolnir-meshd-armv7.tar
```

This cross-builds a static-musl `linux/arm/v7` binary on a `scratch` base via
`messense/rust-musl-cross`. See `deploy/mikrotik/Dockerfile`.

## Step 2 — Install the container package on the router

RouterOS containers need the extra `container` package matching your **RouterOS
version and the `arm` architecture**.

1. Download `container-<version>-arm.npk` from MikroTik (Extra packages).
2. Upload it to the router (drag into Files in WinBox/WebFig, or `scp`).
3. Reboot. Confirm with `/system/package/print` (the `container` package shows enabled).

## Step 3 — Enable container device-mode (PHYSICAL ACCESS REQUIRED)

Containers are gated behind device-mode and **cannot be enabled purely over
SSH** — this is an anti-malware safeguard.

```routeros
/system/device-mode/update container=yes
```

RouterOS will prompt you to confirm by **power-cycling and, within the timeout,
pressing the reset/mode button** (or a cold power-off/on, per the prompt). Have
hands on the device for this step.

## Step 4 — Attach and format USB storage

128 MB NAND is too tight for image + writable layers. Use USB.

```routeros
/disk/print                                  # find the USB disk slot
/disk/format-drive <slot> file-system=ext4   # VERIFY filesystem support
```

The formatted disk gives you a mount point (e.g. `usb1`) for `root-dir`.

## Step 5 — Network the container (veth + bridge)

The container attaches to RouterOS via a **veth** placed on a bridge. RouterOS
keeps doing L2/L3 and DHCP on the LAN; the daemon runs as the P2P/mesh overlay.

```routeros
/interface/veth/add name=veth-mesh address=172.20.0.2/24 gateway=172.20.0.1
/interface/bridge/add name=br-mesh
/interface/bridge/port/add bridge=br-mesh interface=veth-mesh
/ip/address/add address=172.20.0.1/24 interface=br-mesh
# NAT/route to the WAN/LAN as your topology requires
```

> **Addressing decision:** to let the mesh own addressing on a segment, disable
> the RouterOS DHCP server on that interface and let the daemon hand out leases
> there. Otherwise leave RouterOS DHCP in place and the mesh is overlay-only.

> **⚠ TUN risk (VERIFY early):** the substrate uses a TUN device
> (`crates/mjolnir-mesh/src/tun`). A `/dev/net/tun` inside a RouterOS container
> is **not guaranteed** — RouterOS containers are restricted. Validate a TUN
> can be created in-container *before* committing to this design; if not, the
> daemon must operate purely through the veth/bridge (L2/L3) path instead. This
> may feed back into the `mjolnir-mesh-tr6` daemon-scope decision.

## Step 6 — Add and start the container

```routeros
/container/config/set registry-url=https://registry-1.docker.io tmpdir=usb1/tmp

# Option A — upload the tar from Step 1 and import it:
/container/add file=mjolnir-meshd-armv7.tar interface=veth-mesh \
    root-dir=usb1/mjolnir logging=yes \
    envlist=mesh-env

# Option B — pull from a registry you pushed the arm/v7 image to:
# /container/add remote-image=<your-registry>/mjolnir-meshd:armv7 \
#     interface=veth-mesh root-dir=usb1/mjolnir logging=yes

/container/envs/add name=mesh-env key=RUST_LOG value=info

/container/print                              # note the container number
/container/start 0
/container/print                              # status should be 'running'
/log/print where topics~"container"           # daemon logs (logging=yes)
```

> **VERIFY (tar format):** some RouterOS versions expect a flattened root-fs tar
> (`docker export`) rather than a layered `docker save` tar. If `file=` import
> fails, switch `build.sh` to the `docker export` path noted in its comments, or
> use the registry pull (Option B).

## Step 7 — Verify

- `/container/print` → `status: running`
- `/log/print where topics~"container"` → daemon startup logs
- From a mesh peer, confirm this node joins / is reachable.

---

## Quick reference

| Concern | Setting |
|---------|---------|
| Image platform | `linux/arm/v7` |
| Binary | static musl, `scratch` base, headless (no audio) |
| Container enable | `/system/device-mode/update container=yes` + physical button |
| Storage | USB via `/disk`, used as `root-dir` |
| Networking | veth on a bridge; RouterOS keeps DHCP unless mesh owns a segment |
| Biggest unknowns | TUN-in-container support; tar import format |

Related beads: `mjolnir-mesh-tr6` (headless daemon), `mjolnir-mesh-ecd`
(armv7 cross-compile), `mjolnir-mesh-ut9` (OCI image), `mjolnir-mesh-ns1` (this
runbook).
