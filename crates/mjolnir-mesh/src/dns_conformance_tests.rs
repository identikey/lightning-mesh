//! `.mesh` resolver conformance suite (S4.1, bead mjolnir-mesh-e21.6): a
//! dig-class matrix exercising the full answer-discipline surface promised
//! by the PRD (`docs/prd-mesh-naming-first-stone.md` FR1-FR8) against the
//! real responder wired the way `mjolnir-meshd` wires it — `WellKnownTable`
//! stacked ahead of `ServiceTable` in a `CompositeTable` — over a real bound
//! UDP socket wherever the case benefits from exercising the wire encode/decode
//! path, not just `handle_query` in-process.
//!
//! This module is a sibling of [`crate::dns_responder`], not a `#[cfg(test)]`
//! submodule inside it, so it can grow independently of the unit tests each
//! story left behind there; it reaches `handle_query` via the `pub(crate)`
//! seam added for this story (S4.1) rather than duplicating the parser/dispatch
//! under test.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use simple_dns::rdata::{OPT, RData};
use simple_dns::{Name, Packet, QCLASS, Question, RCODE, TYPE};
use tokio::net::UdpSocket;

use crate::crdt::hlc::HLC;
use crate::crdt::service::{ServiceBookV2, ServiceEntryV2};
use crate::dns_responder::{
    CompositeTable, GatewayHandle, NameTable, PRE_CLAIM_GATEWAY, ServiceTable, WellKnownTable,
    handle_query, start,
};

/// Builds a raw query packet the same way a real client (dnsmasq forwarding
/// a `.mesh` lookup) would.
fn build_query(name: &str, qtype: TYPE) -> Vec<u8> {
    let mut query = Packet::new_query(0x4242);
    query.questions.push(Question::new(
        Name::new(name).unwrap(),
        qtype.into(),
        QCLASS::CLASS(simple_dns::CLASS::IN),
        false,
    ));
    query.build_bytes_vec().unwrap()
}

/// Same as [`build_query`], but with an EDNS0 OPT pseudo-record attached —
/// what a resolver announcing a larger UDP payload size looks like on the wire.
fn build_query_with_opt(name: &str, qtype: TYPE) -> Vec<u8> {
    let mut query = Packet::new_query(0x4343);
    query.questions.push(Question::new(
        Name::new(name).unwrap(),
        qtype.into(),
        QCLASS::CLASS(simple_dns::CLASS::IN),
        false,
    ));
    *query.opt_mut() = Some(OPT {
        opt_codes: Vec::new(),
        udp_packet_size: 4096,
        version: 0,
    });
    query.build_bytes_vec().unwrap()
}

