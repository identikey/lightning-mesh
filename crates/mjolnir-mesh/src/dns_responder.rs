//! `.mesh` DNS responder (S1.1, bead mjolnir-mesh-e21.1.1): a minimal
//! authoritative responder for the `.mesh` zone. Binds a UDP socket (default
//! `127.0.0.1:5335`, the port dnsmasq's `server=/mesh/127.0.0.1#5335` stanza
//! forwards `.mesh` queries to — see `docs/sprints/002-mesh-naming/architecture-decisions.md`
//! D-001/D-005) and answers every query with NXDOMAIN + an SOA authority
//! record. Well-known (e21.1.2) and CRDT-projected service (e21.1.3) answers
//! plug in later through the [`NameTable`] seam below — this story only
//! wires the default.
//!
//! Never panics on malformed input: a packet that fails to parse (or a reply
//! that fails to serialize) is logged at debug/warn and dropped — this
//! responder must never take its recv loop down over a bad client.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, RwLock};

use simple_dns::rdata::{A, RData, SOA, SRV, TXT};
use simple_dns::{CLASS, Name, Packet, PacketFlag, QTYPE, RCODE, ResourceRecord, TYPE};
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use crate::crdt::liveness::{LivenessTracker, monotonic_now_ms};
use crate::crdt::service::{ServiceBookV2, is_reserved_service_name};

/// Default bind port for the `.mesh` responder (loopback-only — dnsmasq is
/// the only client). Configurable so tests can bind an ephemeral port
/// instead of racing a real 5335 listener on the host.
pub const DEFAULT_DNS_PORT: u16 = 5335;

/// UDP responses are capped at 512 bytes for this story's scope (no EDNS0
/// larger-response negotiation yet) — the classic plain-DNS ceiling.
const MAX_RESPONSE_LEN: usize = 512;

/// Query recv buffer size. Larger than 512 so an EDNS0 OPT-bearing query
/// (which may legally exceed the classic 512B ceiling) is never truncated on
/// the way in; the OPT record itself is tolerated and ignored (see
/// [`handle_query`]).
const RECV_BUF_LEN: usize = 4096;

/// Seam for well-known (e21.1.2) and CRDT-projected service (e21.1.3)
/// answers. Returning `None` falls through to this story's NXDOMAIN+SOA
/// default. `name` is the query name as written on the wire, dotted and
/// lowercase-insensitive per DNS convention (e.g. `"hello.mesh."`).
pub trait NameTable: Send + Sync {
    /// Look up A-record answers for `name`. `None` means "no answer here" —
    /// SRV/TXT lookups will be added as their own methods on this trait when
    /// e21.1.3 lands, rather than overloading this one.
    fn lookup_a(&self, name: &str) -> Option<Vec<Ipv4Addr>>;

    /// Whether `name` is known to this table at all, independent of the
    /// queried record type. Distinguishes NODATA ("name exists, wrong type"
    /// — NOERROR, empty answer) from NXDOMAIN ("name unknown" — see
    /// [`handle_query`]). Defaults to "known iff an A answer exists", which
    /// is correct for an A-only table; a table serving other record types
    /// (e.g. the future SRV/TXT service table) overrides this.
    fn exists(&self, name: &str) -> bool {
        self.lookup_a(name).is_some()
    }

    /// SRV answer for `name` (e21.1.3): this data model has exactly one SRV
    /// record per service name (one `port`/`protocol` per service entry), so
    /// this returns just the port rather than a list — priority and weight
    /// are always `0`, and the record's target is `name` itself (the same
    /// composite table's `A` answer resolves it; there is no separate SRV
    /// target hostname in this model, e.g. no `_http._tcp.NAME` label).
    /// Defaults to "no SRV data here", correct for any table that only
    /// serves `A` (like [`WellKnownTable`]).
    fn lookup_srv(&self, _name: &str) -> Option<u16> {
        None
    }

    /// TXT answer for `name` (e21.1.3): the service's key/value map, encoded
    /// on the wire as one `"key=value"` character-string per pair (see
    /// [`handle_query`]). Defaults to "no TXT data here".
    fn lookup_txt(&self, _name: &str) -> Option<BTreeMap<String, String>> {
        None
    }
}

/// This story's table: every name falls through to NXDOMAIN+SOA. Later
/// stories replace this with a table backed by the CRDT service/user books.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoAnswers;

impl NameTable for NoAnswers {
    fn lookup_a(&self, _name: &str) -> Option<Vec<Ipv4Addr>> {
        None
    }
}

/// Shared, dynamically-updated client-gateway IP (S1.2, bead e21.1.2): the
/// `.1` address of this node's claimed client /24, or `None` before a claim
/// has landed. A [`RwLock`] (not a constructor-time constant) because the
/// subnet claim can land well after this table is built and handed to the
/// responder — `mjolnir-meshd` writes through this handle every time
/// `assign_client_addr`/`retract_client_addr` runs (fresh claim, restored
/// claim, or a conflict re-claim).
pub type GatewayHandle = Arc<RwLock<Option<Ipv4Addr>>>;

/// Pre-claim fallback gateway (decision D-003): before this node has claimed
/// a client /24, `hello.mesh`/`id.mesh` answer the stock recovery address
/// instead of NXDOMAIN, matching the address a fresh, unclaimed router
/// answers DHCP/UI on.
pub const PRE_CLAIM_GATEWAY: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);

/// Answers the reserved well-known `.mesh` names
/// ([`crate::crdt::service::RESERVED_SERVICE_NAMES`] — `hello`, `id`) with
/// this node's own client gateway IP: the claimed /24's `.1` once a subnet
/// claim lands, or [`PRE_CLAIM_GATEWAY`] before one does.
#[derive(Clone)]
pub struct WellKnownTable {
    gateway: GatewayHandle,
}

