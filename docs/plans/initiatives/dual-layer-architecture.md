# Option C: Dual-Layer Agent Communication Architecture

(this doc is also found in the main mjolnir repo)

## MCP Control Plane + WebRTC Data Plane

**Status:** Proposal
**Date:** 2026-02-26

---

## 1. Executive Summary

Mjolnir's agent communication architecture will be built on two complementary protocol layers, each chosen for what it does best:

1. **MCP (Model Context Protocol)** as the public-facing control plane -- the "front door" through which any AI agent (Claude, GPT, custom toolchains) can discover Mjolnir's capabilities, spawn VMs, execute commands, manage snapshots, and orchestrate workflows. MCP uses Streamable HTTP transport (JSON-RPC 2.0 over HTTP POST/GET with optional SSE streaming).

2. **WebRTC mesh** as the internal data plane -- the "nervous system" for direct peer-to-peer connections between VMs, between agents, and between Mjolnir nodes. WebRTC data channels carry real-time binary streams, VM-to-VM messaging, and distributed compute coordination.

The architectural insight is separation of concerns at the protocol level: MCP handles structured request/response semantics with tool discovery and rich typing, while WebRTC handles high-throughput, low-latency peer-to-peer data flow. Neither protocol alone covers both needs well. Together, they create a system where agents can discover and control infrastructure through a universal standard, while the infrastructure itself communicates through the fastest available path.

---

## 2. Why Two Layers

### The Problem with One Protocol

A single protocol for all agent communication forces compromises:

- **HTTP-only** (what we have today): Works for request/response, but cannot support real-time VM-to-VM streams, has no peer discovery, and requires all traffic to route through the Mjolnir host. Every inter-VM message takes the path VM-A -> vsock -> host -> vsock -> VM-B.

- **WebRTC-only**: Excellent for peer-to-peer data, but has no standard for tool discovery, capability enumeration, or structured agent interaction. Every AI framework would need a custom integration.

- **Custom protocol**: Maximum flexibility, zero ecosystem leverage. Every agent integration is bespoke work.

### The Dual-Layer Solution

```
+------------------------------------------------------------------+
|                        AGENT LAYER                                |
|  Claude, GPT, custom agents, human operators                     |
+------------------------------------------------------------------+
         |                                    |
         | MCP (Streamable HTTP)              | WebRTC (DataChannel)
         | Control plane                      | Data plane
         | "What to do"                       | "Moving the bytes"
         |                                    |
+--------v---------+              +-----------v-----------+
|                   |              |                       |
|  MCP Server       |   signals   |  WebRTC Mesh          |
|  (Mjolnir)        |<----------->|  (mjolnir-mesh)       |
|                   |              |                       |
|  - Tool calls     |              |  - VM-to-VM streams   |
|  - Resources      |              |  - Agent-to-agent     |
|  - Prompts        |              |  - Binary data        |
|  - Task tracking  |              |  - Real-time events   |
|                   |              |                       |
+--------+----------+              +-----------+-----------+
         |                                     |
         | vsock / HTTP                        | DataChannel
         |                                     |
+--------v---------+              +------------v----------+
|  VM GenServers    |              |  VM Guest Agents      |
|  (Elixir/OTP)    |              |  (Rust/tokio)         |
+-------------------+              +-----------------------+
```

**MCP** answers: "What can Mjolnir do? Spawn a VM. Run this command. Show me the snapshots. What's the cluster health?" -- structured, discoverable, typed.

**WebRTC** answers: "Stream this PTY output at 60fps. Pipe VM-A's stdout into VM-B's stdin. Broadcast a coordination signal to all agents in the mesh." -- fast, direct, peer-to-peer.

---

## 3. Layer 1: MCP as the Public Agent API

### 3.1 What MCP Provides

The Model Context Protocol (spec version 2025-03-26, with updates through 2025-11-25) defines a standard for LLM applications to interact with external systems through three primitives:

