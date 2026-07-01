//! buw.1 spike node: a single overlay TUN (`mjolnir0`) multiplexing peers, with
//! a plain UDP socket standing in for the iroh `DatagramConn` seam.
//!
//! The point of the spike is to answer ONE question cheaply and conclusively:
//! *does babeld form a neighbour adjacency over a single point-to-multipoint TUN
//! once the daemon emulates multicast?* iroh is irrelevant to that question — it
//! only supplies the datagram pipe — so we replace it with UDP over a veth pair
//! between two network namespaces (see `spike/buw-multicast-spike.sh`). Everything
//! else (the overlay TUN, the multicast fan-out) is the real code from
//! `mjolnir_mesh::tun::overlay`.
//!
//! Usage:
//!   buw-overlay-spike --tun mjolnir0 --addr 10.254.0.1/16 --ll fe80::1 \
//!                     --listen 0.0.0.0:6000 --peer 10.0.0.2:6000
//!
//! Run inside a netns with CAP_NET_ADMIN. It creates the TUN, assigns the
//! addresses, wires the UDP transport, and bridges the two — then runs until
//! killed. babeld is launched separately by the harness on the same `mjolnir0`.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("buw-overlay-spike is Linux-only (needs TUN + rtnetlink)");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    linux::run().await
}

#[cfg(target_os = "linux")]
mod linux {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::sync::Arc;

    use bytes::Bytes;
    use futures_util::stream::TryStreamExt;
    use rtnetlink::new_connection;
    use tokio::net::UdpSocket;

    use mjolnir_mesh::tun::encap::{DatagramConn, EncapError};
    use mjolnir_mesh::tun::overlay::spawn_overlay;
    use mjolnir_mesh::tun::TUNNEL_MTU;

    /// A `DatagramConn` backed by an UNCONNECTED UDP socket — the spike's
    /// stand-in for an iroh connection. Same one-packet-in / one-packet-out
    /// contract.
    ///
    /// We deliberately do NOT `connect()` the socket. On a connected UDP socket a
    /// transient ICMP port-unreachable (inevitable at startup, before the peer
    /// has bound) is latched and surfaced as `ECONNREFUSED` on the next `recv`,
    /// which would tear the receiver task down for good. `send_to`/`recv_from`
    /// on an unconnected socket ignore ICMP errors — matching the resilience an
    /// iroh connection provides — so the overlay keeps flowing across the peer's
    /// startup race.
    #[derive(Clone)]
    struct UdpConn {
        sock: Arc<UdpSocket>,
        peer: SocketAddr,
    }

    #[async_trait::async_trait]
    impl DatagramConn for UdpConn {
        async fn send_datagram(&self, packet: Bytes) -> Result<(), EncapError> {
            self.sock
                .send_to(&packet, self.peer)
                .await
                .map(|_| ())
                .map_err(EncapError::from)
        }

        async fn recv_datagram(&self) -> Result<Bytes, EncapError> {
            let mut buf = vec![0u8; TUNNEL_MTU as usize];
            // Accept from the peer only; ignore stray datagrams.
            let (n, _from) = self.sock.recv_from(&mut buf).await.map_err(EncapError::from)?;
            buf.truncate(n);
            Ok(Bytes::from(buf))
        }
    }

    struct Args {
        tun: String,
        addr: Ipv4Addr,
        prefix: u8,
        ll: Ipv6Addr,
        listen: SocketAddr,
        peer: SocketAddr,
    }

    fn parse_args() -> Args {
        let mut tun = "mjolnir0".to_string();
        let mut addr_cidr = "10.254.0.1/16".to_string();
        let mut ll = "fe80::1".to_string();
        let mut listen = "0.0.0.0:6000".to_string();
        let mut peer = String::new();

        let mut it = std::env::args().skip(1);
        while let Some(flag) = it.next() {
            let mut val = || it.next().unwrap_or_else(|| panic!("missing value for {flag}"));
            match flag.as_str() {
                "--tun" => tun = val(),
                "--addr" => addr_cidr = val(),
                "--ll" => ll = val(),
                "--listen" => listen = val(),
                "--peer" => peer = val(),
                other => panic!("unknown flag: {other}"),
            }
        }
        assert!(!peer.is_empty(), "--peer <ip:port> is required");

        let (addr_s, prefix_s) = addr_cidr
            .split_once('/')
            .expect("--addr must be CIDR, e.g. 10.254.0.1/16");
        Args {
            tun,
            addr: addr_s.parse().expect("bad --addr"),
            prefix: prefix_s.parse().expect("bad --addr prefix"),
            ll: ll.parse().expect("bad --ll"),
            listen: listen.parse().expect("bad --listen"),
            peer: peer.parse().expect("bad --peer"),
        }
    }

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        // RUST_LOG=overlay=debug surfaces per-packet rd-tun/tx-peer/rx-peer/wr-tun
        // lines so the harness can localise where packets are lost.
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with_writer(std::io::stderr)
            .init();

        let args = parse_args();
        eprintln!(
            "[spike] tun={} addr={}/{} ll={} listen={} peer={}",
            args.tun, args.addr, args.prefix, args.ll, args.listen, args.peer
        );

        // 1. Create the overlay TUN.
        let mut cfg = tun::Configuration::default();
        cfg.name(&args.tun).mtu(TUNNEL_MTU).up();
        let device = tun::create_as_async(&cfg)?;

        // 2. Enable IPv6 on the iface (babel transports over fe80::), then assign
        //    the v4 overlay address and a distinct link-local via rtnetlink.
        let _ = std::fs::write(
            format!("/proc/sys/net/ipv6/conf/{}/disable_ipv6", args.tun),
            "0",
        );
        let (conn, handle, _) = new_connection()?;
        tokio::spawn(conn);
        let mut links = handle.link().get().match_name(args.tun.clone()).execute();
        let link = links
            .try_next()
            .await?
            .ok_or("interface vanished after creation")?;
        let idx = link.header.index;
        handle
            .address()
            .add(idx, std::net::IpAddr::V4(args.addr), args.prefix)
            .execute()
            .await?;
        handle
            .address()
            .add(idx, std::net::IpAddr::V6(args.ll), 64)
            .execute()
            .await?;
        handle
            .link()
            .set(rtnetlink::LinkUnspec::new_with_index(idx).up().build())
            .execute()
            .await?;
        eprintln!("[spike] {} up (idx {idx})", args.tun);

        // 3. UDP transport standing in for the iroh DatagramConn. Unconnected
        //    (see UdpConn docs) — we bind locally and address the peer per-send.
        let sock = UdpSocket::bind(args.listen).await?;
        let udp = UdpConn {
            sock: Arc::new(sock),
            peer: args.peer,
        };

        // 4. Bridge TUN <-> UDP with multicast emulation (the real overlay code).
        let (writer, reader) = device.split()?;
        let handles = spawn_overlay(reader, writer, vec![udp], TUNNEL_MTU as usize);
        eprintln!(
            "[spike] overlay bridge up ({} tasks); running until killed",
            handles.task_count()
        );

        // Run until Ctrl-C / SIGTERM. The harness kills us when done.
        tokio::signal::ctrl_c().await.ok();
        handles.abort();
        Ok(())
    }
}
