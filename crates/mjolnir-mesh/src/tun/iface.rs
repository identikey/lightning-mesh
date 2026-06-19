use std::net::Ipv4Addr;

use crate::tun::encap::DatagramConn;
#[cfg(target_os = "linux")]
use crate::tun::encap::{spawn_encap_pair, EncapHandles};

/// MTU for tunnel TUN devices. Kept under the QUIC datagram limit so one IP
/// packet fits in one iroh datagram (otherwise the encap drops it as
/// `EncapError::DatagramTooLarge`). Tunable; conservative for typical paths.
pub const TUNNEL_MTU: u16 = 1300;

/// A live per-peer L3 tunnel: a TUN interface plus the two encap tasks bridging
/// it to an iroh connection. Aborting the tasks (on drop) releases the TUN fd,
/// so the kernel removes the interface — no explicit teardown needed.
pub struct Tunnel {
    pub iface_name: String,
    pub self_addr: Ipv4Addr,
    pub peer_addr: Ipv4Addr,
    #[cfg(target_os = "linux")]
    handles: EncapHandles,
}

#[cfg(target_os = "linux")]
impl Drop for Tunnel {
    fn drop(&mut self) {
        self.handles.abort();
    }
}

/// Bring up a per-peer /31 TUN interface and bridge it to `conn` (an iroh
/// connection, via the [`DatagramConn`] seam). Returns once the interface is up
/// and the encap loops are running; the [`Tunnel`] keeps them alive until dropped.
#[cfg(target_os = "linux")]
pub async fn spawn_tunnel<C>(
    peer_short_id: &str,
    self_addr: Ipv4Addr,
    peer_addr: Ipv4Addr,
    conn: C,
) -> Result<Tunnel, IfaceError>
where
    C: DatagramConn + Clone,
{
    use futures_util::stream::TryStreamExt;
    use rtnetlink::new_connection;

    let raw_name = format!("mj-peer-{peer_short_id}");
    let iface_name: String = raw_name.chars().take(15).collect();

    // 1. Create the async TUN device (retained for the tunnel's lifetime), MTU set.
    let mut config = tun::Configuration::default();
    config.name(&iface_name).mtu(TUNNEL_MTU).up();
    let device = tun::create_as_async(&config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    // 2. Assign self_addr/31 and bring the link up via rtnetlink.
    let (connection, handle, _) = new_connection()?;
    tokio::spawn(connection);
    let mut links = handle.link().get().match_name(iface_name.clone()).execute();
    let link = links.try_next().await?.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("interface {iface_name} not found after creation"),
        )
    })?;
    let index = link.header.index;
    handle
        .address()
        .add(index, std::net::IpAddr::V4(self_addr), 31)
        .execute()
        .await?;
    handle
        .link()
        .set(rtnetlink::LinkUnspec::new_with_index(index).up().build())
        .execute()
        .await?;

    // 3. Split the device and wire each half to the iroh connection.
    //    split() returns (writer, reader); encap wants (read, write).
    let (writer, reader) = device.split().map_err(IfaceError::Io)?;
    let handles = spawn_encap_pair(reader, writer, conn, TUNNEL_MTU as usize);

    Ok(Tunnel {
        iface_name,
        self_addr,
        peer_addr,
        handles,
    })
}

#[cfg(not(target_os = "linux"))]
pub async fn spawn_tunnel<C>(
    _peer_short_id: &str,
    _self_addr: Ipv4Addr,
    _peer_addr: Ipv4Addr,
    _conn: C,
) -> Result<Tunnel, IfaceError>
where
    C: DatagramConn + Clone,
{
    Err(IfaceError::Unsupported)
}

/// A live per-peer TUN interface. Drops the interface on `close()`.
pub struct PeerInterface {
    name: String,
    self_addr: Ipv4Addr,
    peer_addr: Ipv4Addr,
}

