//! The router swarm inventory — `routers.toml`.
//!
//! This is `meshctl`'s declaration of *which* routers exist and *what role*
//! each plays; the per-router desired config (veth/bridge/NAT/container) is
//! derived from the role + subnet, mirroring `deploy/mikrotik/container-net.rsc`.
//!
//! No secrets live here: SSH auth uses the operator's ssh-agent / default key,
//! and `peer_blob` values are *public* shareable address blobs (the same string
//! a peer's `tun-listen` prints). So this file is safe to commit.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// Default location, relative to the repo root.
pub const DEFAULT_PATH: &str = "deploy/mikrotik/routers.toml";

/// What a router does in the mesh. A `listener` runs `tun-listen` (accepts
/// tunnels); a `connector` dials a listener's address blob with `tun-connect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Listener,
    Connector,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Listener => f.write_str("listener"),
            Role::Connector => f.write_str("connector"),
        }
    }
}

/// One router in the swarm.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Router {
    /// Short stable handle, used on the CLI (`meshctl deploy <name>`) and to
    /// resolve a connector's `peer` reference.
    pub name: String,
    /// IP or hostname reachable over SSH (the router's LAN/management address,
    /// NOT the mesh address).
    pub address: String,
    /// SSH user. Falls back to the inventory `default_user` when unset.
    pub user: Option<String>,
    pub role: Role,
    /// Container subnet on this router's `br-mesh`. Falls back to
    /// `default_subnet`. Each router is its own L2 domain, so the same subnet
    /// on every router is fine.
    pub subnet: Option<String>,
    /// Container image tar to upload. Falls back to `default_tar`.
    pub tar: Option<String>,
    /// For a connector: the `name` of the router it should tunnel to. The live
    /// address blob is resolved at deploy time (M4) unless `peer_blob` pins it.
    pub peer: Option<String>,
    /// Static override: a literal address blob to connect to, bypassing live
    /// resolution. Public string, safe to commit.
    pub peer_blob: Option<String>,
}

impl Router {
    /// SSH `user@address` target, resolving the inventory default.
    pub fn ssh_target(&self, inv: &Inventory) -> String {
        format!("{}@{}", self.user(inv), self.address)
    }

    pub fn user<'a>(&'a self, inv: &'a Inventory) -> &'a str {
        self.user.as_deref().unwrap_or(&inv.default_user)
    }

    pub fn subnet<'a>(&'a self, inv: &'a Inventory) -> &'a str {
        self.subnet.as_deref().unwrap_or(&inv.default_subnet)
    }

    pub fn tar<'a>(&'a self, inv: &'a Inventory) -> Option<&'a str> {
        self.tar.as_deref().or(inv.default_tar.as_deref())
    }
}

fn default_user() -> String {
    "admin".to_string()
}

fn default_subnet() -> String {
    // Matches deploy/mikrotik/container-net.rsc.
    "172.20.0.0/24".to_string()
}

/// The whole swarm.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    #[serde(default = "default_user")]
    pub default_user: String,
    #[serde(default = "default_subnet")]
    pub default_subnet: String,
    #[serde(default)]
    pub default_tar: Option<String>,
    /// `[[router]]` tables.
    #[serde(rename = "router", default)]
    pub routers: Vec<Router>,
}

impl Inventory {
    /// Parse and validate an inventory from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading inventory {}", path.display()))?;
        let inv: Inventory =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        inv.validate()?;
        Ok(inv)
    }

    /// Look up a router by name.
    pub fn get(&self, name: &str) -> Option<&Router> {
        self.routers.iter().find(|r| r.name == name)
    }

    /// Resolve a connector's target router by its `peer` reference, if any.
    pub fn peer_of(&self, router: &Router) -> Option<&Router> {
        router.peer.as_deref().and_then(|p| self.get(p))
    }

    /// Structural checks that are cheap to catch early: unique names, and every
    /// connector's `peer` reference (when present) points at a router that
    /// exists. A connector with neither `peer` nor `peer_blob` is allowed here
    /// — deploy (M4) is where that becomes a hard error, since `plan`/`apply`
    /// don't need it.
    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for r in &self.routers {
            if !seen.insert(r.name.as_str()) {
                bail!("duplicate router name in inventory: {}", r.name);
            }
        }
        for r in &self.routers {
            if let Some(peer) = &r.peer {
                if self.get(peer).is_none() {
                    bail!(
                        "router {}: peer {:?} is not defined in the inventory",
                        r.name,
                        peer
                    );
                }
                if *peer == r.name {
                    bail!("router {}: peer references itself", r.name);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Inventory> {
        let inv: Inventory = toml::from_str(s)?;
        inv.validate()?;
        Ok(inv)
    }

    #[test]
    fn parses_roles_and_defaults() {
        let inv = parse(
            r#"
            default_tar = "deploy/mikrotik/mjolnir-meshd-ros.tar"

            [[router]]
            name = "r1"
            address = "192.168.88.181"
            role = "listener"

            [[router]]
            name = "r2"
            address = "192.168.88.113"
            user = "ops"
            role = "connector"
            peer = "r1"
            "#,
        )
        .unwrap();

        let r1 = inv.get("r1").unwrap();
        assert_eq!(r1.role, Role::Listener);
        // default_user fallback
        assert_eq!(r1.user(&inv), "admin");
        // default_subnet fallback matches container-net.rsc
        assert_eq!(r1.subnet(&inv), "172.20.0.0/24");
        // default_tar fallback
        assert_eq!(r1.tar(&inv), Some("deploy/mikrotik/mjolnir-meshd-ros.tar"));
        assert_eq!(r1.ssh_target(&inv), "admin@192.168.88.181");

        let r2 = inv.get("r2").unwrap();
        assert_eq!(r2.role, Role::Connector);
        assert_eq!(r2.user(&inv), "ops");
        assert_eq!(inv.peer_of(r2).unwrap().name, "r1");
    }

    #[test]
    fn rejects_duplicate_names() {
        let err = parse(
            r#"
            [[router]]
            name = "dup"
            address = "10.0.0.1"
            role = "listener"
            [[router]]
            name = "dup"
            address = "10.0.0.2"
            role = "connector"
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate router name"));
    }

    #[test]
    fn rejects_dangling_peer() {
        let err = parse(
            r#"
            [[router]]
            name = "r2"
            address = "10.0.0.2"
            role = "connector"
            peer = "ghost"
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not defined"));
    }
}