fn service_entry(ip: Ipv4Addr, port: u16, protocol: &str, txt: &[(&str, &str)]) -> ServiceEntryV2 {
    ServiceEntryV2 {
        owner_node_id: "router-conformance".to_string(),
        first_claimed_at: HLC {
            wall_clock: 1,
            counter: 0,
            node_id: "router-conformance".to_string(),
        },
        updated_at: HLC {
            wall_clock: 1,
            counter: 0,
            node_id: "router-conformance".to_string(),
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

/// Builds the same table shape `mjolnir-meshd::run_mesh` wires: well-known
/// names ahead of the CRDT-projected service table, sharing one gateway
/// handle and one service store the caller can mutate after the fact (for
/// the store-mutation-visibility and tombstone cases).
fn full_table(
    gateway: Option<Ipv4Addr>,
) -> (
    Arc<CompositeTable>,
    GatewayHandle,
    Arc<Mutex<ServiceBookV2>>,
) {
    let gateway_handle: GatewayHandle = Arc::new(RwLock::new(gateway));
    let store: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(ServiceBookV2::new()));
    let table = Arc::new(CompositeTable::new(vec![
        Arc::new(WellKnownTable::new(gateway_handle.clone())),
        Arc::new(ServiceTable::new(store.clone())),
    ]));
    (table, gateway_handle, store)
}

/// Spawns a real responder over the given table on an ephemeral loopback
/// port and returns a connected client socket alongside it, so cases can
/// exercise the full recv/parse/dispatch/build/send path rather than calling
/// `handle_query` directly.
async fn spawn_wired_responder(
    table: Arc<dyn NameTable>,
) -> (crate::dns_responder::ResponderHandle, UdpSocket) {
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let handle = start(bind_addr, table)
        .await
        .expect("responder should bind an ephemeral port");
    let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    client.connect(handle.local_addr).await.unwrap();
    (handle, client)
}

/// Sends `query_bytes` and returns the raw reply bytes (the caller parses —
/// `Packet<'_>` borrows from its source buffer, so handing back a `Packet`
/// here would tie the reply to a stack buffer that doesn't outlive this call).
async fn query_over_wire(client: &UdpSocket, query_bytes: &[u8]) -> Vec<u8> {
    client.send(query_bytes).await.unwrap();
    let mut buf = [0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut buf))
        .await
        .expect("responder should reply before the timeout")
        .unwrap();
    buf[..n].to_vec()
}

// --- FR2/D-003: well-known A, pre-claim and claimed ---

#[tokio::test]
async fn well_known_a_answers_preclaim_gateway_over_the_wire() {
    let (table, _gateway, _store) = full_table(None);
    let (handle, client) = spawn_wired_responder(table).await;

    for name in ["hello.mesh.", "id.mesh."] {
        let reply_bytes = query_over_wire(&client, &build_query(name, TYPE::A)).await;
        let reply = Packet::parse(&reply_bytes).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError, "name={name}");
        assert_eq!(reply.answers.len(), 1, "name={name}");
        assert_eq!(reply.answers[0].ttl, 30, "name={name}");
        match &reply.answers[0].rdata {
            RData::A(a) => assert_eq!(Ipv4Addr::from(a.address), PRE_CLAIM_GATEWAY, "name={name}"),
            other => panic!("name={name}: expected A record, got {other:?}"),
        }
    }

    handle.abort();
}

#[tokio::test]
async fn well_known_a_answers_claimed_gateway_over_the_wire() {
    let claimed: Ipv4Addr = "10.42.61.1".parse().unwrap();
    let (table, _gateway, _store) = full_table(Some(claimed));
    let (handle, client) = spawn_wired_responder(table).await;

    for name in ["hello.mesh.", "id.mesh."] {
        let reply_bytes = query_over_wire(&client, &build_query(name, TYPE::A)).await;
        let reply = Packet::parse(&reply_bytes).unwrap();
        assert_eq!(reply.rcode(), RCODE::NoError, "name={name}");
        assert_eq!(reply.answers.len(), 1, "name={name}");
        assert_eq!(reply.answers[0].ttl, 30, "name={name}");
        match &reply.answers[0].rdata {
            RData::A(a) => assert_eq!(Ipv4Addr::from(a.address), claimed, "name={name}"),
            other => panic!("name={name}: expected A record, got {other:?}"),
        }
    }

    handle.abort();
}

// --- case-insensitivity ---

#[tokio::test]
async fn well_known_name_lookup_is_case_insensitive() {
    let claimed: Ipv4Addr = "10.42.61.1".parse().unwrap();
    let (table, _gateway, _store) = full_table(Some(claimed));
    let (handle, client) = spawn_wired_responder(table).await;

    let reply_bytes = query_over_wire(&client, &build_query("HeLLo.MESH.", TYPE::A)).await;
    let reply = Packet::parse(&reply_bytes).unwrap();
    assert_eq!(reply.rcode(), RCODE::NoError);
    assert_eq!(reply.answers.len(), 1);
    match &reply.answers[0].rdata {
        RData::A(a) => assert_eq!(Ipv4Addr::from(a.address), claimed),
        other => panic!("expected A record, got {other:?}"),
    }

    handle.abort();
}

// --- FR3: service A ---

