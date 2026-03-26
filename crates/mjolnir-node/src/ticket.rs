use anyhow::{bail, Context, Result};
use iroh::EndpointAddr;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A ticket for joining a mesh room: `name@base32(postcard(endpoint_addr))`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshTicket {
    pub name: String,
    pub addr: EndpointAddr,
}

impl fmt::Display for MeshTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let addr_bytes =
            postcard::to_allocvec(&self.addr).expect("failed to serialize endpoint addr");
        let encoded = data_encoding::BASE32_NOPAD
            .encode(&addr_bytes)
            .to_lowercase();
        write!(f, "{}@{}", self.name, encoded)
    }
}

impl FromStr for MeshTicket {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (name, addr_part) = s.split_once('@').context("ticket must contain '@'")?;

        if name.is_empty() {
            bail!("room name cannot be empty");
        }

        let addr_bytes = data_encoding::BASE32_NOPAD
            .decode(addr_part.to_uppercase().as_bytes())
            .context("invalid base32 in ticket")?;

        let addr: EndpointAddr =
            postcard::from_bytes(&addr_bytes).context("invalid endpoint address in ticket")?;

        Ok(Self {
            name: name.to_string(),
            addr,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_roundtrip() {
        let id = iroh::SecretKey::generate(&mut rand::rng()).public();
        let ticket = MeshTicket {
            name: "test-room".into(),
            addr: EndpointAddr::new(id),
        };

        let s = ticket.to_string();
        assert!(s.starts_with("test-room@"));

        let parsed: MeshTicket = s.parse().unwrap();
        assert_eq!(parsed.name, "test-room");
        assert_eq!(parsed.addr.id, id);
    }
}