- **Tools**: Executable actions the agent can invoke (spawn a VM, run a command)
- **Resources**: Structured data the agent can read (VM status, snapshot list, cluster health)
- **Prompts**: Pre-defined interaction templates (guided VM setup, debugging workflows)

MCP uses JSON-RPC 2.0 as its message format. The Streamable HTTP transport exposes a single endpoint (e.g., `https://mjolnir.example.com/mcp`) that accepts POST for client-to-server messages and GET for server-initiated SSE streams. Session state is tracked via the `Mcp-Session-Id` header.

### 3.2 Transport: Streamable HTTP

The Streamable HTTP transport (which replaced the deprecated HTTP+SSE transport) operates as follows:

```
Client                                     Mjolnir MCP Server
  |                                              |
  |  POST /mcp                                   |
  |  { "jsonrpc": "2.0",                         |
  |    "method": "initialize", ... }             |
  |--------------------------------------------->|
  |                                              |
  |  200 OK                                      |
  |  Mcp-Session-Id: <uuid>                      |
  |  { "jsonrpc": "2.0",                         |
  |    "result": { "capabilities": ... } }       |
  |<---------------------------------------------|
  |                                              |
  |  POST /mcp (tool call)                       |
  |  Mcp-Session-Id: <uuid>                      |
  |  Accept: application/json, text/event-stream |
  |--------------------------------------------->|
  |                                              |
  |  Content-Type: text/event-stream             |
  |  (SSE stream for long-running operations)    |
  |  data: { progress... }                       |
  |  data: { progress... }                       |
  |  data: { "jsonrpc": "2.0", "result": ... }   |
  |<---------------------------------------------|
  |                                              |
  |  GET /mcp (server-initiated notifications)   |
  |  Mcp-Session-Id: <uuid>                      |
  |--------------------------------------------->|
  |                                              |
  |  text/event-stream                           |
  |  data: { vm_event... }                       |
  |  data: { vm_event... }                       |
  |<---------------------------------------------|
```

Key properties:
- **Single endpoint**: All MCP traffic goes through one URL path
- **Session management**: `Mcp-Session-Id` header tracks client sessions; cryptographically secure UUIDs
- **Flexible response**: Server can return plain JSON for fast responses, or open an SSE stream for long-running operations (VM boot progress, command output streaming)
- **Resumability**: SSE events can carry IDs enabling reconnection without message loss
- **Session termination**: Clients send HTTP DELETE to clean up; server responds 404 to expired sessions

### 3.3 Mjolnir MCP Tools

Every existing Mjolnir API endpoint maps naturally to an MCP tool. The mapping from the current REST API (`/Users/dukejones/work/Mjolnir/mjolnir/lib/mjolnir/api/router.ex`) is direct:

| MCP Tool Name | Current REST Endpoint | Description | Input Schema |
|---|---|---|---|
| `spawn_vm` | `POST /api/vms` | Create a new microVM | `{ base_image?, memory_mb?, vcpus?, snapshot?, rootfs_size_mb?, enable_iroh?, ssh_public_key? }` |
| `list_vms` | `GET /api/vms` | List all running VMs | `{}` |
| `get_vm` | `GET /api/vms/:id` | Get VM details and status | `{ vm_id }` |
| `exec` | `POST /api/vms/:id/exec` | Execute a command in a VM | `{ vm_id, command, timeout? }` |
| `stop_vm` | `DELETE /api/vms/:id` | Stop and clean up a VM | `{ vm_id }` |
| `create_snapshot` | `POST /api/vms/:id/snapshots` | Snapshot a running VM's filesystem | `{ vm_id, name, compact? }` |
| `list_snapshots` | `GET /api/snapshots` | List all available snapshots | `{}` |
| `get_snapshot` | `GET /api/snapshots/:name` | Get snapshot metadata | `{ name }` |
| `delete_snapshot` | `DELETE /api/snapshots/:name` | Delete a snapshot | `{ name }` |
| `deliver_message` | `POST /api/vms/:id/messages` | Send a message to a VM (inter-VM / wake-up) | `{ vm_id, from_vm_id?, payload }` |
| `list_dormant` | `GET /api/dormant` | List dormant (checkpointed) VMs | `{}` |
| `get_connection_ticket` | `GET /api/vms/:id/ticket` | Get Iroh connection ticket for a VM | `{ vm_id }` |
| `await_pty` | `POST /api/vms/:id/await-pty` | Wait for PTY/shell readiness | `{ vm_id, timeout? }` |