#[tokio::test]
async fn service_a_answers_published_ip() {
    let (table, _gateway, store) = full_table(Some("10.42.61.1".parse().unwrap()));
    store.lock().unwrap().insert(
        "wiki".to_string(),
        service_entry("10.42.61.50".parse().unwrap(), 8080, "_http._tcp", &[]),
    );
    let (handle, client) = spawn_wired_responder(table).await;

    let reply_bytes = query_over_wire(&client, &build_query("wiki.mesh.", TYPE::A)).await;
    let reply = Packet::parse(&reply_bytes).unwrap();
    assert_eq!(reply.rcode(), RCODE::NoError);
    assert_eq!(reply.answers.len(), 1);
    assert_eq!(reply.answers[0].ttl, 30);
    match &reply.answers[0].rdata {
        RData::A(a) => assert_eq!(
            Ipv4Addr::from(a.address),
            "10.42.61.50".parse::<Ipv4Addr>().unwrap()
        ),
        other => panic!("expected A record, got {other:?}"),
    }

    handle.abort();
}

// --- FR5: non-A on an existing name is NODATA, never NXDOMAIN ---
// This is the classic bug this suite exists to pin down: an AAAA (or any
// non-A) query against a name the zone DOES serve must come back
// NOERROR/empty, not NXDOMAIN — conflating "wrong type" with "unknown name"
// would poison every A-only client resolver's cache for the whole name.

#[tokio::test]
async fn aaaa_on_well_known_name_is_nodata_not_nxdomain() {
    let (table, _gateway, _store) = full_table(Some("10.42.61.1".parse().unwrap()));
    let (handle, client) = spawn_wired_responder(table).await;

    let reply_bytes = query_over_wire(&client, &build_query("hello.mesh.", TYPE::AAAA)).await;
    let reply = Packet::parse(&reply_bytes).unwrap();
    assert_eq!(
        reply.rcode(),
        RCODE::NoError,
        "AAAA on an existing name must be NODATA, not NXDOMAIN"
    );
    assert_eq!(reply.answers.len(), 0);
    assert_eq!(
        reply.name_servers.len(),
        0,
        "NODATA carries no SOA in this dispatch"
    );

    handle.abort();
}

#[tokio::test]
async fn mx_on_service_name_is_nodata_not_nxdomain() {
    let (table, _gateway, store) = full_table(Some("10.42.61.1".parse().unwrap()));
    store.lock().unwrap().insert(
        "wiki".to_string(),
        service_entry("10.42.61.50".parse().unwrap(), 8080, "_http._tcp", &[]),
    );
    let (handle, client) = spawn_wired_responder(table).await;

    let reply_bytes = query_over_wire(&client, &build_query("wiki.mesh.", TYPE::MX)).await;
    let reply = Packet::parse(&reply_bytes).unwrap();
    assert_eq!(
        reply.rcode(),
        RCODE::NoError,
        "MX on an existing service name must be NODATA, not NXDOMAIN"
    );
    assert_eq!(reply.answers.len(), 0);

    handle.abort();
}

// --- FR6: SRV/TXT for services ---

#[tokio::test]
async fn service_srv_and_txt_match_the_documented_wire_mapping() {
    let (table, _gateway, store) = full_table(Some("10.42.61.1".parse().unwrap()));
    store.lock().unwrap().insert(
        "printer".to_string(),
        service_entry(
            "10.42.61.60".parse().unwrap(),
            631,
            "_ipp._tcp",
            &[("model", "LaserJet"), ("path", "/ipp/print")],
        ),
    );
    let (handle, client) = spawn_wired_responder(table).await;

    let srv_reply_bytes = query_over_wire(&client, &build_query("printer.mesh.", TYPE::SRV)).await;
    let srv_reply = Packet::parse(&srv_reply_bytes).unwrap();
    assert_eq!(srv_reply.rcode(), RCODE::NoError);
    assert_eq!(srv_reply.answers.len(), 1);
    assert_eq!(srv_reply.answers[0].ttl, 30);
    match &srv_reply.answers[0].rdata {
        RData::SRV(srv) => {
            assert_eq!(srv.priority, 0);
            assert_eq!(srv.weight, 0);
            assert_eq!(srv.port, 631);
            // S1.3's documented mapping: the target is the queried qname
            // itself, self-referential (no separate `_proto` label) — the
            // composite table's own A answer resolves it.
            assert_eq!(srv.target.to_string(), "printer.mesh");
        }
        other => panic!("expected SRV, got {other:?}"),
    }

    let txt_reply_bytes = query_over_wire(&client, &build_query("printer.mesh.", TYPE::TXT)).await;
    let txt_reply = Packet::parse(&txt_reply_bytes).unwrap();
    assert_eq!(txt_reply.rcode(), RCODE::NoError);
    assert_eq!(txt_reply.answers.len(), 1);
    match &txt_reply.answers[0].rdata {
        RData::TXT(txt) => {
            let attrs = txt.attributes();
            assert_eq!(
                attrs.get("model").and_then(|v| v.clone()),
                Some("LaserJet".to_string())
            );
            assert_eq!(
                attrs.get("path").and_then(|v| v.clone()),
                Some("/ipp/print".to_string())
            );
        }
        other => panic!("expected TXT, got {other:?}"),
    }

    handle.abort();
}