impl WellKnownTable {
    /// Build a table backed by `gateway`. The caller keeps its own clone of
    /// the handle and writes the current gateway IP into it as the subnet
    /// claim changes.
    pub fn new(gateway: GatewayHandle) -> Self {
        Self { gateway }
    }

    fn current_gateway(&self) -> Ipv4Addr {
        match self.gateway.read() {
            Ok(guard) => guard.unwrap_or(PRE_CLAIM_GATEWAY),
            Err(_) => PRE_CLAIM_GATEWAY,
        }
    }
}

impl NameTable for WellKnownTable {
    fn lookup_a(&self, name: &str) -> Option<Vec<Ipv4Addr>> {
        if is_well_known_name(name) {
            Some(vec![self.current_gateway()])
        } else {
            None
        }
    }

    fn exists(&self, name: &str) -> bool {
        is_well_known_name(name)
    }
}

/// True if `name` (a wire-format qname, e.g. `"hello.mesh."`) is one of the
/// reserved well-known service names under the `.mesh` apex, case-insensitive.
fn is_well_known_name(name: &str) -> bool {
    let lower = name.trim_end_matches('.').to_ascii_lowercase();
    match lower.strip_suffix(".mesh") {
        Some(label) => is_reserved_service_name(label),
        None => false,
    }
}

/// Chains multiple [`NameTable`]s: the first table that answers wins, and a
/// name `exists` if any table in the chain claims it. Lets `mjolnir-meshd`
/// stack the well-known table (S1.2) ahead of the future CRDT-projected
/// service table (S1.3) without either table knowing about the other.
pub struct CompositeTable {
    tables: Vec<Arc<dyn NameTable>>,
}

impl CompositeTable {
    pub fn new(tables: Vec<Arc<dyn NameTable>>) -> Self {
        Self { tables }
    }
}

impl NameTable for CompositeTable {
    fn lookup_a(&self, name: &str) -> Option<Vec<Ipv4Addr>> {
        self.tables.iter().find_map(|t| t.lookup_a(name))
    }

    fn exists(&self, name: &str) -> bool {
        self.tables.iter().any(|t| t.exists(name))
    }

    fn lookup_srv(&self, name: &str) -> Option<u16> {
        self.tables.iter().find_map(|t| t.lookup_srv(name))
    }

    fn lookup_txt(&self, name: &str) -> Option<BTreeMap<String, String>> {
        self.tables.iter().find_map(|t| t.lookup_txt(name))
    }
}

/// Pure CRDT projection over the daemon's shared v2 service store (S1.3,
/// bead e21.1.3): serves `A`/`SRV`/`TXT` answers straight from the live
/// [`ServiceBookV2`], no cache (FR8) — every query takes the lock, reads the
/// current map, and releases it immediately, so a store mutation is visible
/// on the very next query.
///
/// Wire mapping (documented here since the PRD leaves the exact SRV/TXT
/// shape to this story — see [`NameTable::lookup_srv`]/[`NameTable::lookup_txt`]
/// for the rationale): `A` -> `entry.ip` (IPv4 only; a v6-only entry has no
/// `A` answer, which the qtype dispatch turns into NODATA via `exists`);
/// `SRV` -> `SRV 0 0 <port> <name>`; `TXT` -> one `"key=value"`
/// character-string per `entry.txt` pair. A tombstoned or never-published
/// name is absent from the book, so `exists` returns `false` and the
/// composite dispatch falls through to NXDOMAIN.
#[derive(Clone)]
pub struct ServiceTable {
    store: Arc<Mutex<ServiceBookV2>>,
    /// When set, a name whose owning node is stale (bead e21.9) is treated as
    /// absent — it stops resolving (NXDOMAIN) rather than handing back a
    /// black-hole IP for an offline owner. `None` disables the filter (all
    /// entries always resolve), which is the constructor tests use.
    liveness: Option<Arc<Mutex<LivenessTracker>>>,
}

impl ServiceTable {
    /// Build a table reading from `store`, with no liveness filtering — every
    /// entry in the book resolves. The caller (mjolnir-meshd) owns the
    /// daemon-side write path (gossip dispatch, local publish/unpublish); this
    /// table only ever reads.
    pub fn new(store: Arc<Mutex<ServiceBookV2>>) -> Self {
        Self {
            store,
            liveness: None,
        }
    }

    /// Build a table that filters out names whose owner has gone stale per the
    /// shared [`LivenessTracker`] (bead e21.9). Used by the daemon so an
    /// offline owner's names stop resolving; the entry stays in the book, so
    /// the owner's return silently un-stales it.
    pub fn with_liveness(
        store: Arc<Mutex<ServiceBookV2>>,
        liveness: Arc<Mutex<LivenessTracker>>,
    ) -> Self {
        Self {
            store,
            liveness: Some(liveness),
        }
    }

    /// `name` is a wire-format qname (e.g. `"printer.mesh."`); the service
    /// book keys on the bare service name (e.g. `"printer"`) with no zone
    /// suffix, so strip the trailing `.mesh.`/`.mesh` before looking it up.
    /// Returns `None` for a qname outside the `.mesh` apex entirely.
    fn book_key<'a>(&self, name: &'a str) -> Option<&'a str> {
        name.trim_end_matches('.').strip_suffix(".mesh")
    }

    /// True if `owner_node_id`'s records should be hidden because its liveness
    /// beacon has aged past the stale threshold (bead e21.9). Always `false`
    /// when no tracker is configured. Takes the liveness lock only after the
    /// book lock has been released by the caller, so the two never nest.
    fn owner_stale(&self, owner_node_id: &str) -> bool {
        match &self.liveness {
            Some(tracker) => tracker
                .lock()
                .map(|t| t.is_stale(owner_node_id, monotonic_now_ms()))
                .unwrap_or(false),
            None => false,
        }
    }

    /// Read `(value, owner)` for `key` under the book lock, releasing it before
    /// any liveness check. Returns `None` if the name is absent.
    fn get_if_live<T>(
        &self,
        key: &str,
        extract: impl FnOnce(&crate::crdt::service::ServiceEntryV2) -> T,
    ) -> Option<T> {
        let (value, owner) = {
            let book = self.store.lock().ok()?;
            let entry = book.get(key)?;
            (extract(entry), entry.owner_node_id.clone())
        };
        if self.owner_stale(&owner) {
            return None;
        }
        Some(value)
    }
}