Additionally, tools that do not yet exist in the REST API but are natural MCP extensions:

| MCP Tool Name | Description | Notes |
|---|---|---|
| `restore_dormant` | Wake a dormant VM from its snapshot | Currently triggered implicitly by `deliver_message`; deserves explicit exposure |
| `clone_vm` | Clone a running VM via BTRFS reflink | Filesystem clone -- instant, shares blocks with original |
| `get_mesh_peers` | List WebRTC mesh peers and connectivity | Bridge between MCP control plane and WebRTC data plane |
| `join_mesh` | Connect a VM or agent to the WebRTC mesh | Returns signaling credentials |

### 3.4 Mjolnir MCP Resources

Resources are read-only data that agents can pull for context. Unlike tools (which perform actions), resources provide state:

| Resource URI | Description | MIME Type |
|---|---|---|
| `mjolnir://vms` | List of all VMs with status summary | `application/json` |
| `mjolnir://vms/{id}` | Full state of a specific VM | `application/json` |
| `mjolnir://vms/{id}/logs` | Recent VM event log (boot, exec, errors) | `text/plain` |
| `mjolnir://snapshots` | All available snapshots with metadata | `application/json` |
| `mjolnir://dormant` | Dormant VMs with pending message counts | `application/json` |
| `mjolnir://cluster/health` | Cluster health: nodes, VM count, resource usage | `application/json` |
| `mjolnir://mesh/topology` | WebRTC mesh topology: peers, connections, latency | `application/json` |

Resources support **subscriptions** -- an agent can subscribe to `mjolnir://vms` and receive notifications via SSE when VMs are created, destroyed, or change state. This maps directly to Mjolnir's existing `EventBus` module.

### 3.5 MCP Prompts

Prompts are pre-built interaction templates for common workflows:

| Prompt Name | Description |
|---|---|
| `setup_dev_environment` | Guided VM creation with language runtime, editor, and tools |
| `debug_vm` | Diagnostic sequence: check status, inspect logs, test connectivity |
| `snapshot_workflow` | Create snapshot, verify integrity, optionally clone |
| `multi_agent_deploy` | Spawn N VMs, distribute tasks, collect results |

### 3.6 Authentication

MCP Streamable HTTP inherits standard HTTP authentication mechanisms. Mjolnir already has JWT/OIDC authentication (`Mjolnir.API.Auth`), which maps cleanly:

- **Bearer tokens** in the `Authorization` header on every MCP request
- **Scope-based authorization** (`vms:spawn`, `vms:exec`, `snapshots:create`, etc.) checked per tool call
- **OIDC discovery** via the existing Keycloak integration for federated identity
- **Localhost bypass** preserved for development

The `Mcp-Session-Id` header provides session affinity. For multi-tenant deployments, the JWT `sub` claim determines which VMs and snapshots are visible.

---

## 4. Layer 2: WebRTC Mesh as the Internal Nervous System

### 4.1 What WebRTC Provides

WebRTC data channels give us:

- **Peer-to-peer connections** that bypass the Mjolnir host for VM-to-VM communication
- **NAT traversal** via ICE (STUN/TURN), complementing Iroh's relay-based traversal
- **Ordered and unordered delivery** modes per channel
- **Binary frames** with no serialization overhead
- **DTLS encryption** on every connection
- **Browser compatibility** for web-based agent UIs

### 4.2 The Signaling Server (mjolnir-mesh)

The WebRTC signaling server already exists as `mjolnir-mesh` (Elysia/Bun). It handles:

```
+------------------+
|  mjolnir-mesh    |
|  (Elysia/Bun)    |
|                  |
|  - Peer registry |
|  - SDP exchange  |
|  - ICE relay     |
|  - Room mgmt     |
+--------+---------+
         |
    WebSocket signaling
         |
    +----+----+----+----+
    |    |    |    |    |
   VM1  VM2  VM3 Agent Agent
    |              |    |
    +----- DataChannel -+
    (direct peer-to-peer)
```

Once signaling completes, peers communicate directly. The signaling server is only involved in connection setup, not in data transfer.

### 4.3 Mesh Topology

For Mjolnir's use case, a **full mesh** topology is appropriate for small-to-medium clusters (up to 20-30 peers). Each peer maintains direct data channel connections to every other peer:

```
    VM-A -------- VM-B
     | \        / |
     |  \      /  |
     |   \    /   |
     |    \  /    |
     |     \/     |
     |     /\     |
     |    /  \    |
     |   /    \   |
     |  /      \  |
     | /        \ |
    VM-C -------- Agent-1
```

For larger deployments, a **superpeer** pattern applies: designated relay nodes (likely the Mjolnir host nodes themselves) act as forwarding hubs, reducing the O(n^2) connection count to O(n).

### 4.4 Data Channel Protocols

WebRTC data channels within the mesh carry typed messages. Each channel is labeled with its purpose:

| Channel Label | Mode | Content | Use Case |
|---|---|---|---|
| `control` | Ordered, reliable | JSON messages | Coordination, state sync, heartbeats |
| `pty:{session_id}` | Ordered, reliable | Binary terminal data | Remote shell sessions between peers |
| `pipe:{pipe_id}` | Ordered, reliable | Binary stream | Unix-pipe-style data flow between VMs |
| `broadcast` | Unordered, unreliable | JSON events | Cluster-wide notifications, discovery |
| `bulk:{transfer_id}` | Ordered, reliable | Binary chunks | File transfer, snapshot distribution |

This channel labeling maps to Mjolnir's existing protocol philosophy of channel multiplexing (see `Mjolnir.Vsock.Protocol` which already uses 1-byte channel IDs for multiplexing control vs. PTY data).

### 4.5 Authentication at the Signaling Level

WebRTC authentication happens during signaling, before any data channel is established:

1. Peer connects to `mjolnir-mesh` via WebSocket
2. Peer presents a JWT (same token used for MCP, same Keycloak/OIDC issuer)
3. Signaling server validates the token and extracts identity + scopes
4. Only authorized peers can exchange SDP offers/answers
5. Once the data channel is established, DTLS provides transport encryption

This means a single identity system (JWT/OIDC) governs both the MCP control plane and the WebRTC data plane.

---

## 5. How the Layers Interact

### 5.1 The Bridge Pattern

The MCP server and WebRTC mesh are not isolated -- they are bridged through the Mjolnir orchestration layer (Elixir/OTP). The bridge pattern works as follows:

```
Agent (Claude, GPT, etc.)
  |
  |  MCP tool call: spawn_vm
  |
  v
Mjolnir MCP Server (Elixir)
  |
  |  1. Spawn VM via Mjolnir.VM.spawn/1
  |  2. VM boots, guest agent starts
  |  3. Guest agent joins WebRTC mesh
  |  4. Return VM ID + mesh peer ID to agent
  |
  v
Agent receives:
  {
    "vm_id": "abc-123",
    "mesh_peer_id": "peer_abc123",
    "status": "running"
  }
  |
  |  Agent can now:
  |  - Use MCP tools for structured operations (exec, snapshot)
  |  - Connect to WebRTC mesh for real-time data (PTY, streams)
  |
```

### 5.2 Concrete Interaction Flows

**Flow 1: Agent spawns a VM and gets a shell**

