//! RouterOS query layer — the "observe" half of the SSH-only reconciler.
//!
//! RouterOS has no clean JSON-over-SSH; its `print as-value` returns arrays with
//! type quirks (see MikroTik scripting docs — run values through `:tostr`). So
//! instead of parsing RouterOS's serialization we generate a script that emits
//! *our own* format: one line per record, each a `MCTL>`-prefixed list of
//! `key=value` pairs joined by a `~|~` delimiter, every value coerced with
//! `:tostr`. We control the format end-to-end, so parsing is unambiguous and any
//! login banner / prompt noise is ignored (only `MCTL>` lines are parsed).
//!
//! Example generated script (path `/interface/veth`, fields name+address):
//!
//! ```text
//! :foreach _i in=[/interface/veth/find] do={:put ("MCTL>" . \
//!   "name=" . [:tostr [/interface/veth/get $_i name]] . "~|~" . \
//!   "address=" . [:tostr [/interface/veth/get $_i address]])}
//! ```
//!
//! Caveat: a value containing the literal `~|~` would mis-split. The fields we
//! read are RouterOS config (names, addresses, our own `mjolnir …` comments),
//! so this can't occur in practice; documented here rather than escaped.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use crate::ssh::Ssh;

/// Line prefix marking a record emitted by our query script.
const REC_PREFIX: &str = "MCTL>";
/// Field separator within a record line.
const FIELD_SEP: &str = "~|~";
/// Sentinel printed *after* the loop completes. RouterOS prints script errors
/// (e.g. "syntax error (line 1 column 10)") to stdout and still exits 0, so a
/// broken query would otherwise parse as a silent "0 records". If this marker
/// is absent from the output, the script aborted partway and we error loudly.
const END_MARKER: &str = "MCTL-END";

/// One observed RouterOS record: field name → value (ordered for stable
/// display + deterministic tests).
pub type Record = BTreeMap<String, String>;

/// Build the RouterOS script that prints `fields` for every item under `path`
/// (optionally filtered by a `where` expression, e.g. `comment~"mjolnir"`).
///
/// `path` is a menu path like `/interface/veth`; `fields` are property names
/// valid for that menu (`name`, `address`, `comment`, …).
pub fn build_query_script(path: &str, find_filter: Option<&str>, fields: &[&str]) -> String {
    let find = match find_filter {
        Some(f) => format!("[{path}/find where {f}]"),
        None => format!("[{path}/find]"),
    };

    // `"<field>=" . [:tostr [<path>/get $i <field>]]` for each field, joined by
    // the field separator, with the record prefix in front. The loop variable is
    // `i` (letter-led): RouterOS rejects underscore-led names (`_i` → syntax
    // error at the variable). After the loop, print the completion sentinel.
    let mut expr = format!("{:?}", REC_PREFIX); // quoted "MCTL>"
    for (i, field) in fields.iter().enumerate() {
        if i == 0 {
            expr.push_str(" . ");
        } else {
            expr.push_str(&format!(" . {:?} . ", FIELD_SEP));
        }
        expr.push_str(&format!(
            "{:?} . [:tostr [{path}/get $i {field}]]",
            format!("{field}=")
        ));
    }

    format!(
        ":foreach i in={find} do={{:put ({expr})}}; :put {:?}",
        END_MARKER
    )
}

/// Parse the stdout of a query script into records. Non-`MCTL>` lines (banners,
/// prompts, blank lines) are ignored.
pub fn parse_records(stdout: &str) -> Vec<Record> {
    let mut records = Vec::new();
    for raw in stdout.lines() {
        let line = raw.trim_end_matches('\r');
        let Some(rest) = line.strip_prefix(REC_PREFIX) else {
            continue;
        };
        let mut rec = Record::new();
        if !rest.is_empty() {
            for pair in rest.split(FIELD_SEP) {
                // split_once('=') keeps any '=' in the value (e.g. a comment).
                if let Some((k, v)) = pair.split_once('=') {
                    rec.insert(k.to_string(), v.to_string());
                }
            }
        }
        records.push(rec);
    }
    records
}

/// Verify the query ran to completion. RouterOS exits 0 even on a script error,
/// so the only reliable signal is whether the trailing sentinel was printed.
/// Returns the output (sans control lines) on success; on failure, an error
/// carrying the raw output — which contains RouterOS's own error message.
fn ensure_complete(stdout: &str) -> Result<()> {
    if stdout
        .lines()
        .any(|l| l.trim_end_matches('\r') == END_MARKER)
    {
        Ok(())
    } else {
        bail!(
            "RouterOS query did not complete (no {END_MARKER} sentinel) — the \
             script likely errored. Router output:\n{}",
            stdout.trim()
        )
    }
}

