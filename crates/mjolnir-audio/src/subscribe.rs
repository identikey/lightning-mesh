use anyhow::{Context, Result};
use moq_lite::TrackConsumer;
use tracing::{debug, info};

use crate::mixer::PeerInput;

/// Subscribes to a remote audio track and pushes each Opus frame into the
/// peer's jitter buffer with a monotonic local sequence number.
///
/// MoQ preserves frame order within a stream; the local counter is
/// sufficient for the jitter buffer to detect gaps when frames are lost.
pub async fn run_subscribe(mut track: TrackConsumer, peer_input: PeerInput) -> Result<()> {
    let mut seq: u64 = 0;

    info!("audio subscribe pipeline started");

    while let Some(mut group) = track.next_group().await.context("track next_group failed")? {
        while let Some(frame) = group.read_frame().await.context("group read_frame failed")? {
            debug!(seq, len = frame.len(), "received audio frame");
            peer_input.push_frame(seq, frame);
            seq += 1;
        }
    }

    info!("track finished, stopping subscribe");
    Ok(())
}
