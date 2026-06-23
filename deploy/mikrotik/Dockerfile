# OCI image for the MikroTik RouterOS container target.
#
# Board: MikroTik L23UGSR-5HaxD2HaxD (IPQ-5010, ARM 32-bit / ARMv7, 256MB RAM,
# 128MB NAND). RouterOS runs this chip in 32-bit mode, so the image MUST be
# linux/arm/v7 — NOT arm64.
#
# Produces a minimal scratch-based image holding a single static-musl binary,
# keeping the image tiny for the 128MB NAND / 256MB RAM budget.
#
# BIN: the headless router daemon. This binary does not exist yet — it is
# tracked by beads mjolnir-mesh-tr6. Do NOT point this at the audio example
# client (the current `mjolnir-mesh` bin in crates/mjolnir-node), which pulls
# cpal/ALSA + tract-onnx and is the wrong thing for a headless router.
ARG BIN=mjolnir-meshd
ARG RUST_TARGET=armv7-unknown-linux-musleabihf
# The mjolnir-meshd bin is gated behind the `daemon` feature (keeps the
# substrate lib iroh-free). Override if building a different bin.
ARG FEATURES=daemon

# --- build stage: bundles the armv7 musl cross toolchain ---
# Pinned to $BUILDPLATFORM so the cross toolchain runs NATIVELY on the build
# host (it cross-compiles to armv7); without this, `--platform linux/arm/v7`
# would force the toolchain itself to run under slow qemu emulation.
FROM --platform=$BUILDPLATFORM messense/rust-musl-cross:armv7-musleabihf AS build
ARG BIN
ARG RUST_TARGET
ARG FEATURES
WORKDIR /src
COPY . .
# Builds only the requested bin's dependency tree (so the audio crates are not
# compiled). The size-optimized release profile lives in .cargo/config.toml.
RUN cargo build --release --locked --target "${RUST_TARGET}" \
      --bin "${BIN}" --features "${FEATURES}" \
 && cp "target/${RUST_TARGET}/release/${BIN}" /out-bin

# --- runtime stage: Alpine (armv7) + babeld for cross-site routing ---
# Switched from `scratch` to Alpine so the babel supervisor has a real `babeld`
# to spawn (apk provides /usr/sbin/babeld on PATH). meshd is a static musl
# binary so it needs no libc; babeld (musl-linked) gets Alpine's. ca-certificates
# gives meshd CA roots for relay/pkarr TLS. Grows the image ~10MB -> ~15MB, well
# within the board's ~86MB free NAND.
FROM alpine:3.20
RUN apk add --no-cache ca-certificates babeld
COPY --from=build /out-bin /mjolnir-meshd
ENTRYPOINT ["/mjolnir-meshd"]
