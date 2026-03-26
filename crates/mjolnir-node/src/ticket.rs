use anyhow::{bail, Context, Result};
use iroh::EndpointAddr;
use iroh_gossip::proto::TopicId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A ticket for joining a mesh room: `name@base32(postcard(addrs + topic_id))`
///
/// Contains one or more peer addresses as bootstrap entry points.
/// Any live peer in the room can mint a ticket — the joiner succeeds
/// if *any* of the addresses is reachable.
///
/// **DHT opportunity:** With a DHT, peers could publish their address under
/// the topic_id key, eliminating the need to embed addresses in tickets
/// entirely. A ticket could shrink to just `name` (since topic_id is
/// deterministic from name), with the DHT providing bootstrap addresses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshTicket {
    pub name: String,
    pub addrs: Vec<EndpointAddr>,
    pub topic_id: [u8; 32],
}

impl MeshTicket {
    /// Create a ticket with a single bootstrap address.
    pub fn new(name: String, addr: EndpointAddr) -> Self {
        let topic_id = Self::topic_id_from_name(&name);
        Self {
            name,
            addrs: vec![addr],
            topic_id,
        }
    }

    /// Create a ticket with multiple bootstrap addresses (more resilient).
    pub fn with_peers(name: String, addrs: Vec<EndpointAddr>) -> Self {
        assert!(!addrs.is_empty(), "ticket must have at least one address");
        let topic_id = Self::topic_id_from_name(&name);
        Self {
            name,
            addrs,
            topic_id,
        }
    }

    /// Derive a deterministic topic ID from a room name.
    pub fn topic_id_from_name(name: &str) -> [u8; 32] {
        blake3::hash(name.as_bytes()).into()
    }

    /// Get the gossip TopicId.
    pub fn gossip_topic_id(&self) -> TopicId {
        TopicId::from_bytes(self.topic_id)
    }

    /// All EndpointIds from the ticket's addresses, for gossip bootstrap.
    pub fn bootstrap_peer_ids(&self) -> Vec<iroh::EndpointId> {
        self.addrs.iter().map(|a| a.id).collect()
    }
}

impl fmt::Display for MeshTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ticket_bytes = postcard::to_allocvec(&(&self.addrs, &self.topic_id))
            .expect("failed to serialize ticket");
        let encoded = data_encoding::BASE32_NOPAD
            .encode(&ticket_bytes)
            .to_lowercase();
        write!(f, "{}@{}", self.name, encoded)
    }
}

impl FromStr for MeshTicket {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (name, encoded_part) = s.split_once('@').context("ticket must contain '@'")?;

        if name.is_empty() {
            bail!("room name cannot be empty");
        }

        let ticket_bytes = data_encoding::BASE32_NOPAD
            .decode(encoded_part.to_uppercase().as_bytes())
            .context("invalid base32 in ticket")?;

        let (addrs, topic_id): (Vec<EndpointAddr>, [u8; 32]) =
            postcard::from_bytes(&ticket_bytes).context("invalid ticket payload")?;

        if addrs.is_empty() {
            bail!("ticket must contain at least one address");
        }

        Ok(Self {
            name: name.to_string(),
            addrs,
            topic_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_roundtrip_single_addr() {
        let id = iroh::SecretKey::generate(&mut rand::rng()).public();
        let ticket = MeshTicket::new("test-room".into(), EndpointAddr::new(id));

        let s = ticket.to_string();
        assert!(s.starts_with("test-room@"));

        let parsed: MeshTicket = s.parse().unwrap();
        assert_eq!(parsed.name, "test-room");
        assert_eq!(parsed.addrs.len(), 1);
        assert_eq!(parsed.addrs[0].id, id);
        assert_eq!(parsed.topic_id, MeshTicket::topic_id_from_name("test-room"));
    }

    #[test]
    fn ticket_roundtrip_multi_addr() {
        let id_a = iroh::SecretKey::generate(&mut rand::rng()).public();
        let id_b = iroh::SecretKey::generate(&mut rand::rng()).public();
        let addrs = vec![EndpointAddr::new(id_a), EndpointAddr::new(id_b)];
        let ticket = MeshTicket::with_peers("multi-room".into(), addrs);

        let s = ticket.to_string();
        let parsed: MeshTicket = s.parse().unwrap();
        assert_eq!(parsed.name, "multi-room");
        assert_eq!(parsed.addrs.len(), 2);
        assert_eq!(parsed.addrs[0].id, id_a);
        assert_eq!(parsed.addrs[1].id, id_b);
    }

    #[test]
    fn topic_id_deterministic() {
        let a = MeshTicket::topic_id_from_name("my-room");
        let b = MeshTicket::topic_id_from_name("my-room");
        assert_eq!(a, b);

        let c = MeshTicket::topic_id_from_name("other-room");
        assert_ne!(a, c);
    }

    #[test]
    fn bootstrap_peer_ids() {
        let id_a = iroh::SecretKey::generate(&mut rand::rng()).public();
        let id_b = iroh::SecretKey::generate(&mut rand::rng()).public();
        let ticket = MeshTicket::with_peers(
            "room".into(),
            vec![EndpointAddr::new(id_a), EndpointAddr::new(id_b)],
        );
        let ids = ticket.bootstrap_peer_ids();
        assert_eq!(ids, vec![id_a, id_b]);
    }
}