/// Run a mutating RouterOS command (`add`/`set`/`remove`), verifying it
/// completed. Same hazard as queries: RouterOS prints errors to stdout and
/// exits 0. We append the OK sentinel — RouterOS aborts the script before
/// printing it on error — so a missing sentinel means failure, with the
/// router's own message surfaced. Returns stdout (which may include an `add`'s
/// new internal id; that's fine, we only require the sentinel).
pub async fn run_command(ssh: &Ssh, cmd: &str) -> Result<String> {
    let script = format!("{cmd}; :put {END_MARKER:?}");
    let out = ssh.run(&script).await?;
    ensure_complete(&out).with_context(|| format!("RouterOS rejected `{cmd}`"))?;
    Ok(out)
}

/// Observe: run the generated query over SSH and parse the result.
pub async fn query(
    ssh: &Ssh,
    path: &str,
    find_filter: Option<&str>,
    fields: &[&str],
) -> Result<Vec<Record>> {
    let script = build_query_script(path, find_filter, fields);
    let out = ssh.run(&script).await?;
    ensure_complete(&out)?;
    Ok(parse_records(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_no_filter() {
        let s = build_query_script("/interface/veth", None, &["name", "address"]);
        assert_eq!(
            s,
            r#":foreach i in=[/interface/veth/find] do={:put ("MCTL>" . "name=" . [:tostr [/interface/veth/get $i name]] . "~|~" . "address=" . [:tostr [/interface/veth/get $i address]])}; :put "MCTL-END""#
        );
    }

    #[test]
    fn script_uses_letter_led_loop_var() {
        // RouterOS rejects underscore-led variable names — must never emit `_i`.
        let s = build_query_script("/interface/veth", None, &["name"]);
        assert!(!s.contains("$_i"));
        assert!(!s.contains(":foreach _i"));
        assert!(s.ends_with(r#"; :put "MCTL-END""#));
    }

    #[test]
    fn script_with_filter() {
        let s = build_query_script(
            "/ip/firewall/nat",
            Some(r#"comment~"mjolnir""#),
            &["action", "comment"],
        );
        assert!(s.contains(r#"in=[/ip/firewall/nat/find where comment~"mjolnir"]"#));
        assert!(s.contains(r#""action=" . [:tostr [/ip/firewall/nat/get $i action]]"#));
    }

    #[test]
    fn script_zero_fields_is_just_prefix() {
        let s = build_query_script("/interface/bridge", None, &[]);
        assert_eq!(
            s,
            r#":foreach i in=[/interface/bridge/find] do={:put ("MCTL>")}; :put "MCTL-END""#
        );
    }

    #[test]
    fn ensure_complete_accepts_sentinel() {
        assert!(ensure_complete("MCTL>name=br-mesh\r\nMCTL-END\r\n").is_ok());
        assert!(ensure_complete("MCTL-END").is_ok());
    }

    #[test]
    fn ensure_complete_rejects_missing_sentinel() {
        // A RouterOS error with no sentinel must surface as an error carrying
        // the router's message — not a silent empty result.
        let err = ensure_complete("syntax error (line 1 column 10)").unwrap_err();
        assert!(err.to_string().contains("did not complete"));
        assert!(err.to_string().contains("syntax error"));
    }

    #[test]
    fn parses_records_and_ignores_noise() {
        let stdout = "\
some banner line\r
MCTL>name=veth-mesh~|~address=172.20.0.2/24\r
MCTL>name=other~|~address=10.0.0.1/24
\r
press any key";
        let recs = parse_records(stdout);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0]["name"], "veth-mesh");
        assert_eq!(recs[0]["address"], "172.20.0.2/24");
        assert_eq!(recs[1]["name"], "other");
    }

    #[test]
    fn value_keeps_embedded_equals() {
        // A comment with '=' must round-trip (split on FIRST '=').
        let recs = parse_records("MCTL>comment=mjolnir a=b~|~name=x");
        assert_eq!(recs[0]["comment"], "mjolnir a=b");
        assert_eq!(recs[0]["name"], "x");
    }

    #[test]
    fn empty_record_line() {
        let recs = parse_records("MCTL>");
        assert_eq!(recs.len(), 1);
        assert!(recs[0].is_empty());
    }
}
