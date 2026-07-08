//! M2/M3: desired-state model, `plan` (observe + diff), and the command
//! fragments `apply` uses to converge.
//!
//! Forge models each resource kind with a behaviour (trait). For RouterOS every
//! kind shares the *same* observe (query a menu path filtered by a `where`
//! expression) and the *same* diff (compare desired property values against the
//! observed record). The only per-kind variation is data — path, identity,
//! desired fields — so instead of a trait-per-kind we describe each resource as
//! a [`Desired`] value and run one uniform observe/classify over them.
//!
//! The desired set mirrors `deploy/mikrotik/container-net.rsc`: for subnet
//! `172.20.0.0/24` the router owns `172.20.0.1` on `br-mesh` and the container
//! gets `172.20.0.2` via `veth-mesh`, with a srcnat masquerade + a forward
//! accept for the subnet, both tagged `comment="mjolnir container egress"`.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use ipnet::Ipv4Net;

use crate::inventory::{Inventory, Router};
use crate::routeros::{self, Record};
use crate::ssh::Ssh;

/// The comment tag marking RouterOS items this tool owns (matches
/// container-net.rsc). Prune only ever touches items carrying a `mjolnir …`
/// comment, so user-owned config is never in scope.
const OWN_COMMENT: &str = "mjolnir container egress";
/// Regex (RouterOS `~`) matching any item we own, for the prune scan.
const OWN_PREFIX_RE: &str = r#"comment~"^mjolnir""#;

