//! Single overlay-TUN multiplexer with multicast emulation (mjolnir-mesh-buw.1).
//!
//! Where [`spawn_encap_pair`](crate::tun::encap::spawn_encap_pair) bridges ONE
//! per-peer TUN to ONE iroh connection (the current per-peer-tunnel data plane),
//! this bridges ONE overlay TUN (`mjolnir0`) to MANY peer connections — the
//! `buw` fork that lets babeld see a single multi-access interface instead of N
//! churning point-to-point tunnels.
//!
//! Three moving parts:
//!   - one **reader** task: read each IP packet off the TUN and forward it. A
//!     link-local multicast packet (babel Hello -> `ff02::1:6`) is REPLICATED to
//!     every peer connection — the multicast emulation a TUN cannot do itself.
//!     A unicast packet is, in this spike, also flooded to all peers; buw.4
//!     replaces that branch with `LPM(dest)->peer` once the FIB mirror exists.
//!   - one **writer** task: owns the TUN write half and drains an mpsc queue fed
//!     by every peer, so inbound packets from all connections funnel to the one
//!     TUN without contending for a lock.
//!   - one **receiver** task per connection: read datagrams and push them to the
//!     writer queue.
//!
//! The reader keys its forward decision on [`classify`], so buw.4 can slot an
//! LPM router into the unicast branch without touching the multicast path.

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::tun::encap::{DatagramConn, EncapError};
use crate::tun::mcast::{OverlayDest, classify};

/// Depth of the inbound queue feeding the single TUN writer. Generous: a burst
/// of babel updates from many peers should never block a receiver task.
const INBOUND_QUEUE: usize = 1024;

/// Handles for the overlay multiplexer's tasks. Dropping or calling
/// [`abort`](OverlayHandles::abort) tears the whole thing down.
pub struct OverlayHandles {
    tasks: Vec<JoinHandle<Result<(), EncapError>>>,
}

impl OverlayHandles {
    /// Abort every task. Idempotent.
    pub fn abort(&self) {
        for t in &self.tasks {
            t.abort();
        }
    }

    /// The number of live tasks (1 reader + 1 writer + one receiver per peer).
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }
}

/// Resolves how the overlay forwards a UNICAST packet read off the TUN: to the
/// single peer connection that owns the destination, or `None` to drop it as
/// unroutable. (Multicast is never routed here — the overlay floods it to every
/// peer for babel discovery.) The daemon implements this over the FIB
/// (`dest -> next-hop 10.254.x`) plus the connection manager
/// (`next-hop -> Connection`); tests and the spike use a plain closure.
pub trait UnicastRouter<C>: Send + 'static {
    /// Return the connection that should carry this unicast `packet`, or `None`.
    fn resolve(&self, packet: &[u8]) -> Option<C>;
}

impl<C, F> UnicastRouter<C> for F
where
    F: Fn(&[u8]) -> Option<C> + Send + 'static,
{
    fn resolve(&self, packet: &[u8]) -> Option<C> {
        self(packet)
    }
}

/// Spawn the inbound half shared by both overlay variants: one writer task that
/// drains an mpsc queue to the TUN, and one receiver task per connection feeding
/// it. Returns the spawned tasks (writer first).
fn spawn_inbound<W, C>(tun_write: W, conns: &[C]) -> Vec<JoinHandle<Result<(), EncapError>>>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    C: DatagramConn + Clone,
{
    let (inbound_tx, mut inbound_rx) = mpsc::channel::<Bytes>(INBOUND_QUEUE);
    let mut tasks = Vec::with_capacity(conns.len() + 1);

    // Writer task — sole owner of the TUN write half.
    let mut tun_write = tun_write;
    tasks.push(tokio::spawn(async move {
        while let Some(pkt) = inbound_rx.recv().await {
            tun_write.write_all(&pkt).await?;
            tracing::debug!(target: "overlay", "wr-tun {}B", pkt.len());
        }
        Ok(())
    }));

    // One receiver task per peer connection.
    for conn in conns {
        let conn = conn.clone();
        let inbound_tx = inbound_tx.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                match conn.recv_datagram().await {
                    Ok(pkt) => {
                        tracing::debug!(target: "overlay", "rx-peer {}B", pkt.len());
                        // Writer gone => nothing to deliver to; stop.
                        if inbound_tx.send(pkt).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(EncapError::ConnectionClosed) => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
        }));
    }
    tasks
}

