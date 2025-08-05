use chrono::{DateTime, Local};
use libp2p::PeerId;

/// enum for each type of message we handle in the chat (others sit in system messages)
#[derive(Clone, Debug)]
pub(crate) enum MessageType {
    UserJoin,
    UserLeave,
    ChatMessage(String),
    MyOwnMessage(String),

    FileRequestedIncoming(String),
    FileReceivedIncoming(String),
    FileRequestedOutgoing(String),
    FileReceivedOutgoing(String)
}

/// Store generic message data, important that we have peer_id to crossref username
#[derive(Debug, Clone)]
pub(crate) struct Message {
    pub message_type: MessageType,
    pub peer_id: PeerId,
    pub timestamp: DateTime<Local>,
}

/// Helper function for formatting a timestamp
pub(crate) fn format_local_time(time: DateTime<Local>) -> String {
    time.format("%H:%M").to_string()
}

/// Implement formatting for message
impl Message {
    pub(crate) fn format(&self, username: Option<String>) -> String {
        let formatted_time = format_local_time(self.timestamp);

        // Generate sender string, either username or peer_id if we don't have their username
        let sender = match username {
            Some(username) => username,
            None => {
                self.peer_id.to_string()
            }
        };

        // Handle each MessageType formatting
        match self.message_type.clone() {
            MessageType::UserJoin => format!("{formatted_time}: {sender} has joined the topic"),
            MessageType::UserLeave => format!("{formatted_time}: {sender} has left the topic"),
            MessageType::ChatMessage(message) => format!("{formatted_time} {sender}: {message}"),
            MessageType::MyOwnMessage(message) => format!("{formatted_time} You: {message}"),
            MessageType::FileRequestedIncoming(filename) => format!("You have received a request for {sender} from {filename}"),
            MessageType::FileReceivedIncoming(filename) => format!("You have received file {filename} from {sender}"),
            MessageType::FileRequestedOutgoing(filename) => format!("You have requested {filename} from {sender}"),
            MessageType::FileReceivedOutgoing(filename) => format!("You have sent file {filename} to {sender}")
        }
    }
}

/// system message - different to message as it doesn't it's all local
/// and doesn't care about where it came from
#[derive(Clone)]
pub(crate) struct SystemMessage {
    message: String,
    timestamp: DateTime<Local>
}

/// implement formatting for system messages
impl SystemMessage {
    pub(crate) fn new(message: String) -> Self {
        SystemMessage {
            message,
            timestamp: Local::now()
        }
    }

    pub(crate) fn format(&self) -> String {
        let formatted_time = format_local_time(self.timestamp);
        format!("{formatted_time}: {}", self.message)
    }
}