```
Agent                MCP Server           VM              Mesh
  |                      |                 |                |
  |-- spawn_vm --------->|                 |                |
  |                      |-- VM.spawn() -->|                |
  |                      |                 |-- boot ------->|
  |                      |                 |-- join mesh -->|
  |<-- { vm_id, ... } --|                 |                |
  |                      |                 |                |
  |-- get_mesh_peers --->|                 |                |
  |<-- { peers: [...] } -|                 |                |
  |                      |                 |                |
  |-- [WebRTC signal] ----------------------------- offer ->|
  |<- [WebRTC signal] ----------------------------- answer -|
  |                      |                 |                |
  |== DataChannel "pty:1" ========================= PTY ====|
  |   (direct peer-to-peer, bypasses MCP server)            |
```

**Flow 2: Multi-VM coordination**

```
Agent                MCP Server           VM-A    VM-B    Mesh
  |                      |                 |        |       |
  |-- spawn_vm x2 ------>|                 |        |       |
  |<-- [vm_a, vm_b] -----|                 |        |       |
  |                      |                 |        |       |
  |-- exec(vm_a, "produce data") -------->|        |       |
  |<-- { output: ... } --|                 |        |       |
  |                      |                 |        |       |
  |  (Agent decides VM-B should process VM-A's output)      |
  |  (Two options:)                                         |
  |                      |                 |        |       |
  |  Option A: MCP (structured, auditable)                  |
  |-- deliver_message(vm_b, payload) ---->|------->|       |
  |                      |                 |        |       |
  |  Option B: WebRTC (fast, direct)                        |
  |  VM-A ==[DataChannel "pipe:1"]================>|       |
  |  (peer-to-peer, no host involvement)                    |
```

**Flow 3: Dormant VM wake-up via MCP, communication via WebRTC**

```
Agent                MCP Server         DormantRegistry    Mesh
  |                      |                    |              |
  |-- deliver_message -->|                    |              |
  |   (to dormant VM)    |-- queue_message -->|              |
  |                      |-- begin_restore -->|              |
  |                      |-- VM.spawn() ---->  (VM boots)    |
  |                      |                    |-- join mesh ->|
  |<-- { ok: true } -----|                    |              |
  |                      |                    |              |
  |  (VM is now live in the mesh)                            |
  |== DataChannel ==========================================|
```

### 5.3 When to Use Which Layer

| Scenario | Use MCP | Use WebRTC | Why |
|---|---|---|---|
| Spawn/stop VMs | Yes | -- | Lifecycle management is structured control |
| Execute a command | Yes | -- | Request/response with typed output |
| Stream PTY output | -- | Yes | Real-time binary, latency-sensitive |
| VM-to-VM data pipe | -- | Yes | Direct peer path, no host bottleneck |
| List snapshots | Yes | -- | Read-only query, typed response |
| Broadcast event to all VMs | -- | Yes | Fan-out to mesh, unreliable OK |
| Create snapshot | Yes | -- | Orchestrated multi-step operation |
| Transfer file between VMs | -- | Yes | Bulk binary, direct peer path |
| Agent discovers capabilities | Yes | -- | MCP's core purpose: tool/resource discovery |
| Health monitoring | Both | Both | MCP for queries, WebRTC for real-time heartbeats |

---

## 6. Implementation Path

### Phase 1: MCP Server (Elixir)

Build an MCP server module within the existing Mjolnir Elixir application. This is a new Plug pipeline that sits alongside the existing REST API.

**Work items:**
- Implement `Mjolnir.MCP.Router` -- a Plug that handles the `/mcp` endpoint
- JSON-RPC 2.0 message parsing and dispatch
- Session management with `Mcp-Session-Id` headers
- SSE streaming for long-running tool calls (VM boot progress)
- Tool registry mapping tool names to `Mjolnir.VM` and `Mjolnir.BTRFS` function calls
- Resource registry with EventBus-driven subscription notifications
- Authentication: reuse `Mjolnir.API.Auth` (JWT validation, scope checking)

**Key design decision:** The MCP server is a thin translation layer. It does not duplicate business logic. Every tool call delegates to the same `Mjolnir.VM`, `Mjolnir.BTRFS`, and `Mjolnir.DormantRegistry` modules that the REST API uses. This means:

```
                   +-- REST API (/api/vms, /api/snapshots, ...)
                   |
Mjolnir Core ------+
(VM, BTRFS,        |
 DormantRegistry)  +-- MCP Server (/mcp)
                   |
                   +-- IEx console (direct function calls)
```

**Estimated scope:** ~800-1200 lines of Elixir. The JSON-RPC framing, SSE streaming, and session management are the bulk of the work. Tool implementations are thin wrappers.

### Phase 2: WebRTC Mesh Integration

Integrate the existing `mjolnir-mesh` signaling server with the Mjolnir orchestration layer and guest agent.

**Work items:**
- Guest agent (Rust): Add WebRTC data channel support via `webrtc-rs` or `str0m`
- Signaling client in guest agent: connect to `mjolnir-mesh` WebSocket on boot
- Elixir side: Register mesh peer IDs in VM state alongside existing Iroh ticket
- Data channel protocol: Define frame format for `control`, `pty`, `pipe`, and `bulk` channels
- MCP bridge tools: `get_mesh_peers`, `join_mesh`

**Relationship to Iroh:** WebRTC and Iroh serve overlapping but distinct roles. Iroh provides NAT-traversing QUIC connections with content-addressed data. WebRTC provides browser-compatible peer-to-peer connections with standard ICE/STUN/TURN traversal. In the near term, both coexist -- Iroh for the existing shell client (`mjolnir connect`), WebRTC for mesh coordination and browser-based agents. Long term, the mesh may converge on one transport, but the signaling and channel abstractions remain the same regardless.

### Phase 3: Agent Workflow Primitives

Build higher-level primitives on top of the dual-layer foundation.

**Work items:**
- MCP prompt templates for common multi-VM workflows
- Task tracking (MCP 2025-11-25 spec): long-running operations return task IDs that agents can poll
- Agent-to-agent messaging over the mesh with delivery guarantees
- Workflow orchestration: agent spawns N VMs, distributes work, collects results, snapshots successful runs

---

## 7. Architectural Properties

### 7.1 Standards Compliance

Both layers are built on open standards:
- MCP: JSON-RPC 2.0, HTTP, SSE -- every AI framework already speaks these
- WebRTC: ICE, DTLS, SCTP -- every browser and most runtimes have native support

No proprietary protocols. No vendor lock-in. An agent written for OpenAI's function calling can talk to Mjolnir's MCP server with minimal adaptation. A web browser can join the mesh without plugins.

### 7.2 Failure Isolation

The two layers fail independently:
- If the MCP server goes down, existing WebRTC data channels continue operating (they are peer-to-peer)
- If the WebRTC signaling server goes down, existing mesh connections survive (signaling is only needed for setup); MCP control operations continue normally
- If a VM crashes, the Elixir supervision tree cleans up its MCP session and mesh registrations

### 7.3 Performance Characteristics

| Metric | MCP (Control Plane) | WebRTC (Data Plane) |
|---|---|---|
| Latency | 10-50ms (HTTP round-trip) | 1-5ms (direct peer, same host) |
| Throughput | ~1000 req/s (JSON-RPC) | ~1 Gbps (data channel, LAN) |
| Connection setup | ~100ms (HTTP) | ~500ms (ICE + DTLS handshake) |
| Message overhead | ~200 bytes (JSON-RPC envelope) | ~28 bytes (SCTP + DTLS header) |
| NAT traversal | Standard HTTP (works everywhere) | ICE with STUN/TURN fallback |

### 7.4 Relationship to Existing Architecture

This dual-layer design does not replace Mjolnir's existing components. It layers on top:

```
+---------------------------------------------------------------+
|  NEW: MCP Server          |  NEW: WebRTC Mesh Integration     |
|  (public agent API)       |  (peer-to-peer data plane)        |
+---------------------------+-----------------------------------+
|  EXISTING: REST API       |  EXISTING: Iroh QUIC              |
|  (HTTP, JWT auth)         |  (NAT traversal, shell access)    |
+---------------------------+-----------------------------------+
|  EXISTING: Elixir/OTP Orchestration                           |
|  VM GenServer, DynamicSupervisor, Registry, EventBus          |
+---------------------------------------------------------------+
|  EXISTING: Vsock Protocol (host <-> guest)                    |
+---------------------------------------------------------------+
|  EXISTING: Hypervisor (Firecracker / Cloud Hypervisor)        |
+---------------------------------------------------------------+
|  EXISTING: BTRFS (CoW filesystem cloning)                     |
+---------------------------------------------------------------+
```