impl NameTable for ServiceTable {
    fn lookup_a(&self, name: &str) -> Option<Vec<Ipv4Addr>> {
        let key = self.book_key(name)?;
        match self.get_if_live(key, |e| e.ip)? {
            IpAddr::V4(v4) => Some(vec![v4]),
            // No AAAA support in this story's scope; an A query against a
            // v6-only entry is NODATA (the name still `exists`), not NXDOMAIN.
            IpAddr::V6(_) => None,
        }
    }

    fn exists(&self, name: &str) -> bool {
        match self.book_key(name) {
            // A stale owner's name reports absent, so the composite dispatch
            // falls through to NXDOMAIN rather than NODATA (bead e21.9).
            Some(key) => self.get_if_live(key, |_| ()).is_some(),
            None => false,
        }
    }

    fn lookup_srv(&self, name: &str) -> Option<u16> {
        let key = self.book_key(name)?;
        // Port 0 means "no service port" — a stationary device published as an
        // A-record only (bead e21.3, e.g. a NAS reachable by IP with no SRV).
        // The name still resolves (A), so this is NODATA for SRV, not NXDOMAIN.
        self.get_if_live(key, |e| e.port).filter(|p| *p != 0)
    }

    fn lookup_txt(&self, name: &str) -> Option<BTreeMap<String, String>> {
        let key = self.book_key(name)?;
        self.get_if_live(key, |e| e.txt.clone())
    }
}

/// Wall-clock ms since the Unix epoch. Leased-name freshness is compared
/// against the HLC `wall_clock` stamped on each record, so this must be the
/// wall clock — NOT the monotonic clock the liveness plane
/// ([`monotonic_now_ms`]) uses for beacon staleness.
fn wall_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// How long after a leased name's last renewal it stops **resolving** — the
/// fast UX fade. Deliberately far shorter than the ownership lease
/// ([`LEASE_TTL_MS`](crate::crdt::leased_name::LEASE_TTL_MS), 1h): a client that
/// stops heartbeating vanishes from DNS within this window, yet keeps *owning*
/// the name for the full lease, so a brief outage or reboot (with a renewal)
/// brings it straight back and no other key can grab it in the meantime. The
/// claiming client must renew well inside this window (≤ ~30s).
pub const LEASED_NAME_RESOLVE_STALE_MS: u64 = 90_000;

/// Pure CRDT projection over the daemon's key-owned **leased** name store
/// (bead mjolnir-mesh-71x), the client-claimed counterpart to [`ServiceTable`].
/// A name resolves only while its lease has been renewed within
/// [`LEASED_NAME_RESOLVE_STALE_MS`]; a lapsed-but-not-yet-reclaimed name reads
/// as absent (NXDOMAIN) rather than black-holing traffic to an offline client.
/// Ownership/reclaim across keys is the CRDT merge's job
/// ([`merge_leased_name`](crate::crdt::leased_name::merge_leased_name)); this
/// table only decides what currently answers.
#[derive(Clone)]
pub struct LeasedNameTable {
    store: Arc<Mutex<crate::crdt::leased_name::LeasedNameBook>>,
}

impl LeasedNameTable {
    pub fn new(store: Arc<Mutex<crate::crdt::leased_name::LeasedNameBook>>) -> Self {
        Self { store }
    }

    /// Strip the `.mesh` apex from a wire qname; the book keys on the bare flat
    /// name (e.g. `"walkie-talkie"`). Mirrors [`ServiceTable::book_key`].
    fn book_key<'a>(&self, name: &'a str) -> Option<&'a str> {
        name.trim_end_matches('.').strip_suffix(".mesh")
    }

    /// True iff `e`'s last renewal is within the resolve-freshness window of
    /// `now_ms`. Pure (time injected) so the fade is unit-testable without a
    /// real clock.
    fn resolves(e: &crate::crdt::leased_name::LeasedName, now_ms: u64) -> bool {
        now_ms.saturating_sub(e.renewed_at.wall_clock) <= LEASED_NAME_RESOLVE_STALE_MS
    }

    fn get_if_fresh<T>(
        &self,
        key: &str,
        now_ms: u64,
        extract: impl FnOnce(&crate::crdt::leased_name::LeasedName) -> T,
    ) -> Option<T> {
        let book = self.store.lock().ok()?;
        let e = book.get(key)?;
        if !Self::resolves(e, now_ms) {
            return None;
        }
        Some(extract(e))
    }
}

impl NameTable for LeasedNameTable {
    fn lookup_a(&self, name: &str) -> Option<Vec<Ipv4Addr>> {
        let key = self.book_key(name)?;
        match self.get_if_fresh(key, wall_now_ms(), |e| e.ip)? {
            IpAddr::V4(v4) => Some(vec![v4]),
            IpAddr::V6(_) => None, // no AAAA in scope; NODATA, not NXDOMAIN
        }
    }

    fn exists(&self, name: &str) -> bool {
        match self.book_key(name) {
            Some(key) => self.get_if_fresh(key, wall_now_ms(), |_| ()).is_some(),
            None => false,
        }
    }

    fn lookup_srv(&self, name: &str) -> Option<u16> {
        let key = self.book_key(name)?;
        // Port 0 → A-only claim, no SRV (NODATA, not NXDOMAIN).
        self.get_if_fresh(key, wall_now_ms(), |e| e.port).filter(|p| *p != 0)
    }
}