/// Bridge one overlay TUN to `conns`, FLOODING every packet to all peers.
///
/// This is the buw.1 spike / walking-skeleton data plane: correct for a single
/// peer (or a full mesh with no transit), but it does not scale to multi-hop
/// because it floods unicast too. Production uses [`spawn_overlay_routed`], which
/// routes unicast to one peer via a [`UnicastRouter`] while still flooding
/// multicast. `mtu` bounds each TUN read (one read = one IP packet).
pub fn spawn_overlay<R, W, C>(
    mut tun_read: R,
    tun_write: W,
    conns: Vec<C>,
    mtu: usize,
) -> OverlayHandles
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    C: DatagramConn + Clone,
{
    let mut tasks = spawn_inbound(tun_write, &conns);

    // Reader task — read each packet off the TUN and flood it to every peer.
    let send_conns = conns;
    tasks.push(tokio::spawn(async move {
        let mut buf = vec![0u8; mtu];
        loop {
            let n = match tun_read.read(&mut buf).await {
                Ok(0) => return Ok(()), // TUN closed
                Ok(n) => n,
                Err(e) => return Err(EncapError::from(e)),
            };
            let pkt = Bytes::copy_from_slice(&buf[..n]);
            let dest = classify(&pkt);
            tracing::debug!(target: "overlay", "rd-tun {n}B {dest:?}");
            match dest {
                // Both multicast (babel Hello) and unicast are flooded here.
                Some(OverlayDest::Multicast) | Some(OverlayDest::Unicast) => {
                    for conn in &send_conns {
                        // A single dead peer must not sink the whole overlay;
                        // best-effort per-peer send, like UDP multicast.
                        match conn.send_datagram(pkt.clone()).await {
                            Ok(()) => tracing::debug!(target: "overlay", "tx-peer {n}B"),
                            Err(e) => {
                                tracing::debug!(target: "overlay", "tx-peer dropped: {e}")
                            }
                        }
                    }
                }
                // Not an IP packet we understand (runt / unknown version): drop.
                None => {
                    tracing::debug!(target: "overlay", "drop {n}B non-IP frame off TUN");
                }
            }
        }
    }));

    OverlayHandles { tasks }
}