// --- FR4/FR7/D-005: unknown name NXDOMAIN + SOA authority ---

#[tokio::test]
async fn unknown_name_is_nxdomain_with_d005_soa_fields() {
    let (table, _gateway, _store) = full_table(Some("10.42.61.1".parse().unwrap()));
    let (handle, client) = spawn_wired_responder(table).await;

    let reply_bytes = query_over_wire(&client, &build_query("nonexistent.mesh.", TYPE::A)).await;
    let reply = Packet::parse(&reply_bytes).unwrap();
    assert_eq!(reply.rcode(), RCODE::NameError);
    assert_eq!(reply.answers.len(), 0);
    assert_eq!(reply.name_servers.len(), 1);
    match &reply.name_servers[0].rdata {
        RData::SOA(soa) => {
            // D-005 fixed MNAME/RNAME/timers; the SOA owner name (zone apex)
            // was left to the implementation and S1.1 shipped "mesh." — no
            // drift to flag, this matches what e21.1.1's bead notes recorded.
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
    assert_eq!(
        reply.name_servers[0].ttl, 30,
        "SOA TTL should match `minimum` per RFC 2308"
    );

    handle.abort();
}

// --- FR1/NFR1: EDNS0 tolerated, not negotiated, response bounded ---

#[tokio::test]
async fn edns0_query_is_answered_without_echoing_opt_and_stays_bounded() {
    let (table, _gateway, store) = full_table(Some("10.42.61.1".parse().unwrap()));
    store.lock().unwrap().insert(
        "wiki".to_string(),
        service_entry("10.42.61.50".parse().unwrap(), 8080, "_http._tcp", &[]),
    );
    let (handle, client) = spawn_wired_responder(table).await;

    client
        .send(&build_query_with_opt("wiki.mesh.", TYPE::A))
        .await
        .unwrap();
    let mut buf = [0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut buf))
        .await
        .expect("responder should reply before the timeout")
        .unwrap();
    assert!(
        n <= 512,
        "reply must stay within the classic 512B UDP ceiling, got {n}"
    );
    let reply = Packet::parse(&buf[..n]).unwrap();
    assert_eq!(reply.rcode(), RCODE::NoError);
    assert_eq!(reply.answers.len(), 1);
    assert!(
        reply.opt().is_none(),
        "responder tolerates EDNS0 but must never echo an OPT back"
    );

    handle.abort();
}

// --- oversized/garbage/truncated datagrams never take the loop down ---

#[tokio::test]
async fn garbage_and_oversized_datagrams_never_kill_the_recv_loop() {
    let (table, _gateway, _store) = full_table(Some("10.42.61.1".parse().unwrap()));
    let (handle, client) = spawn_wired_responder(table).await;

    // Empty datagram.
    client.send(&[]).await.unwrap();
    // A handful of single-byte and short garbage frames.
    client.send(&[0xFF]).await.unwrap();
    client.send(&[0xAA; 3]).await.unwrap();
    // A header claiming a question section that never actually follows
    // (truncated body) — parses far enough to see a QDCOUNT but has no
    // question bytes behind it.
    let mut truncated_header = [0u8; 12];
    truncated_header[5] = 1;
    client.send(&truncated_header).await.unwrap();
    // A big, structurally-nonsensical blob well past a normal query's size.
    client.send(&[0x5A; 4000]).await.unwrap();

    // None of the above should have produced a reply (all are unparseable/
    // droppable) and, critically, none of them should have killed the loop:
    // a well-formed query sent right after must still be answered.
    let reply_bytes = query_over_wire(&client, &build_query("hello.mesh.", TYPE::A)).await;
    let reply = Packet::parse(&reply_bytes).unwrap();
    assert_eq!(
        reply.rcode(),
        RCODE::NoError,
        "responder must still be alive after garbage datagrams"
    );
    assert_eq!(reply.answers.len(), 1);

    handle.abort();
}

// --- FR8: store mutation visible on the very next query, no cache ---

#[tokio::test]
async fn store_mutation_is_visible_on_the_next_query() {
    let (table, _gateway, store) = full_table(Some("10.42.61.1".parse().unwrap()));
    let (handle, client) = spawn_wired_responder(table).await;

    let before_bytes = query_over_wire(&client, &build_query("printer.mesh.", TYPE::A)).await;
    let before = Packet::parse(&before_bytes).unwrap();
    assert_eq!(before.rcode(), RCODE::NameError, "not yet published");

    store.lock().unwrap().insert(
        "printer".to_string(),
        service_entry("10.42.61.70".parse().unwrap(), 631, "_ipp._tcp", &[]),
    );

    let after_bytes = query_over_wire(&client, &build_query("printer.mesh.", TYPE::A)).await;
    let after = Packet::parse(&after_bytes).unwrap();
    assert_eq!(
        after.rcode(),
        RCODE::NoError,
        "must answer from the mutated store with no reload step"
    );
    assert_eq!(after.answers.len(), 1);
    match &after.answers[0].rdata {
        RData::A(a) => assert_eq!(
            Ipv4Addr::from(a.address),
            "10.42.61.70".parse::<Ipv4Addr>().unwrap()
        ),
        other => panic!("expected A record, got {other:?}"),
    }

    handle.abort();
}

// --- tombstoned/removed service reverts to NXDOMAIN ---

#[tokio::test]
async fn removed_service_reverts_to_nxdomain() {
    let (table, _gateway, store) = full_table(Some("10.42.61.1".parse().unwrap()));
    store.lock().unwrap().insert(
        "printer".to_string(),
        service_entry("10.42.61.70".parse().unwrap(), 631, "_ipp._tcp", &[]),
    );
    let (handle, client) = spawn_wired_responder(table).await;

    let published_bytes = query_over_wire(&client, &build_query("printer.mesh.", TYPE::A)).await;
    let published = Packet::parse(&published_bytes).unwrap();
    assert_eq!(published.rcode(), RCODE::NoError);

    store.lock().unwrap().remove("printer");

    let removed_bytes = query_over_wire(&client, &build_query("printer.mesh.", TYPE::A)).await;
    let removed = Packet::parse(&removed_bytes).unwrap();
    assert_eq!(
        removed.rcode(),
        RCODE::NameError,
        "a removed/tombstoned service must go back to NXDOMAIN"
    );
    assert_eq!(
        removed.name_servers.len(),
        1,
        "NXDOMAIN still carries the SOA authority record"
    );

    handle.abort();
}

// --- in-process sanity: handle_query directly, no socket, over the exact
// composite shape mjolnir-meshd wires (belt-and-suspenders against the
// over-the-wire cases above, cheap enough to keep both) ---

#[test]
fn handle_query_in_process_matches_wired_composite_shape() {
    let (table, _gateway, store) = full_table(Some("10.42.61.1".parse().unwrap()));
    store.lock().unwrap().insert(
        "wiki".to_string(),
        service_entry("10.42.61.50".parse().unwrap(), 8080, "_http._tcp", &[]),
    );

    let hello = handle_query(&build_query("hello.mesh.", TYPE::A), table.as_ref()).unwrap();
    assert_eq!(Packet::parse(&hello).unwrap().rcode(), RCODE::NoError);

    let wiki = handle_query(&build_query("wiki.mesh.", TYPE::A), table.as_ref()).unwrap();
    assert_eq!(Packet::parse(&wiki).unwrap().rcode(), RCODE::NoError);

    let ghost = handle_query(&build_query("ghost.mesh.", TYPE::A), table.as_ref()).unwrap();
    assert_eq!(Packet::parse(&ghost).unwrap().rcode(), RCODE::NameError);
}