/// A bound, running responder. Dropping this does not stop the background
/// task — call [`ResponderHandle::abort`] at shutdown, as `mjolnir-meshd` does.
pub struct ResponderHandle {
    /// The address actually bound (useful in tests that pass port 0).
    pub local_addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl ResponderHandle {
    /// Stop the responder's recv loop.
    pub fn abort(&self) {
        self.task.abort();
    }
}

/// Bind the responder socket and spawn its recv loop. Returns once the
/// socket is bound (not once the loop exits), so callers can sequence
/// startup — `mjolnir-meshd` binds this BEFORE any UCI/dnsmasq reconcile
/// (FR14), so dnsmasq's `.mesh` upstream is answerable the moment it's
/// configured.
pub async fn start(
    bind_addr: SocketAddr,
    table: Arc<dyn NameTable>,
) -> std::io::Result<ResponderHandle> {
    let socket = UdpSocket::bind(bind_addr).await?;
    let local_addr = socket.local_addr()?;
    info!(%local_addr, "mesh DNS responder bound");
    let task = tokio::spawn(recv_loop(socket, table));
    Ok(ResponderHandle { local_addr, task })
}

async fn recv_loop(socket: UdpSocket, table: Arc<dyn NameTable>) {
    let mut buf = [0u8; RECV_BUF_LEN];
    loop {
        let (len, peer) = match socket.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                // e.g. an ICMP port-unreachable bounced back from a prior
                // send — not fatal, keep serving other peers.
                warn!("mesh DNS responder: recv error: {e}");
                continue;
            }
        };

        match handle_query(&buf[..len], table.as_ref()) {
            Some(reply) => {
                if let Err(e) = socket.send_to(&reply, peer).await {
                    warn!(%peer, "mesh DNS responder: send error: {e}");
                }
            }
            None => {
                // Malformed packet or an unbuildable reply — never crash the
                // loop over a bad client, just drop and keep serving.
                debug!(%peer, "mesh DNS responder: dropping unparseable/unbuildable packet");
            }
        }
    }
}

/// Parse `query_bytes`, dispatch through `table`, and build the wire-format
/// reply. Returns `None` if the query fails to parse or the reply fails to
/// serialize — callers treat that as "drop, don't respond, keep the loop
/// alive."
pub(crate) fn handle_query(query_bytes: &[u8], table: &dyn NameTable) -> Option<Vec<u8>> {
    let query = match Packet::parse(query_bytes) {
        Ok(p) => p,
        Err(e) => {
            debug!("mesh DNS responder: failed to parse query: {e}");
            return None;
        }
    };

    // EDNS0 OPT is tolerated: `Packet::parse` already lifts any OPT record
    // out of `additional_records` into `query.opt()`. We simply never look
    // at it and never echo one back in the reply — "tolerate and ignore"
    // per this story's scope.
    let mut reply = Packet::new_reply(query.id());
    reply.set_flags(PacketFlag::AUTHORITATIVE_ANSWER);

    // Scratch storage for TXT character-strings: `TXT::add_string` borrows,
    // so the formatted "key=value" strings must outlive `reply` itself (which
    // is serialized well after the match arm below returns) — declared here,
    // at the outermost function scope, rather than inside the match arm.
    let mut txt_scratch: Vec<String> = Vec::new();

    match query.questions.into_iter().next() {
        Some(question) => {
            let qname = question.qname.to_string();

            // Only a matching qtype produces answers here; any other qtype
            // (or a qtype this table has no answer for) falls through to the
            // NODATA/NXDOMAIN dispatch below untouched, per this story's
            // "NODATA, never NXDOMAIN for a known name" rule (e21.1.1/e21.1.3).
            let answers: Option<Vec<ResourceRecord>> = match question.qtype {
                QTYPE::TYPE(TYPE::A) => table.lookup_a(&qname).filter(|a| !a.is_empty()).map(|addrs| {
                    addrs
                        .into_iter()
                        .map(|addr| {
                            ResourceRecord::new(
                                question.qname.clone(),
                                CLASS::IN,
                                30,
                                RData::A(A { address: addr.into() }),
                            )
                        })
                        .collect::<Vec<_>>()
                }),
                QTYPE::TYPE(TYPE::SRV) => table.lookup_srv(&qname).map(|port| {
                    vec![ResourceRecord::new(
                        question.qname.clone(),
                        CLASS::IN,
                        30,
                        RData::SRV(SRV { priority: 0, weight: 0, port, target: question.qname.clone() }),
                    )]
                }),
                QTYPE::TYPE(TYPE::TXT) => table.lookup_txt(&qname).filter(|m| !m.is_empty()).map(|map| {
                    let mut rec = TXT::new();
                    for (k, v) in &map {
                        txt_scratch.push(format!("{k}={v}"));
                    }
                    for s in &txt_scratch {
                        if let Err(e) = rec.add_string(s) {
                            warn!("mesh DNS responder: TXT character-string too long, dropping: {e}");
                        }
                    }
                    vec![ResourceRecord::new(question.qname.clone(), CLASS::IN, 30, RData::TXT(rec))]
                }),
                _ => None,
            };

            // Independent of qtype: does the table know this name at all?
            // Distinguishes "name exists, wrong type" (NODATA) from "name
            // unknown" (NXDOMAIN) — a non-A query, or an A query the table
            // has no A answer for, still gets NODATA if the name exists.
            let name_exists = table.exists(&qname);

            reply.questions.push(question);

            match answers {
                Some(records) => reply.answers = records,
                None if name_exists => {
                    // NODATA (RFC 2308): NOERROR, no answers, no SOA — the
                    // reply's rcode already defaults to NoError from
                    // `Packet::new_reply`.
                }
                None => {
                    *reply.rcode_mut() = RCODE::NameError;
                    reply.name_servers.push(mesh_soa_record());
                }
            }
        }
        None => {
            // No question section at all — nothing to look up; still answer
            // NXDOMAIN+SOA so a parseable-but-empty query gets a well-formed,
            // bounded response instead of silence.
            *reply.rcode_mut() = RCODE::NameError;
            reply.name_servers.push(mesh_soa_record());
        }
    }

    let bytes = match reply.build_bytes_vec() {
        Ok(b) => b,
        Err(e) => {
            warn!("mesh DNS responder: failed to build reply: {e}");
            return None;
        }
    };

    if bytes.len() > MAX_RESPONSE_LEN {
        // Not reachable at this story's answer sizes; guarded anyway so a
        // future oversized answer can't silently violate the UDP/512B
        // contract.
        warn!(
            len = bytes.len(),
            "mesh DNS responder: reply exceeds 512B, dropping"
        );
        return None;
    }

    Some(bytes)
}