impl PeerInterface {
    /// Create the TUN interface, assign the /31, bring link up.
    ///
    /// `peer_short_id` is a short hex prefix of the peer's iroh NodeId
    /// (8 chars is enough for uniqueness in any realistic mesh).
    ///
    /// Linux-only for the MVP. On other platforms, returns
    /// `Err(IfaceError::Unsupported)` with a clear message.
    #[cfg(target_os = "linux")]
    pub async fn create(
        peer_short_id: &str,
        self_addr: Ipv4Addr,
        peer_addr: Ipv4Addr,
    ) -> Result<Self, IfaceError> {
        use futures_util::stream::TryStreamExt;
        use rtnetlink::new_connection;

        // Build interface name: "mj-peer-<peer_short_id>", truncated to 15 chars
        // (Linux IFNAMSIZ is 16 including the null terminator).
        let raw_name = format!("mj-peer-{peer_short_id}");
        let name: String = raw_name.chars().take(15).collect();

        // 1. Create the TUN device using the `tun` crate.
        let mut config = tun::Configuration::default();
        config.name(&name).up();
        // We create the device but don't need the handle for this bead
        // (US-005 will use it for packet I/O). The device persists in the
        // kernel as long as we hold the tun::Device handle, so we keep it.
        let _device = tun::create(&config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        // 2. Use rtnetlink to assign the /31 address.
        let (connection, handle, _) = new_connection()?;
        tokio::spawn(connection);

        // Find the interface by name to get its index.
        let mut links = handle.link().get().match_name(name.clone()).execute();
        let link = links
            .try_next()
            .await?
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("interface {name} not found after creation")))?;
        let index = link.header.index;

        // Add self_addr/31 to the interface.
        handle
            .address()
            .add(index, std::net::IpAddr::V4(self_addr), 31)
            .execute()
            .await?;

        // 3. Bring the link up.
        handle
            .link()
            .set(rtnetlink::LinkUnspec::new_with_index(index).up().build())
            .execute()
            .await?;

        Ok(Self {
            name,
            self_addr,
            peer_addr,
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn create(
        _peer_short_id: &str,
        _self_addr: Ipv4Addr,
        _peer_addr: Ipv4Addr,
    ) -> Result<Self, IfaceError> {
        Err(IfaceError::Unsupported)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn self_addr(&self) -> Ipv4Addr {
        self.self_addr
    }

    pub fn peer_addr(&self) -> Ipv4Addr {
        self.peer_addr
    }

    /// Tear down the interface. Idempotent: subsequent close() calls are no-ops.
    #[cfg(target_os = "linux")]
    pub async fn close(self) -> Result<(), IfaceError> {
        use futures_util::stream::TryStreamExt;
        use rtnetlink::new_connection;

        let (connection, handle, _) = new_connection()?;
        tokio::spawn(connection);

        let mut links = handle
            .link()
            .get()
            .match_name(self.name.clone())
            .execute();
        if let Some(link) = links.try_next().await? {
            handle.link().del(link.header.index).execute().await?;
        }
        // If not found, it's already gone — idempotent.
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn close(self) -> Result<(), IfaceError> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IfaceError {
    #[cfg(target_os = "linux")]
    #[error("netlink error: {0}")]
    Netlink(#[from] rtnetlink::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported platform — TUN lifecycle is Linux-only")]
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires root or CAP_NET_ADMIN; run with `cargo test -- --ignored`"]
    async fn create_and_destroy_real_tun() {
        let iface = PeerInterface::create(
            "test1234",
            "10.255.1.0".parse().unwrap(),
            "10.255.1.1".parse().unwrap(),
        )
        .await
        .expect("create succeeded");
        assert!(iface.name().starts_with("mj-peer-"));
        iface.close().await.expect("close succeeded");
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn create_unsupported_off_linux() {
        let r = PeerInterface::create(
            "test1234",
            "10.255.1.0".parse().unwrap(),
            "10.255.1.1".parse().unwrap(),
        )
        .await;
        assert!(matches!(r, Err(IfaceError::Unsupported)));
    }
}
