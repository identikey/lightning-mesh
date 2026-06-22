//! M2: desired-state model + `plan` (observe + diff, no mutation).
//!
//! Forge models each resource kind with a behaviour (trait). For RouterOS every
//! kind shares the *same* observe (query a menu path filtered by a `where`
//! expression) and the *same* diff (compare desired property values against the
//! observed record). The only per-kind variation is data — path, identity, find
//! filter, desired fields — so instead of a trait-per-kind we describe each
//! resource as a [`Desired`] value and run one uniform classify/observe over
//! them. Same behaviour as Forge's trait, without the boilerplate.
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

/// One desired RouterOS resource: where it lives, how to find the live instance,
/// and the property values it should have.
#[derive(Debug, Clone)]
pub struct Desired {
    pub kind: &'static str,
    pub path: &'static str,
    /// Human-facing identity (also the comment, for comment-owned kinds).
    pub id: String,
    /// RouterOS `where` expression locating the live instance.
    pub find: String,
    /// Desired property → value, in canonical RouterOS string form.
    pub fields: Vec<(&'static str, String)>,
    /// Whether this kind is owned/identified by the `mjolnir …` comment tag
    /// (firewall rules) — and thus participates in prune.
    pub comment_owned: bool,
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

pub struct PlanEntry {
    pub kind: &'static str,
    pub id: String,
    pub status: Status,
}

/// A mjolnir-tagged item on the router that no desired resource claims — a
/// leftover to be removed by `apply` (M3).
pub struct PruneEntry {
    pub kind: &'static str,
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
            find: r#"name="veth-mesh""#.into(),
            fields: vec![("address", veth_addr), ("gateway", gw.to_string())],
            comment_owned: false,
        },
        Desired {
            kind: "bridge",
            path: "/interface/bridge",
            id: "br-mesh".into(),
            find: r#"name="br-mesh""#.into(),
            fields: vec![], // existence only
            comment_owned: false,
        },
        Desired {
            kind: "bridge-port",
            path: "/interface/bridge/port",
            id: "veth-mesh@br-mesh".into(),
            find: r#"interface="veth-mesh""#.into(),
            fields: vec![("bridge", "br-mesh".into())],
            comment_owned: false,
        },
        Desired {
            kind: "ip-address",
            path: "/ip/address",
            id: format!("{gw_cidr}@br-mesh"),
            find: format!(r#"address="{gw_cidr}""#),
            fields: vec![("interface", "br-mesh".into())],
            comment_owned: false,
        },
        Desired {
            kind: "nat",
            path: "/ip/firewall/nat",
            id: OWN_COMMENT.into(),
            find: format!(r#"comment="{OWN_COMMENT}""#),
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
            find: format!(r#"comment="{OWN_COMMENT}""#),
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
/// comment-owned paths. No mutation.
pub async fn plan_router(
    ssh: &Ssh,
    inv: &Inventory,
    r: &Router,
) -> Result<(Vec<PlanEntry>, Vec<PruneEntry>)> {
    let desired = desired_resources(inv, r)?;

    let mut entries = Vec::with_capacity(desired.len());
    for d in &desired {
        let want_fields: Vec<&str> = d.fields.iter().map(|(k, _)| *k).collect();
        let recs = routeros::query(ssh, d.path, Some(&d.find), &want_fields)
            .await
            .with_context(|| format!("observing {} {}", d.kind, d.id))?;
        entries.push(PlanEntry {
            kind: d.kind,
            id: d.id.clone(),
            status: classify(&d.fields, &recs),
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
            .map(|d| d.id.as_str())
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
            if let Some(c) = rec.get("comment") {
                if !want.contains(c.as_str()) {
                    prunes.push(PruneEntry {
                        kind,
                        comment: c.clone(),
                    });
                }
            }
        }
    }

    Ok((entries, prunes))
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
        assert_eq!(veth.fields, vec![
            ("address", "172.20.0.2/24".to_string()),
            ("gateway", "172.20.0.1".to_string()),
        ]);
        let ip = d.iter().find(|x| x.kind == "ip-address").unwrap();
        assert_eq!(ip.id, "172.20.0.1/24@br-mesh");
        let nat = d.iter().find(|x| x.kind == "nat").unwrap();
        assert!(nat
            .fields
            .contains(&("src-address", "172.20.0.0/24".to_string())));
        assert!(nat.comment_owned);
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
            classify(&fields, &[rec(&[("bridge", "br-mesh")]), rec(&[("bridge", "br-mesh")])]),
            Status::Conflict(2)
        );
    }

    #[test]
    fn classify_empty_fields_is_existence_only() {
        // bridge: no fields → present means Converged regardless of properties.
        assert_eq!(classify(&[], &[rec(&[])]), Status::Converged);
        assert_eq!(classify(&[], &[]), Status::Missing);
    }
}