/// Bridge one overlay TUN to many peers with PRODUCTION demux: multicast is
/// flooded to every peer (babel discovery — the emulation), while each unicast
/// packet is routed to the single peer `router` resolves for it (`LPM(dest) ->
/// next-hop -> conn`). An unroutable unicast dest is dropped, NOT flooded, so a
/// transit node can forward without looping.
///
/// `flood_conns` are the connections multicast fans out to (typically every
/// current peer); `router` owns the unicast decision and can consult live state
/// (FIB + connection map) per packet.
pub fn spawn_overlay_routed<R, W, C, U>(
    mut tun_read: R,
    tun_write: W,
    flood_conns: Vec<C>,
    router: U,
    mtu: usize,
) -> OverlayHandles
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    C: DatagramConn + Clone,
    U: UnicastRouter<C>,
{
    let mut tasks = spawn_inbound(tun_write, &flood_conns);

    tasks.push(tokio::spawn(async move {
        let mut buf = vec![0u8; mtu];
        loop {
            let n = match tun_read.read(&mut buf).await {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) => return Err(EncapError::from(e)),
            };
            let pkt = Bytes::copy_from_slice(&buf[..n]);
            let dest = classify(&pkt);
            tracing::debug!(target: "overlay", "rd-tun {n}B {dest:?}");
            match dest {
                // Multicast (babel Hello) is flooded to every peer — the emulation.
                Some(OverlayDest::Multicast) => {
                    for conn in &flood_conns {
                        if let Err(e) = conn.send_datagram(pkt.clone()).await {
                            tracing::debug!(target: "overlay", "mcast tx dropped: {e}");
                        }
                    }
                }
                // Unicast goes to the ONE peer the router resolves, or is dropped.
                Some(OverlayDest::Unicast) => match router.resolve(&pkt) {
                    Some(conn) => match conn.send_datagram(pkt).await {
                        Ok(()) => tracing::debug!(target: "overlay", "tx-routed {n}B"),
                        Err(e) => tracing::debug!(target: "overlay", "routed tx dropped: {e}"),
                    },
                    None => tracing::debug!(target: "overlay", "unicast drop: unroutable"),
                },
                None => {
                    tracing::debug!(target: "overlay", "drop {n}B non-IP frame off TUN");
                }
            }
        }
    }));

    OverlayHandles { tasks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// A test connection: `send_datagram` pushes onto the peer's inbound queue.
    #[derive(Clone)]
    struct MockConn {
        out: Arc<mpsc::Sender<Bytes>>,
        inb: Arc<Mutex<mpsc::Receiver<Bytes>>>,
    }

    /// A pair of endpoints wired to each other, plus a tap on what each SENDS.
    struct Wire {
        conn: MockConn,
        sent: Arc<Mutex<mpsc::Receiver<Bytes>>>,
    }

    fn wire() -> Wire {
        let (tx, rx) = mpsc::channel::<Bytes>(256);
        let (_peer_tx, peer_rx) = mpsc::channel::<Bytes>(256);
        Wire {
            conn: MockConn {
                out: Arc::new(tx),
                inb: Arc::new(Mutex::new(peer_rx)),
            },
            sent: Arc::new(Mutex::new(rx)),
        }
    }

    #[async_trait::async_trait]
    impl DatagramConn for MockConn {
        async fn send_datagram(&self, packet: Bytes) -> Result<(), EncapError> {
            self.out
                .send(packet)
                .await
                .map_err(|_| EncapError::ConnectionClosed)
        }
        async fn recv_datagram(&self) -> Result<Bytes, EncapError> {
            self.inb
                .lock()
                .await
                .recv()
                .await
                .ok_or(EncapError::ConnectionClosed)
        }
    }

    fn babel_hello() -> Bytes {
        // IPv6 header (40 bytes) destined for ff02::1:6.
        let mut pkt = vec![0u8; 40];
        pkt[0] = 0x60;
        pkt[24..40].copy_from_slice(&crate::tun::mcast::BABEL_MCAST.octets());
        Bytes::from(pkt)
    }

    #[tokio::test]
    async fn multicast_hello_is_replicated_to_every_peer() {
        // One TUN, two peers: a babel Hello read off the TUN must reach BOTH.
        let (tun_write_in, tun_read) = tokio::io::duplex(2048);
        let (_dummy_w, _dummy_r) = tokio::io::duplex(2048);

        let a = wire();
        let b = wire();
        let handles = spawn_overlay(
            tun_read,
            _dummy_w,
            vec![a.conn.clone(), b.conn.clone()],
            1300,
        );

        // Inject a babel Hello into the TUN.
        let hello = babel_hello();
        {
            let mut w = tun_write_in;
            w.write_all(&hello).await.unwrap();
        }

        // Both peers must have received it (multicast emulation).
        let got_a = a.sent.lock().await.recv().await.unwrap();
        let got_b = b.sent.lock().await.recv().await.unwrap();
        assert_eq!(got_a, hello);
        assert_eq!(got_b, hello);

        handles.abort();
    }

    fn ipv4_unicast(dst: &str) -> Bytes {
        // Minimal IPv4 header (20 bytes) to a unicast destination.
        let mut pkt = vec![0u8; 20];
        pkt[0] = 0x45; // version 4, IHL 5
        let d: std::net::Ipv4Addr = dst.parse().unwrap();
        pkt[16..20].copy_from_slice(&d.octets());
        Bytes::from(pkt)
    }

    #[tokio::test]
    async fn routed_unicast_reaches_only_the_resolved_peer() {
        let (tun_write_in, tun_read) = tokio::io::duplex(2048);
        let (dummy_w, _dummy_r) = tokio::io::duplex(2048);

        let a = wire();
        let b = wire();
        // Router sends every unicast to peer B only.
        let b_conn = b.conn.clone();
        let router = move |_pkt: &[u8]| Some(b_conn.clone());
        let handles = spawn_overlay_routed(
            tun_read,
            dummy_w,
            vec![a.conn.clone(), b.conn.clone()],
            router,
            1300,
        );

        let pkt = ipv4_unicast("10.42.1.5");
        {
            let mut w = tun_write_in;
            w.write_all(&pkt).await.unwrap();
        }

        // B receives it; A does NOT (unicast is routed, not flooded).
        let got_b = b.sent.lock().await.recv().await.unwrap();
        assert_eq!(got_b, pkt);
        assert!(
            a.sent.lock().await.try_recv().is_err(),
            "unicast must not reach the unrelated peer"
        );
        handles.abort();
    }

    #[tokio::test]
    async fn routed_multicast_still_floods_all_peers() {
        let (tun_write_in, tun_read) = tokio::io::duplex(2048);
        let (dummy_w, _dummy_r) = tokio::io::duplex(2048);

        let a = wire();
        let b = wire();
        // Router would send unicast nowhere, but multicast must ignore it.
        let router = move |_pkt: &[u8]| None::<MockConn>;
        let handles = spawn_overlay_routed(
            tun_read,
            dummy_w,
            vec![a.conn.clone(), b.conn.clone()],
            router,
            1300,
        );

        let hello = babel_hello();
        {
            let mut w = tun_write_in;
            w.write_all(&hello).await.unwrap();
        }
        assert_eq!(a.sent.lock().await.recv().await.unwrap(), hello);
        assert_eq!(b.sent.lock().await.recv().await.unwrap(), hello);
        handles.abort();
    }

    #[tokio::test]
    async fn routed_unroutable_unicast_is_dropped_not_flooded() {
        let (tun_write_in, tun_read) = tokio::io::duplex(2048);
        let (dummy_w, _dummy_r) = tokio::io::duplex(2048);

        let a = wire();
        // Router drops all unicast (None).
        let router = move |_pkt: &[u8]| None::<MockConn>;
        let handles = spawn_overlay_routed(tun_read, dummy_w, vec![a.conn.clone()], router, 1300);

        // Send an unroutable unicast — it must be dropped, NOT flooded to A.
        let unicast = ipv4_unicast("10.42.9.9");
        {
            let mut w = tun_write_in;
            w.write_all(&unicast).await.unwrap();
        }
        // Nothing should ever reach A: a bounded wait must time out (empty).
        let mut sent = a.sent.lock().await;
        let got = tokio::time::timeout(std::time::Duration::from_millis(250), sent.recv()).await;
        assert!(
            got.is_err(),
            "unroutable unicast must be dropped, not forwarded to any peer"
        );
        handles.abort();
    }

    #[tokio::test]
    async fn inbound_from_any_peer_reaches_the_tun() {
        // A datagram arriving on a peer connection must be written to the TUN.
        let (tun_write, mut tun_out) = tokio::io::duplex(2048);
        let (dummy_w, dummy_r) = tokio::io::duplex(2048);
        drop(dummy_w);

        let a = wire();
        // Prime the peer->us direction: replace the receiver with one we feed.
        let (feed_tx, feed_rx) = mpsc::channel::<Bytes>(8);
        let conn = MockConn {
            out: a.conn.out.clone(),
            inb: Arc::new(Mutex::new(feed_rx)),
        };

        let handles = spawn_overlay(dummy_r, tun_write, vec![conn], 1300);

        let hello = babel_hello();
        feed_tx.send(hello.clone()).await.unwrap();

        let mut buf = vec![0u8; hello.len()];
        tokio::io::AsyncReadExt::read_exact(&mut tun_out, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, hello.as_ref());

        handles.abort();
    }

    #[tokio::test]
    async fn task_count_is_reader_writer_plus_peers() {
        let (_w, r) = tokio::io::duplex(64);
        let (w2, _r2) = tokio::io::duplex(64);
        let handles = spawn_overlay(r, w2, vec![wire().conn, wire().conn, wire().conn], 1300);
        // 1 reader + 1 writer + 3 receivers.
        assert_eq!(handles.task_count(), 5);
        handles.abort();
    }
}
