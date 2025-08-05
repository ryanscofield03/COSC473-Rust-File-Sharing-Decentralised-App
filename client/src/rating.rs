use libp2p::PeerId;
use serde::{Deserialize, Serialize};

/// Stores a positive/negative rating
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) enum RatingType {
    Positive, Negative
}

/// Stores all necessary data for a rating
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Rating {
    pub(crate) rating: RatingType,
    pub(crate) rater_peer_id: PeerId,
    pub(crate) rated_peer_id: PeerId,
    pub(crate) timestamp: u64,
}