/// The SOA authority record for negative (`NXDOMAIN`/`NODATA`) answers in the
/// `.mesh` zone (decision D-005): owner name is the zone apex (`mesh.`,
/// this responder's authority); TTL matches `minimum` per RFC 2308's
/// negative-caching convention.
fn mesh_soa_record() -> ResourceRecord<'static> {
    ResourceRecord::new(
        Name::new_unchecked("mesh."),
        CLASS::IN,
        30,
        RData::SOA(SOA {
            mname: Name::new_unchecked("hello.mesh."),
            rname: Name::new_unchecked("ops.hello.mesh."),
            serial: 1,
            refresh: 3600,
            retry: 600,
            expire: 86400,
            minimum: 30,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_dns::QCLASS;
    use std::net::{IpAddr, SocketAddr};
    use std::time::Duration;

    fn build_query(name: &str, qtype: TYPE) -> Vec<u8> {
        let mut query = Packet::new_query(0x1234);
        query.questions.push(simple_dns::Question::new(
            Name::new(name).unwrap(),
            qtype.into(),
            QCLASS::CLASS(CLASS::IN),
            false,
        ));
        query.build_bytes_vec().unwrap()
    }

    #[test]
    fn unknown_name_returns_nxdomain_with_soa() {
        let bytes = build_query("unknown.mesh.", TYPE::A);
        let reply_bytes = handle_query(&bytes, &NoAnswers).expect("should build a reply");

        let reply = Packet::parse(&reply_bytes).expect("reply should parse");
        assert_eq!(reply.rcode(), RCODE::NameError);
        assert_eq!(reply.questions.len(), 1);
        assert_eq!(reply.answers.len(), 0);
        assert_eq!(reply.name_servers.len(), 1);
        match &reply.name_servers[0].rdata {
            RData::SOA(soa) => {
                assert_eq!(soa.mname.to_string(), "hello.mesh");
                assert_eq!(soa.rname.to_string(), "ops.hello.mesh");
                assert_eq!(soa.serial, 1);
                assert_eq!(soa.refresh, 3600);
                assert_eq!(soa.retry, 600);
                assert_eq!(soa.expire, 86400);
                assert_eq!(soa.minimum, 30);
            }
            other => panic!("expected SOA authority record, got {other:?}"),
        }
        assert!(reply_bytes.len() <= MAX_RESPONSE_LEN);
    }

    #[test]
    fn malformed_bytes_never_panics() {
        // Assorted garbage: empty, too short, and a header claiming a
        // question section that isn't actually there (truncated body).
        assert!(handle_query(&[], &NoAnswers).is_none());
        assert!(handle_query(&[0xFF; 3], &NoAnswers).is_none());
        let mut truncated_header = [0u8; 12];
        truncated_header[5] = 1; // QDCOUNT = 1, but no question bytes follow
        assert!(handle_query(&truncated_header, &NoAnswers).is_none());
        assert!(handle_query(&[0xAA; 200], &NoAnswers).is_none());
    }

    #[test]
    fn well_known_names_answer_pre_claim_gateway() {
        let table = WellKnownTable::new(Arc::new(RwLock::new(None)));
        for name in ["hello.mesh.", "id.mesh.", "HELLO.MESH.", "Id.Mesh."] {
            let bytes = build_query(name, TYPE::A);
            let reply_bytes = handle_query(&bytes, &table).expect("should build a reply");
            let reply = Packet::parse(&reply_bytes).expect("reply should parse");
            assert_eq!(reply.rcode(), RCODE::NoError, "name={name}");
            assert_eq!(reply.answers.len(), 1, "name={name}");
            match &reply.answers[0].rdata {
                RData::A(a) => assert_eq!(Ipv4Addr::from(a.address), PRE_CLAIM_GATEWAY),
                other => panic!("expected A record, got {other:?}"),
            }
            assert_eq!(reply.answers[0].ttl, 30);
        }
    }

    #[test]
    fn well_known_names_answer_claimed_gateway_once_set() {
        let gateway = Arc::new(RwLock::new(None));
        let table = WellKnownTable::new(gateway.clone());
        let claimed: Ipv4Addr = "10.42.61.1".parse().unwrap();
        *gateway.write().unwrap() = Some(claimed);

        let bytes = build_query("hello.mesh.", TYPE::A);
        let reply_bytes = handle_query(&bytes, &table).expect("should build a reply");
        let reply = Packet::parse(&reply_bytes).expect("reply should parse");
        assert_eq!(reply.rcode(), RCODE::NoError);
        assert_eq!(reply.answers.len(), 1);
        match &reply.answers[0].rdata {
            RData::A(a) => assert_eq!(Ipv4Addr::from(a.address), claimed),
            other => panic!("expected A record, got {other:?}"),
        }
    }

    #[test]
    fn well_known_name_non_a_qtype_is_nodata_not_nxdomain() {
        let table = WellKnownTable::new(Arc::new(RwLock::new(None)));
        let bytes = build_query("hello.mesh.", TYPE::AAAA);
        let reply_bytes = handle_query(&bytes, &table).expect("should build a reply");
        let reply = Packet::parse(&reply_bytes).expect("reply should parse");
        assert_eq!(
            reply.rcode(),
            RCODE::NoError,
            "NODATA must be NOERROR, not NXDOMAIN"
        );
        assert_eq!(reply.answers.len(), 0);
        assert_eq!(
            reply.name_servers.len(),
            0,
            "NODATA carries no SOA in this dispatch"
        );
    }

    #[test]
    fn unreserved_name_is_still_nxdomain_through_well_known_table() {
        let table = WellKnownTable::new(Arc::new(RwLock::new(None)));
        let bytes = build_query("printer.mesh.", TYPE::A);
        let reply_bytes = handle_query(&bytes, &table).expect("should build a reply");
        let reply = Packet::parse(&reply_bytes).expect("reply should parse");
        assert_eq!(reply.rcode(), RCODE::NameError);
        assert_eq!(reply.name_servers.len(), 1);
    }

    #[test]
    fn composite_table_stacks_well_known_ahead_of_no_answers() {
        let gateway = Arc::new(RwLock::new(Some("10.42.7.1".parse().unwrap())));
        let composite = CompositeTable::new(vec![
            Arc::new(WellKnownTable::new(gateway)),
            Arc::new(NoAnswers),
        ]);

        let hello = build_query("hello.mesh.", TYPE::A);
        let hello_reply_bytes = handle_query(&hello, &composite).unwrap();
        let reply = Packet::parse(&hello_reply_bytes).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError);
        assert_eq!(reply.answers.len(), 1);

        let unknown = build_query("unknown.mesh.", TYPE::A);
        let unknown_reply_bytes = handle_query(&unknown, &composite).unwrap();
        let reply = Packet::parse(&unknown_reply_bytes).unwrap();
        assert_eq!(reply.rcode(), RCODE::NameError);
    }

    // --- ServiceTable (bead e21.1.3) ---

    fn v2_entry(
        ip: Ipv4Addr,
        port: u16,
        protocol: &str,
        txt: &[(&str, &str)],
    ) -> crate::crdt::service::ServiceEntryV2 {
        use crate::crdt::hlc::HLC;
        crate::crdt::service::ServiceEntryV2 {
            owner_node_id: "router-a".to_string(),
            first_claimed_at: HLC {
                wall_clock: 1,
                counter: 0,
                node_id: "router-a".to_string(),
            },
            updated_at: HLC {
                wall_clock: 1,
                counter: 0,
                node_id: "router-a".to_string(),
            },
            ip: IpAddr::V4(ip),
            port,
            protocol: protocol.to_string(),
            txt: txt
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            host_mac: None,
        }
    }

    #[test]
    fn service_table_a_srv_txt_from_store() {
        let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
        store.lock().unwrap().insert(
            "wiki".to_string(),
            v2_entry(
                "10.42.1.50".parse().unwrap(),
                8080,
                "_http._tcp",
                &[("path", "/wiki")],
            ),
        );
        let table = ServiceTable::new(store);

        let a_reply = handle_query(&build_query("wiki.mesh.", TYPE::A), &table).unwrap();
        let reply = Packet::parse(&a_reply).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError);
        match &reply.answers[0].rdata {
            RData::A(a) => assert_eq!(
                Ipv4Addr::from(a.address),
                "10.42.1.50".parse::<Ipv4Addr>().unwrap()
            ),
            other => panic!("expected A, got {other:?}"),
        }

        let srv_reply = handle_query(&build_query("wiki.mesh.", TYPE::SRV), &table).unwrap();
        let reply = Packet::parse(&srv_reply).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError);
        match &reply.answers[0].rdata {
            RData::SRV(srv) => {
                assert_eq!(srv.priority, 0);
                assert_eq!(srv.weight, 0);
                assert_eq!(srv.port, 8080);
                assert_eq!(srv.target.to_string(), "wiki.mesh");
            }
            other => panic!("expected SRV, got {other:?}"),
        }

        let txt_reply = handle_query(&build_query("wiki.mesh.", TYPE::TXT), &table).unwrap();
        let reply = Packet::parse(&txt_reply).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError);
        match &reply.answers[0].rdata {
            RData::TXT(txt) => {
                let attrs = txt.attributes();
                assert_eq!(
                    attrs.get("path").and_then(|v| v.clone()),
                    Some("/wiki".to_string())
                );
            }
            other => panic!("expected TXT, got {other:?}"),
        }
    }

    #[test]
    fn scoped_device_resolves_a_but_port_zero_has_no_srv() {
        // A stationary device (bead e21.3) is a scoped, two-label service key
        // `<host>.<scope>`, published A-record-only (port 0). It resolves at
        // `<host>.<scope>.mesh` with no responder changes, and SRV is NODATA.
        use crate::crdt::service::device_service_key;
        let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
        let key = device_service_key("nas", "router-a").unwrap(); // e.g. "nas.<scope>"
        store.lock().unwrap().insert(
            key.clone(),
            v2_entry("192.168.7.20".parse().unwrap(), 0, "_tcp", &[]),
        );
        let table = ServiceTable::new(store);
        let qname = format!("{key}.mesh.");

        let a_reply = handle_query(&build_query(&qname, TYPE::A), &table).unwrap();
        let reply = Packet::parse(&a_reply).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError);
        match &reply.answers[0].rdata {
            RData::A(a) => assert_eq!(
                Ipv4Addr::from(a.address),
                "192.168.7.20".parse::<Ipv4Addr>().unwrap()
            ),
            other => panic!("expected A, got {other:?}"),
        }

        // SRV: name exists (A answered) but has no service port → NODATA, no answers.
        let srv_reply = handle_query(&build_query(&qname, TYPE::SRV), &table).unwrap();
        let reply = Packet::parse(&srv_reply).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError);
        assert!(
            reply.answers.is_empty(),
            "port-0 device must not answer SRV"
        );
    }

    #[test]
    fn service_table_store_mutation_visible_on_next_query() {
        let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
        let table = ServiceTable::new(store.clone());

        let before = handle_query(&build_query("printer.mesh.", TYPE::A), &table).unwrap();
        assert_eq!(Packet::parse(&before).unwrap().rcode(), RCODE::NameError);

        store.lock().unwrap().insert(
            "printer".to_string(),
            v2_entry("10.42.1.60".parse().unwrap(), 631, "_ipp._tcp", &[]),
        );

        let after = handle_query(&build_query("printer.mesh.", TYPE::A), &table).unwrap();
        let reply = Packet::parse(&after).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError);
        assert_eq!(reply.answers.len(), 1);
    }

    #[test]
    fn service_table_absent_name_is_nxdomain() {
        let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
        let table = ServiceTable::new(store);
        let bytes = handle_query(&build_query("ghost.mesh.", TYPE::A), &table).unwrap();
        let reply = Packet::parse(&bytes).unwrap();
        assert_eq!(reply.rcode(), RCODE::NameError);
        assert_eq!(reply.name_servers.len(), 1);
    }

    // --- ServiceTable liveness filter (bead e21.9) ---

    fn liveness_tracker() -> Arc<Mutex<LivenessTracker>> {
        // 60s stale / 1h hard-expiry, matching the daemon defaults.
        Arc::new(Mutex::new(LivenessTracker::new(60_000, 3_600_000)))
    }

    #[test]
    fn service_table_stale_owner_is_nxdomain() {
        // A name whose owner ("router-a") has never beaconed reads as stale, so
        // the name stops resolving (NXDOMAIN) rather than handing back a
        // black-hole IP — the headline e21.9 fix.
        let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
        store.lock().unwrap().insert(
            "wiki".to_string(),
            v2_entry(
                "10.42.1.50".parse().unwrap(),
                8080,
                "_http._tcp",
                &[("path", "/wiki")],
            ),
        );
        let table = ServiceTable::with_liveness(store, liveness_tracker());

        for qtype in [TYPE::A, TYPE::SRV, TYPE::TXT] {
            let bytes = handle_query(&build_query("wiki.mesh.", qtype), &table).unwrap();
            let reply = Packet::parse(&bytes).unwrap();
            assert_eq!(
                reply.rcode(),
                RCODE::NameError,
                "stale owner should NXDOMAIN for {qtype:?}"
            );
        }
    }

    #[test]
    fn service_table_fresh_owner_resolves() {
        let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
        store.lock().unwrap().insert(
            "wiki".to_string(),
            v2_entry("10.42.1.50".parse().unwrap(), 8080, "_http._tcp", &[]),
        );
        let tracker = liveness_tracker();
        // Owner has beaconed just now -> fresh -> the name resolves normally.
        tracker
            .lock()
            .unwrap()
            .touch("router-a", monotonic_now_ms());
        let table = ServiceTable::with_liveness(store, tracker);

        let bytes = handle_query(&build_query("wiki.mesh.", TYPE::A), &table).unwrap();
        let reply = Packet::parse(&bytes).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError);
        assert_eq!(reply.answers.len(), 1);
    }

    #[test]
    fn service_table_owner_return_unstales_the_name() {
        // Stale -> NXDOMAIN; after the owner's beacon arrives the SAME entry
        // (never removed from the book) resolves again — the silent-recovery
        // property. Uses one shared tracker mutated between queries.
        let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
        store.lock().unwrap().insert(
            "wiki".to_string(),
            v2_entry("10.42.1.50".parse().unwrap(), 8080, "_http._tcp", &[]),
        );
        let tracker = liveness_tracker();
        let table = ServiceTable::with_liveness(store, tracker.clone());

        let before = handle_query(&build_query("wiki.mesh.", TYPE::A), &table).unwrap();
        assert_eq!(Packet::parse(&before).unwrap().rcode(), RCODE::NameError);

        tracker
            .lock()
            .unwrap()
            .observe("router-a", 100, 1, monotonic_now_ms());

        let after = handle_query(&build_query("wiki.mesh.", TYPE::A), &table).unwrap();
        assert_eq!(Packet::parse(&after).unwrap().rcode(), RCODE::NoError);
    }

    #[test]
    fn service_table_txt_query_with_no_txt_data_is_nodata() {
        let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
        store.lock().unwrap().insert(
            "printer".to_string(),
            v2_entry("10.42.1.60".parse().unwrap(), 631, "_ipp._tcp", &[]),
        );
        let table = ServiceTable::new(store);
        let bytes = handle_query(&build_query("printer.mesh.", TYPE::TXT), &table).unwrap();
        let reply = Packet::parse(&bytes).unwrap();
        assert_eq!(
            reply.rcode(),
            RCODE::NoError,
            "NODATA, not NXDOMAIN, for an existing name with no TXT data"
        );
        assert_eq!(reply.answers.len(), 0);
    }

    #[test]
    fn service_table_composed_after_well_known_table() {
        let gateway = Arc::new(RwLock::new(Some("10.42.7.1".parse().unwrap())));
        let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
        store.lock().unwrap().insert(
            "wiki".to_string(),
            v2_entry("10.42.1.50".parse().unwrap(), 8080, "_http._tcp", &[]),
        );
        let composite = CompositeTable::new(vec![
            Arc::new(WellKnownTable::new(gateway)),
            Arc::new(ServiceTable::new(store)),
        ]);

        let hello = handle_query(&build_query("hello.mesh.", TYPE::A), &composite).unwrap();
        assert_eq!(Packet::parse(&hello).unwrap().rcode(), RCODE::NoError);

        let wiki = handle_query(&build_query("wiki.mesh.", TYPE::A), &composite).unwrap();
        assert_eq!(Packet::parse(&wiki).unwrap().rcode(), RCODE::NoError);

        let ghost = handle_query(&build_query("ghost.mesh.", TYPE::A), &composite).unwrap();
        assert_eq!(Packet::parse(&ghost).unwrap().rcode(), RCODE::NameError);
    }

    #[test]
    fn edns0_opt_is_tolerated_and_ignored() {
        let mut query = Packet::new_query(0x5678);
        query.questions.push(simple_dns::Question::new(
            Name::new("foo.mesh.").unwrap(),
            TYPE::A.into(),
            QCLASS::CLASS(CLASS::IN),
            false,
        ));
        *query.opt_mut() = Some(simple_dns::rdata::OPT {
            opt_codes: Vec::new(),
            udp_packet_size: 4096,
            version: 0,
        });
        let bytes = query.build_bytes_vec().unwrap();

        let reply_bytes =
            handle_query(&bytes, &NoAnswers).expect("OPT-bearing query should still get a reply");
        let reply = Packet::parse(&reply_bytes).expect("reply should parse");
        assert_eq!(reply.rcode(), RCODE::NameError);
        // We never echo an OPT back — tolerate and ignore, not negotiate.
        assert!(reply.opt().is_none());
    }

    #[test]
    fn empty_question_section_still_gets_a_bounded_reply() {
        let query = Packet::new_query(0x9);
        let bytes = query.build_bytes_vec().unwrap();

        let reply_bytes = handle_query(&bytes, &NoAnswers).expect("should still build a reply");
        let reply = Packet::parse(&reply_bytes).expect("reply should parse");
        assert_eq!(reply.rcode(), RCODE::NameError);
        assert_eq!(reply.name_servers.len(), 1);
    }

    #[tokio::test]
    async fn responder_binds_and_answers_over_the_wire() {
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let handle = start(bind_addr, Arc::new(NoAnswers))
            .await
            .expect("responder should bind an ephemeral port");

        let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        client.connect(handle.local_addr).await.unwrap();

        // A well-formed query gets NXDOMAIN+SOA.
        let query = build_query("hello.mesh.", TYPE::A);
        client.send(&query).await.unwrap();
        let mut buf = [0u8; RECV_BUF_LEN];
        let n = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut buf))
            .await
            .expect("responder should reply before the timeout")
            .unwrap();
        let reply = Packet::parse(&buf[..n]).unwrap();
        assert_eq!(reply.rcode(), RCODE::NameError);

        // A garbage datagram must not kill the loop — the next well-formed
        // query still gets answered.
        client.send(&[0xFF; 5]).await.unwrap();
        client.send(&query).await.unwrap();
        let n = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut buf))
            .await
            .expect("responder should still be alive after a garbage packet")
            .unwrap();
        let reply = Packet::parse(&buf[..n]).unwrap();
        assert_eq!(reply.rcode(), RCODE::NameError);

        handle.abort();
    }
}

