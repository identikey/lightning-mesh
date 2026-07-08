//! M3: `apply` — converge a router to its desired state.
//!
//! Apply is driven by a fresh observe: Rust decides per-resource what to do
//! (RouterOS just executes the `add`/`set`/`remove`), which keeps reporting
//! accurate and avoids the `:if/else` snippet gymnastics. Because the decision
//! comes from the live `plan`, re-running apply is idempotent — a converged
//! resource yields no change.
//!
//! Ownership: every managed item carries `comment="mjolnir …"` (the firewall
//! rules) or a fixed mesh name (`veth-mesh`/`br-mesh`); prune only removes
//! mjolnir-comment-tagged leftovers, so untagged user config is never touched.
//! Resources are emitted in dependency order (veth → bridge → port → ip → nat →
//! filter), so e.g. the bridge exists before its port is added.

use anyhow::Result;

use crate::inventory::{Inventory, Router};
use crate::plan::{self, Status};
use crate::routeros;
use crate::ssh::Ssh;

/// A single converging mutation to run on the router.
pub struct Change {
    pub kind: &'static str,
    pub id: String,
    pub verb: &'static str, // "add" | "set" | "remove"
    pub cmd: String,        // full RouterOS command
    pub reason: String,     // why this change is needed
}

/// Something observe surfaced that apply deliberately will NOT change.
pub struct Skipped {
    pub kind: &'static str,
    pub id: String,
    pub why: String,
}

/// Compute the changes needed to converge `r`, from a fresh observe. Order
/// follows the desired list (dependency order) with prunes last.
pub async fn plan_changes(
    ssh: &Ssh,
    inv: &Inventory,
    r: &Router,
) -> Result<(Vec<Change>, Vec<Skipped>)> {
    let (observed, prunes) = plan::observe_router(ssh, inv, r).await?;

    let mut changes = Vec::new();
    let mut skipped = Vec::new();

    for o in &observed {
        let d = &o.desired;
        match &o.status {
            Status::Missing => changes.push(Change {
                kind: d.kind,
                id: d.id.clone(),
                verb: "add",
                cmd: format!("{}/add {}", d.path, d.add_args()),
                reason: "missing".into(),
            }),
            Status::Drifted(diffs) => {
                let fields = diffs.iter().map(|x| x.field).collect::<Vec<_>>().join(",");
                changes.push(Change {
                    kind: d.kind,
                    id: d.id.clone(),
                    verb: "set",
                    cmd: format!("{}/set [find where {}] {}", d.path, d.find(), d.set_args()),
                    reason: format!("drift: {fields}"),
                });
            }
            Status::Converged => {}
            Status::Conflict(n) => skipped.push(Skipped {
                kind: d.kind,
                id: d.id.clone(),
                why: format!("{n} live matches — ambiguous, resolve by hand"),
            }),
        }
    }

    for p in &prunes {
        changes.push(Change {
            kind: p.kind,
            id: p.comment.clone(),
            verb: "remove",
            cmd: format!(r#"{}/remove [find where comment="{}"]"#, p.path, p.comment),
            reason: "prune leftover".into(),
        });
    }

    Ok((changes, skipped))
}

/// Execute one change on the router, verifying it completed (RouterOS prints
/// errors to stdout and exits 0 — see [`routeros::run_command`]).
pub async fn run_change(ssh: &Ssh, c: &Change) -> Result<()> {
    routeros::run_command(ssh, &c.cmd).await?;
    Ok(())
}
