use iroh::{Endpoint, EndpointId};
use std::collections::HashSet;
use tracing::info;

/// A mesh room — tracks connected peers for full-mesh audio.
pub struct Room {
    pub name: String,
    pub peers: HashSet<EndpointId>,
    endpoint: Endpoint,
}

impl Room {
    pub fn new(name: String, endpoint: Endpoint) -> Self {
        info!(room = %name, "room created");
        Self {
            name,
            peers: HashSet::new(),
            endpoint,
        }
    }

    pub fn add_peer(&mut self, peer: EndpointId) {
        self.peers.insert(peer);
        info!(room = %self.name, %peer, count = self.peers.len(), "peer joined");
    }

    pub fn remove_peer(&mut self, peer: &EndpointId) {
        self.peers.remove(peer);
        info!(room = %self.name, %peer, count = self.peers.len(), "peer left");
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}