#[cfg(test)]
mod leased_name_table_tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::{Arc, Mutex};

    use super::{wall_now_ms, LeasedNameTable, NameTable, LEASED_NAME_RESOLVE_STALE_MS};
    use crate::crdt::hlc::HLC;
    use crate::crdt::leased_name::{LeasedName, LeasedNameBook};

    fn entry(renewed_ms: u64) -> LeasedName {
        LeasedName {
            owner_pubkey: "aa".repeat(32),
            sig: "00".repeat(64),
            challenge: "ab".repeat(32),
            ip: IpAddr::V4(Ipv4Addr::new(10, 42, 5, 23)),
            port: 3000,
            first_claimed_at: HLC {
                wall_clock: renewed_ms,
                counter: 0,
                node_id: "n".into(),
            },
            renewed_at: HLC {
                wall_clock: renewed_ms,
                counter: 0,
                node_id: "n".into(),
            },
        }
    }

    fn table_with(name: &str, e: LeasedName) -> LeasedNameTable {
        let mut book = LeasedNameBook::new();
        book.insert(name.to_string(), e);
        LeasedNameTable::new(Arc::new(Mutex::new(book)))
    }

    #[test]
    fn freshly_renewed_name_resolves() {
        let t = table_with("walkie-talkie", entry(wall_now_ms()));
        assert_eq!(
            t.lookup_a("walkie-talkie.mesh."),
            Some(vec![Ipv4Addr::new(10, 42, 5, 23)])
        );
        assert!(t.exists("walkie-talkie.mesh."));
        assert_eq!(t.lookup_srv("walkie-talkie.mesh."), Some(3000));
    }

    #[test]
    fn name_stops_resolving_once_heartbeats_lapse() {
        // Last renewal well past the resolve window → absent (NXDOMAIN), even
        // though ownership would still hold for the full hour lease.
        let stale = wall_now_ms().saturating_sub(LEASED_NAME_RESOLVE_STALE_MS + 60_000);
        let t = table_with("walkie-talkie", entry(stale));
        assert_eq!(t.lookup_a("walkie-talkie.mesh."), None);
        assert!(!t.exists("walkie-talkie.mesh."));
        assert_eq!(t.lookup_srv("walkie-talkie.mesh."), None);
    }

    #[test]
    fn resolve_window_boundary_is_pure_and_injectable() {
        let e = entry(1_000_000);
        assert!(LeasedNameTable::resolves(&e, 1_000_000)); // same instant
        assert!(LeasedNameTable::resolves(
            &e,
            1_000_000 + LEASED_NAME_RESOLVE_STALE_MS
        )); // exactly at edge
        assert!(!LeasedNameTable::resolves(
            &e,
            1_000_000 + LEASED_NAME_RESOLVE_STALE_MS + 1
        )); // one ms past
    }

    #[test]
    fn port_zero_is_a_only_no_srv() {
        let mut e = entry(wall_now_ms());
        e.port = 0;
        let t = table_with("nas", e);
        assert!(t.exists("nas.mesh.")); // A still answers
        assert_eq!(t.lookup_srv("nas.mesh."), None); // NODATA for SRV
    }

    #[test]
    fn name_outside_mesh_apex_is_absent() {
        let t = table_with("walkie-talkie", entry(wall_now_ms()));
        assert!(!t.exists("walkie-talkie.example.com."));
        assert_eq!(t.lookup_a("walkie-talkie.example.com."), None);
    }
}