/// One desired RouterOS resource: where it lives, the single key/value that
/// uniquely identifies the live instance, and the other property values it
/// should have.
#[derive(Debug, Clone)]
pub struct Desired {
    pub kind: &'static str,
    pub path: &'static str,
    /// Human-facing identity for display.
    pub id: String,
    /// The unique find/add key=value (e.g. `("name","veth-mesh")`). All our
    /// resources are identifiable by a single property.
    pub identity: (&'static str, String),
    /// Other desired property → value, in canonical RouterOS string form.
    pub fields: Vec<(&'static str, String)>,
    /// Whether this kind is owned/identified by the `mjolnir …` comment tag
    /// (firewall rules) — and thus participates in prune.
    pub comment_owned: bool,
}

impl Desired {
    /// RouterOS `where` expression locating the live instance.
    pub fn find(&self) -> String {
        format!(r#"{}="{}""#, self.identity.0, self.identity.1)
    }

    /// Property names to observe (the non-identity desired fields).
    pub fn field_keys(&self) -> Vec<&str> {
        self.fields.iter().map(|(k, _)| *k).collect()
    }

    /// Arguments for `add`: identity + all fields, each quoted (`key="value"`).
    /// Always quoting is safe in RouterOS and handles values with spaces (the
    /// comment).
    pub fn add_args(&self) -> String {
        std::iter::once(&self.identity)
            .chain(self.fields.iter())
            .map(|(k, v)| format!(r#"{k}="{v}""#))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Arguments for `set` (drift correction): the fields only — identity is
    /// used in the `[find where …]` selector. Empty when there are no fields.
    pub fn set_args(&self) -> String {
        self.fields
            .iter()
            .map(|(k, v)| format!(r#"{k}="{v}""#))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Diff outcome for one resource, mirroring Forge's statuses.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// Declared but not present on the router.
    Missing,
    /// Present and every desired field matches.
    Converged,
    /// Present but one or more fields differ.
    Drifted(Vec<FieldDiff>),
    /// More than one live instance matched the find filter (ambiguous).
    Conflict(usize),
}

#[derive(Debug, PartialEq, Eq)]
pub struct FieldDiff {
    pub field: &'static str,
    pub want: String,
    pub got: String,
}

/// A desired resource paired with its observed status.
pub struct Observed {
    pub desired: Desired,
    pub status: Status,
}

/// A mjolnir-tagged item on the router that no desired resource claims — a
/// leftover for `apply` to remove.
pub struct PruneEntry {
    pub kind: &'static str,
    pub path: &'static str,
    pub comment: String,
}

/// Build the desired resource set for a router from its subnet, mirroring
/// container-net.rsc. Gateway (router) = network+1, container = network+2.
pub fn desired_resources(inv: &Inventory, r: &Router) -> Result<Vec<Desired>> {
    let subnet = r.subnet(inv);
    let net: Ipv4Net = subnet
        .parse()
        .with_context(|| format!("router {}: invalid subnet {subnet:?}", r.name))?;
    let base = u32::from(net.network());
    let plen = net.prefix_len();
    let gw = Ipv4Addr::from(base + 1); // router side on br-mesh
    let ct = Ipv4Addr::from(base + 2); // container side on veth-mesh
    let veth_addr = format!("{ct}/{plen}");
    let gw_cidr = format!("{gw}/{plen}");
    let src = format!("{}/{plen}", net.network());

    Ok(vec![
        Desired {
            kind: "veth",
            path: "/interface/veth",
            id: "veth-mesh".into(),
            identity: ("name", "veth-mesh".into()),
            fields: vec![("address", veth_addr), ("gateway", gw.to_string())],
            comment_owned: false,
        },
        Desired {
            kind: "bridge",
            path: "/interface/bridge",
            id: "br-mesh".into(),
            identity: ("name", "br-mesh".into()),
            fields: vec![], // existence only
            comment_owned: false,
        },
        Desired {
            kind: "bridge-port",
            path: "/interface/bridge/port",
            id: "veth-mesh@br-mesh".into(),
            identity: ("interface", "veth-mesh".into()),
            fields: vec![("bridge", "br-mesh".into())],
            comment_owned: false,
        },
        Desired {
            kind: "ip-address",
            path: "/ip/address",
            id: format!("{gw_cidr}@br-mesh"),
            identity: ("address", gw_cidr),
            fields: vec![("interface", "br-mesh".into())],
            comment_owned: false,
        },
        Desired {
            kind: "nat",
            path: "/ip/firewall/nat",
            id: OWN_COMMENT.into(),
            identity: ("comment", OWN_COMMENT.into()),
            fields: vec![
                ("chain", "srcnat".into()),
                ("action", "masquerade".into()),
                ("src-address", src.clone()),
            ],
            comment_owned: true,
        },
        Desired {
            kind: "filter",
            path: "/ip/firewall/filter",
            id: OWN_COMMENT.into(),
            identity: ("comment", OWN_COMMENT.into()),
            fields: vec![
                ("chain", "forward".into()),
                ("action", "accept".into()),
                ("src-address", src),
            ],
            comment_owned: true,
        },
    ])
}

/// Classify a desired resource against the records the find filter matched.
/// Pure (no I/O) so it can be unit-tested.
pub fn classify(fields: &[(&'static str, String)], recs: &[Record]) -> Status {
    match recs {
        [] => Status::Missing,
        [obs] => {
            let diffs: Vec<FieldDiff> = fields
                .iter()
                .filter_map(|(k, want)| {
                    let got = obs.get(*k).cloned().unwrap_or_default();
                    (got != *want).then(|| FieldDiff {
                        field: k,
                        want: want.clone(),
                        got,
                    })
                })
                .collect();
            if diffs.is_empty() {
                Status::Converged
            } else {
                Status::Drifted(diffs)
            }
        }
        many => Status::Conflict(many.len()),
    }
}

/// Observe + diff every desired resource on `r`, plus a prune scan over the
/// comment-owned paths. No mutation. Shared by `plan` and `apply`.
pub async fn observe_router(
    ssh: &Ssh,
    inv: &Inventory,
    r: &Router,
) -> Result<(Vec<Observed>, Vec<PruneEntry>)> {
    let desired = desired_resources(inv, r)?;

    let mut observed = Vec::with_capacity(desired.len());
    for d in desired.iter() {
        let recs = routeros::query(ssh, d.path, Some(&d.find()), &d.field_keys())
            .await
            .with_context(|| format!("observing {} {}", d.kind, d.id))?;
        let status = classify(&d.fields, &recs);
        observed.push(Observed {
            desired: d.clone(),
            status,
        });
    }

    // Prune: for each comment-owned path, find mjolnir-tagged items whose
    // comment no desired resource claims.
    let mut prunes = Vec::new();
    let owned_paths: BTreeSet<&str> = desired
        .iter()
        .filter(|d| d.comment_owned)
        .map(|d| d.path)
        .collect();
    for path in owned_paths {
        let want: BTreeSet<&str> = desired
            .iter()
            .filter(|d| d.path == path)
            .map(|d| d.identity.1.as_str())
            .collect();
        let kind = desired
            .iter()
            .find(|d| d.path == path)
            .map(|d| d.kind)
            .unwrap_or("?");
        let recs = routeros::query(ssh, path, Some(OWN_PREFIX_RE), &["comment"])
            .await
            .with_context(|| format!("prune scan on {path}"))?;
        for rec in recs {
            if let Some(c) = rec.get("comment")
                && !want.contains(c.as_str())
            {
                prunes.push(PruneEntry {
                    kind,
                    path,
                    comment: c.clone(),
                });
            }
        }
    }

    Ok((observed, prunes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(pairs: &[(&str, &str)]) -> Record {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn desired_set_matches_container_net_rsc() {
        let inv: Inventory = toml::from_str(
            r#"
            [[router]]
            name = "r1"
            address = "10.0.0.1"
            role = "listener"
            "#,
        )
        .unwrap();
        let r = inv.get("r1").unwrap();
        let d = desired_resources(&inv, r).unwrap();

        let veth = d.iter().find(|x| x.kind == "veth").unwrap();
        assert_eq!(veth.identity, ("name", "veth-mesh".to_string()));
        assert_eq!(
            veth.fields,
            vec![
                ("address", "172.20.0.2/24".to_string()),
                ("gateway", "172.20.0.1".to_string()),
            ]
        );
        let ip = d.iter().find(|x| x.kind == "ip-address").unwrap();
        assert_eq!(ip.identity, ("address", "172.20.0.1/24".to_string()));
        let nat = d.iter().find(|x| x.kind == "nat").unwrap();
        assert!(
            nat.fields
                .contains(&("src-address", "172.20.0.0/24".to_string()))
        );
        assert!(nat.comment_owned);
    }

    #[test]
    fn add_and_set_args() {
        let inv: Inventory =
            toml::from_str("[[router]]\nname='r1'\naddress='10.0.0.1'\nrole='listener'\n").unwrap();
        let d = desired_resources(&inv, inv.get("r1").unwrap()).unwrap();

        let veth = d.iter().find(|x| x.kind == "veth").unwrap();
        assert_eq!(veth.find(), r#"name="veth-mesh""#);
        assert_eq!(
            veth.add_args(),
            r#"name="veth-mesh" address="172.20.0.2/24" gateway="172.20.0.1""#
        );
        assert_eq!(
            veth.set_args(),
            r#"address="172.20.0.2/24" gateway="172.20.0.1""#
        );

        // comment with spaces must round-trip quoted.
        let nat = d.iter().find(|x| x.kind == "nat").unwrap();
        assert!(
            nat.add_args()
                .contains(r#"comment="mjolnir container egress""#)
        );

        // bridge: existence-only → empty set args (no `set` ever needed).
        let bridge = d.iter().find(|x| x.kind == "bridge").unwrap();
        assert_eq!(bridge.set_args(), "");
        assert_eq!(bridge.add_args(), r#"name="br-mesh""#);
    }

    #[test]
    fn classify_missing_converged_drifted_conflict() {
        let fields = vec![("bridge", "br-mesh".to_string())];

        assert_eq!(classify(&fields, &[]), Status::Missing);
        assert_eq!(
            classify(&fields, &[rec(&[("bridge", "br-mesh")])]),
            Status::Converged
        );
        match classify(&fields, &[rec(&[("bridge", "other")])]) {
            Status::Drifted(d) => {
                assert_eq!(d[0].field, "bridge");
                assert_eq!(d[0].want, "br-mesh");
                assert_eq!(d[0].got, "other");
            }
            s => panic!("expected Drifted, got {s:?}"),
        }
        assert_eq!(
            classify(
                &fields,
                &[rec(&[("bridge", "br-mesh")]), rec(&[("bridge", "br-mesh")])]
            ),
            Status::Conflict(2)
        );
    }

    #[test]
    fn classify_empty_fields_is_existence_only() {
        assert_eq!(classify(&[], &[rec(&[])]), Status::Converged);
        assert_eq!(classify(&[], &[]), Status::Missing);
    }
}
