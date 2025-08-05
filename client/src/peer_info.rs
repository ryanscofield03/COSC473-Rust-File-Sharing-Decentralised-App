use std::cmp::min;
use std::collections::HashMap;
use std::ops::Sub;
use chrono::{DateTime, Local, TimeDelta};
use libp2p::PeerId;
use tokio::time::Instant;
use crate::messages::{format_local_time, Message, MessageType};

/// stores relevant information for a peer
#[derive(Debug, Clone)]
pub(crate) struct PeerInfo {
    pub(crate) username: Option<String>,
    pub(crate) rating: i16,
    pub(crate) dms: Vec<Message>,
    pub(crate) dm_open: bool,
    pub(crate) last_read: Option<DateTime<Local>>,
    pub(crate) last_updated: DateTime<Local>,
}

impl PeerInfo {
    /// default method for peer info if we don't have all info
    pub(crate) fn default() -> Self {
        PeerInfo {
            username: None,
            rating: 0,
            dms: Vec::new(),
            dm_open: false,
            last_read: None,
            last_updated: DateTime::<Local>::from(Local::now()).sub(TimeDelta::minutes(1)),
        }
    }
}

/// helper function to get the username of a peer from peer_info_hashmap
pub(crate) fn get_username(peer_info_hashmap: HashMap<PeerId, PeerInfo>, peer_id: PeerId) -> Option<String> {
    peer_info_hashmap.get(&peer_id).cloned()?.username
}

/// helper function to get the dms with a peer from peer_info_hashmap
pub(crate) fn get_direct_messages(peer_info_hashmap: HashMap<PeerId, PeerInfo>, peer_id: PeerId) -> Vec<Message> {
    peer_info_hashmap.get(&peer_id).cloned().unwrap_or(PeerInfo::default()).dms
}

/// helper function to get a list of peer ids, rating, and usernames mapping
/// (REMOVES USERS WHOSE NAMES ARE UNKNOWN)
pub(crate) fn get_list_of_peer_data(peer_info_hashmap: HashMap<PeerId, PeerInfo>) -> Vec<String> {
    peer_info_hashmap
        .iter()
        .filter(|(_, peer_info)| peer_info.username.is_some())
        .map(|(other_peer_id, peer_info)| {
            let peer_id = other_peer_id.to_string();
            let username = peer_info.username.clone().unwrap();
            let shortened_peer_id = &peer_id[0..min(peer_id.len(), 20)];
            let rating = peer_info.rating.clone();

            format!("Username: {}, PeerId: {}, Rating: {}", username, shortened_peer_id, rating)
        }
        ).collect::<Vec<String>>()
}

/// helper function to get a list of pending dms and the number of unread messages
pub(crate) fn get_list_of_pending_dms(peer_info_hashmap: HashMap<PeerId, PeerInfo>) -> Vec<String> {
    peer_info_hashmap
        .iter()
        .filter(|(_, peer_info)| {
            peer_info.username.is_some() && peer_info.dm_open
        })
        .map(|(other_peer_id, peer_info)| {
            let username = peer_info.username.clone().unwrap();

            // collect just the other peer's messages
            let unread_other_peer_dms = peer_info.dms
                .clone()
                .iter()
                .filter(|msg| {
                    let from_other_peer = msg.peer_id == *other_peer_id;
                    let has_not_been_read = match peer_info.last_read {
                        Some(last_read) => { msg.timestamp >= last_read }
                        None => true
                    };

                    from_other_peer && has_not_been_read
                })
                .cloned()
                .collect::<Vec<Message>>();

            // if there are none, then early return
            if unread_other_peer_dms.is_empty() {
                // return if we don't have any DMs
                return format!("📥 {username}");
            }

            // find the last chat message of the unread chat messages
            let mut last_chat_message: String = String::new();
            if let MessageType::ChatMessage(message) =
                unread_other_peer_dms.last().unwrap().clone().message_type
            {
                last_chat_message = message.chars().take(16).collect();
            }

            // find when the other peer last messaged
            let last_messaged = format_local_time(peer_info.dms.last().unwrap().timestamp);

            // format the final display
            format!("📥 {username}, last messaged '{last_chat_message}...' at {last_messaged} ({})",
                    unread_other_peer_dms.len())
        }
        ).collect::<Vec<String>>()
}