The REST API continues to work. Existing shell access via Iroh continues to work. MCP and WebRTC are additive capabilities, not replacements.

---

## 8. Security Model

### 8.1 Unified Identity

A single JWT/OIDC identity system governs both layers:

```
Keycloak / OIDC Provider
         |
         | JWT with scopes
         |
    +----+----+
    |         |
    v         v
MCP Server   WebRTC Signaling
(validate    (validate token
 Bearer       on WebSocket
 header)      connect)
```

Scopes are shared: `vms:spawn` authorizes both the MCP `spawn_vm` tool and the ability to register as a mesh peer for that VM.

### 8.2 Transport Security

- **MCP**: HTTPS (TLS 1.3) for all Streamable HTTP traffic
- **WebRTC**: DTLS for all data channel traffic; certificates exchanged during ICE
- **Vsock**: Inherently local (host-guest only), no network exposure

### 8.3 Origin Validation

Per the MCP specification, the server must validate the `Origin` header on all incoming connections to prevent DNS rebinding attacks. When running locally, the MCP endpoint binds only to localhost.

---

## 9. Open Questions

1. **Iroh + WebRTC coexistence**: Should the guest agent maintain both an Iroh endpoint and a WebRTC peer connection? Or should WebRTC subsume Iroh's role for shell access? The answer likely depends on whether browser-based shell access (which WebRTC enables natively) is a priority.

2. **MCP server library**: Should we use an existing Elixir MCP library (if one exists with sufficient maturity) or implement the JSON-RPC + SSE framing ourselves? The protocol is simple enough that a from-scratch implementation in ~800 lines is feasible and avoids dependency risk.

3. **Mesh persistence**: When a VM goes dormant, should its mesh peer state be preserved so it can rejoin with the same peer ID on restore? This would enable seamless reconnection for agents holding stale peer references.

4. **Backpressure**: WebRTC data channels have built-in flow control (SCTP), but MCP over SSE does not. For long-running tool calls that produce large output (e.g., `exec` with verbose commands), how do we handle backpressure in the SSE stream?

5. **Multi-node mesh topology**: When Mjolnir runs as a cluster (multiple Elixir nodes via Erlang distribution), should each node run its own signaling server, or should there be a single federated signaling service? The superpeer pattern suggests designated relay nodes, which aligns with having signaling on each Mjolnir host.

---

## 10. References

- [MCP Specification (2025-03-26) - Transports](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports)
- [MCP Specification (2025-11-25) - Latest](https://modelcontextprotocol.io/specification/2025-11-25)
- [Why MCP Deprecated SSE for Streamable HTTP](https://blog.fka.dev/blog/2025-06-06-why-mcp-deprecated-sse-and-go-with-streamable-http/)
- [MCP Streamable HTTP Security (Auth0)](https://auth0.com/blog/mcp-streamable-http/)
- [WebRTC Network Topology Guide](https://dev.to/akeel_almas_9a2ada3db4257/webrtc-network-topology-complete-guide-to-mesh-sfu-and-mcu-architecture-selection-published-by-3fi6)
- [WebRTC Mesh Networks and Zero-Trust Architecture](https://dev.to/j3rryh0well/building-peersuite-webrtc-mesh-networks-and-zero-trust-architecture-4f0h)
- [WebRTC Scalability Guide 2025](https://antmedia.io/webrtc-scalability/)
- Mjolnir internal: `/docs/architecture.md`
- Mjolnir internal: `/docs/computational-fabric.md`
- Mjolnir internal: `/.omc/plans/mjolnir-architecture-transition.md`
