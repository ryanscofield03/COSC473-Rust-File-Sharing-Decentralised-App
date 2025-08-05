mod ui;
mod network;
mod security;
mod file_management;
mod rating;
mod messages;
mod peer_info;

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use chrono::{Local};
use ratatui::{
    prelude::*,
    Terminal,
    backend::CrosstermBackend,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::{StreamExt};
use futures::stream::BoxStream;
use libp2p::gossipsub::IdentTopic;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use tokio::{spawn, time};
use tokio::time::Instant;
use crate::file_management::create_directories;
use crate::messages::{Message, MessageType, SystemMessage};
use crate::network::{Client};
use crate::rating::{Rating, RatingType};
use crate::ui::{render_found_users, render_input_box, render_pending_dms, render_system_messages,
                ChatroomScreen, DirectMessageScreen, EnterNameScreen, ScreenRenderer, SelectChatroomScreen};
use crate::peer_info::{get_direct_messages, get_list_of_peer_data,
                       get_list_of_pending_dms, get_username, PeerInfo};

// various commands that we want to match to inputs
const EXIT_COMMAND: &str = "/exit";
const NEW_TOPIC_PREFIX: &str = "/new ";
const REQUEST_PREFIX: &str = "/req ";
const RESPONSE_PREFIX: &str = "/res ";
const DM_PREFIX: &str = "/dm ";

const INFILTRATION_PREFIX: &str = "/inf ";

/// Stores an TimestampedPubSubList object, which is retrieved from the network kad DHT
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct TimestampedPubSubList {
    timestamp: u64,
    topics: Vec<String>
}

/// Holds all app data - could be broken into components but makes things quite complex
struct App {
    debug_mode: bool,
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    state: AppState,
    peer_id: PeerId,
    input: String,
    username: String,
    username_saved: bool,
    peer_ratings_hashmap: HashMap<PeerId, Vec<Rating>>,
    peer_info_hashmap: HashMap<PeerId, PeerInfo>,
    pubsub_messages_hashmap: HashMap<String, Vec<Message>>,
    timestamped_pubsubs: TimestampedPubSubList,
    system_messages: Vec<SystemMessage>,
    network_client: Client,
    network_events: BoxStream<'static, network::Event>
}

/// The current screen that the app is on with possible relevant metadata
#[derive(Clone)]
enum AppState {
    EnterName,
    SelectChatroom,
    Chatroom(IdentTopic),
    DirectMessage(PeerId),
}

/// Generic trait for navigation between screens
trait Navigate {
    /// generic for going to the next screen in our "state machine"
    fn next(&mut self, topic: Option<IdentTopic>, other_peer_id: Option<PeerId>);

    /// generic for exiting the current screen
    fn exit(&mut self);
}

impl Navigate for AppState {
    /// match based off of current screen and go to next screen if allowed
    fn next(&mut self, topic: Option<IdentTopic>, other_peer_id: Option<PeerId>) {
        *self = match self {
            AppState::EnterName => AppState::SelectChatroom,
            AppState::SelectChatroom => AppState::Chatroom(topic.expect("Expected a topic")),
            AppState::Chatroom(_) => AppState::DirectMessage(other_peer_id.expect("Expected a peer id")),
            AppState::DirectMessage(_) => return,
        };
    }

    /// match based off of current screen and go to exit screen if allowed (this isn't always the previous screen)
    fn exit(&mut self) {
        *self = match self {
            AppState::EnterName => return,
            AppState::SelectChatroom => return,
            AppState::Chatroom(_) => AppState::SelectChatroom,
            AppState::DirectMessage(_) => AppState::SelectChatroom,
        };
    }
}

/// Implements basically all app related methods
impl App {

    /// Instantiate the network, the network event loop, network listeners, and our ui
    async fn new(debug_mode: bool) -> Result<Self, Box<dyn Error>> {
        // Set up network (swarm, peer_id, keypair, sender/receiver channels for communicating
        // between app and network layers)
        let (
            peer_id,
            network_client,
            network_events,
            network_event_loop) = network::new().await?;

        // Start our network event loop, for handling commands from the app, and events from the network
        spawn(network_event_loop.run());

        // create directories for files, using peer_id
        create_directories(peer_id).await;

        // Setup UI
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(App {
            debug_mode,
            terminal,
            state: AppState::EnterName,
            input: String::new(),
            peer_id,
            username: String::new(),
            username_saved: false,
            system_messages: Vec::new(),
            peer_info_hashmap: HashMap::new(),
            peer_ratings_hashmap: HashMap::new(),
            timestamped_pubsubs: TimestampedPubSubList { timestamp: 0, topics: vec!["Chat".to_string()] },
            pubsub_messages_hashmap: HashMap::new(),
            network_client,
            network_events: Box::pin(network_events),
        })
    }

    /// Frame generation and event handling (network and user inputs)
    async fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let mut context = Context::from_waker(futures::task::noop_waker_ref());

        // Spawn an async function that continuously loops and requests the most recent pubsub topics
        let mut client = self.network_client.clone();
        spawn(async move {
            let mut interval = time::interval(Duration::from_secs(20));
            loop {
                interval.tick().await;
                client.request_pubsubs().await;
            }
        });

        // Render the screen, handle user inputs, and network events
        let render_interval = Duration::from_millis(1000/60); // 60 fps
        let store_info_interval = Duration::from_secs(5);     // every 5 seconds
        let update_info_interval = Duration::from_secs(2);    // every 2 seconds

        let mut last_render_time = Instant::now();
        let mut last_store_time = Instant::now();
        let mut last_update_time = Instant::now();
        loop {
            let now = Instant::now();
            if now.duration_since(last_render_time) >= render_interval {
                self.render()?;
                last_render_time = now;
            }
            if now.duration_since(last_store_time) >= store_info_interval {
                self.store_user_info();
                last_store_time = now;
            }
            if now.duration_since(last_update_time) >= update_info_interval {
                self.update_all_user_info();
                last_update_time = now;
            }

            // Updates the last read for a dm if we are in said dm
            if let AppState::DirectMessage(other_peer_id) = self.state {
                self.update_last_read_for_dm(other_peer_id)
            }

            // Poll for 100ms for user inputs
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(event) = event::read()? {
                    if event.kind == KeyEventKind::Press {
                        match event.code {
                            // Let the user exit with graceful handling with ctrl + q
                            quit if quit == KeyCode::Char('q')
                                && event.modifiers.contains(KeyModifiers::CONTROL) => break,

                            // Handle pressing enter on current screen
                            KeyCode::Enter => {
                                match self.input.clone() {
                                    _ if self.input.clone() == EXIT_COMMAND => {
                                        // Unsubscribe from pubsub if the user is exiting chatroom
                                        if let AppState::Chatroom(topic) = self.state.clone() {
                                            self.unsubscribe_from_pubsub(topic);
                                        };

                                        self.state.exit();
                                        self.input.clear();
                                    }
                                    _ => {
                                        match self.state.clone() {
                                            AppState::EnterName =>
                                                self.handle_enter_name_submit(),
                                            AppState::SelectChatroom =>
                                                self.handle_select_chatroom_submit(),
                                            AppState::Chatroom(topic) =>
                                                self.handle_chatroom_submit(topic),
                                            AppState::DirectMessage(other_peer_id) =>
                                                self.handle_direct_message_submit(other_peer_id),
                                        }
                                    }
                                }
                            }

                            // if they type any character, push it to the input
                            KeyCode::Char(c) => {
                                // don't allow an input to start with spaces :)
                                if c == ' ' && self.input.len() == 0 {
                                    continue;
                                };

                                self.input.push(c);
                            },
                            // if they press backspace, remove it from the input
                            KeyCode::Backspace => {
                                self.input.pop();
                            },
                            _ => {}
                        }
                    }
                }
            }

            // handle network events, these are queued by our network when our swarm handles an event
            // that the app needs to know about (e.g. received a message)
            if let Poll::Ready(Some(network_event)) = self.network_events.as_mut().poll_next_unpin(&mut context) {
                match network_event {
                    // received a generic system message (basically println if we enable debug mode)
                    network::Event::SystemMessage(message) => {
                        self.system_messages.push(SystemMessage::new(message));
                    }
                    // handle when we have a new listener
                    network::Event::NewListener(peer_id) => {
                        if !self.peer_info_hashmap.contains_key(&peer_id) {
                            self.peer_info_hashmap.insert(peer_id, PeerInfo::default());
                        }
                    }
                    // handle receiving a username
                    network::Event::ReceivedUsername(peer_id, username) => {
                        // update peer's info to hold their username
                        self.peer_info_hashmap
                            .entry(peer_id)
                            .and_modify(|info| info.username = Some(username.clone()))
                            .or_insert_with(|| {
                                let mut info = PeerInfo::default();
                                info.username = Some(username.clone());
                                info
                            });
                    },
                    // Handle peer subscribing to a topic we are in, assumes we can't be in many topics
                    network::Event::PeerSubscribed(peer_id, topic_hash) => {
                        // If we are in a chatroom, then inject a chat message saying who joined
                        if let AppState::Chatroom(topic) = self.state.clone() {
                            if topic.to_string() == topic_hash.as_str() {
                                let message = Message {
                                    message_type: MessageType::UserJoin,
                                    peer_id,
                                    timestamp: Local::now()
                                };
                                self.add_message_to_current_pubsub(message);
                            }
                        }
                    }
                    // Handle peer unsubscribing from a topic we are in
                    network::Event::PeerUnsubscribed(peer_id, topic_hash) => {
                        // if we are in a chatroom, then inject a chat message saying who left
                        if let AppState::Chatroom(topic) = self.state.clone() {
                            if topic.to_string() == topic_hash.as_str() {
                                let message = Message {
                                    message_type: MessageType::UserLeave,
                                    peer_id,
                                    timestamp: Local::now()
                                };
                                self.add_message_to_current_pubsub(message);
                            }
                        }
                    }
                    // Handle receiving pubsub topics from DHT
                    network::Event::ReceivedPubsubTopics(timestamped_topic_list) => {
                        // Only take it if it is the more recent that what we currently have
                        // This can easily be gamed, but it is assumed that peers are not malicious :)
                        if timestamped_topic_list.timestamp > self.timestamped_pubsubs.timestamp {
                            self.timestamped_pubsubs = timestamped_topic_list;
                        }
                    }
                    // Handle receiving a pubsub message
                    network::Event::ReceivedPubsubMessage(peer_id, message) => {
                        // Create a chat message
                        let message = Message {
                            message_type: MessageType::ChatMessage(message),
                            peer_id,
                            timestamp: Local::now()
                        };
                        // Store is locally in our local hashmap under the topic of our current pubsub
                        self.add_message_to_current_pubsub(message);
                    }
                    // Handle receiving a notification that a peer DMd you
                    network::Event::ReceivedDmNotification(other_peer_id) => {
                        let dm_not_initiated = match self.peer_info_hashmap.get(&other_peer_id) {
                            Some(info) => !info.dm_open,
                            None => true
                        };

                        if dm_not_initiated {
                            self.initiate_direct_message(other_peer_id);
                        }
                    }
                    // Handle receiving a direct message
                    network::Event::ReceivedDirectMessage(other_peer_id, message) => {
                        // Create a chat message
                        let message = Message {
                            message_type: MessageType::ChatMessage(message),
                            peer_id: other_peer_id,
                            timestamp: Local::now(),
                        };

                        // Store that chat message to our local hashmap of peer_id, messages
                        self.add_message_to_current_dm(other_peer_id, message);
                    }
                    // Handle a file request from another user
                    network::Event::FileRequested(other_peer_id, filename) => {
                        // Write an incoming file request message, this is necessary since our
                        // app allows the user to confirm that they want to send the file to
                        // the other user.
                        let message = Message {
                            message_type: MessageType::FileRequestedIncoming(filename),
                            peer_id: other_peer_id,
                            timestamp: Local::now(),
                        };

                        // Inject this message into our chat
                        self.add_message_to_current_dm(other_peer_id, message);
                    }
                    // Handle receiving a file (UI side of things)
                    network::Event::FileReceived(other_peer_id, filename) => {
                        // Write an incoming file received message
                        let message = Message {
                            message_type: MessageType::FileReceivedIncoming(filename),
                            peer_id: other_peer_id,
                            timestamp: Local::now(),
                        };

                        // Inject this message into our chat
                        self.add_message_to_current_dm(other_peer_id, message);
                    }

                    // handle an incoming rating
                    network::Event::ReceivedPeerRating(rating ) => {
                        // Check if the rated_peer_id has ratings stored
                        if let Some(ratings) =
                            self.peer_ratings_hashmap.get_mut(&rating.rated_peer_id)
                        {
                            let mut rating_found = false;
                            for current in ratings.iter_mut() {
                                if current.rater_peer_id == rating.rater_peer_id {
                                    // If the rater has rated this user before
                                    // and the current rating is older, update it
                                    if current.timestamp < rating.timestamp {
                                        *current = rating.clone();
                                    }
                                    rating_found = true;
                                    break;
                                }
                            }

                            // If the rater has not rated this user yet, we add the new rating
                            if !rating_found {
                                ratings.push(rating.clone());
                            }
                        } else {
                            // If the rated_peer_id doesn't exist in the hashmap,
                            // create a new entry for it with the new rating
                            self.peer_ratings_hashmap.insert(
                                rating.rated_peer_id,
                                vec![rating.clone()],
                            );
                        }

                        self.update_peer_rating(
                            rating.rated_peer_id,
                            self.peer_ratings_hashmap.get(&rating.rated_peer_id).unwrap().clone()
                        )
                    }

                    // If we fail to save our username in the DHT, try again!
                    network::Event::UsernameFailedToSave => {
                        self.username_saved = false;
                    }
                }
            }
        }

        Ok(())
    }

    /// helper for getting the peer id by a user input of username of at least the
    /// first 8 characters of the peer id
    fn get_peer_id_by_user_input(&self, user_input: &str) -> Option<PeerId> {
        self.peer_info_hashmap
            .iter()
            .find(|(other_peer_id, peer_info)| {
                peer_info.username.as_ref().map(|username| *username == user_input).unwrap_or(false)
                    || (other_peer_id.to_string().starts_with(&user_input) && user_input.len() >= 8)
            })
            .map(|(other_peer_id, _)| *other_peer_id)
    }

    /// helper function to open a dm with a peer
    fn open_dm_with_peer(&mut self, other_peer_id: PeerId) {
        self.peer_info_hashmap
            .entry(other_peer_id)
            .and_modify(|peer_info| {
                peer_info.dm_open = true;
            })
            .or_insert_with(|| {
                let mut info = PeerInfo::default();
                info.dm_open = true;
                info
            });
    }

    /// helper function to update the last read of a dm with peer
    fn update_last_read_for_dm(&mut self, other_peer_id: PeerId) {
        self.peer_info_hashmap
            .entry(other_peer_id)
            .and_modify(|peer_info| {
                peer_info.last_read = Some(Local::now());
            })
            .or_insert_with(|| {
                let mut info = PeerInfo::default();
                info.last_read = Some(Local::now());
                info
            });

        self.system_messages.push(
            SystemMessage::new(format!("updated last read for {}", other_peer_id))
        );
    }

    /// adds some message to the hashmap vec value for a given peer_id
    fn add_message_to_current_dm(
        &mut self,
        other_peer_id: PeerId,
        message: Message,
    ) {
        // insert to old vec ot create a new vec with new message
        self.peer_info_hashmap
            .entry(other_peer_id)
            .and_modify(|peer_info| {
                peer_info.dms.push(message.clone());
            })
            .or_insert_with(|| {
                let mut info = PeerInfo::default();
                info.dms.push(message);
                info
            });
    }

    fn update_peer_rating(
        &mut self,
        other_peer_id: PeerId,
        ratings: Vec<Rating>
    ) {
        let new_rating: i16 = ratings.iter().map(|rating| match rating.rating {
            RatingType::Positive => 1,
            RatingType::Negative => -1,
        }).sum();

        self.peer_info_hashmap
            .entry(other_peer_id)
            .and_modify(|peer_info| {
                peer_info.rating = new_rating
            })
            .or_insert_with(|| {
                let mut info = PeerInfo::default();
                info.rating = new_rating;
                info
            });
    }

    /// spawn an async task for storing our public info (if not yet saved)
    /// this makes a request to our client, but must await a response
    fn store_user_info(&mut self) {
        if !self.username_saved && !self.username.trim().is_empty() {
            self.username_saved = true;
            let mut network_client = self.network_client.clone();
            let username = self.username.clone();
            spawn(async move {
                network_client.store_username(username).await;
                network_client.store_public_key().await;
            });
        }
    }

    /// requests all the unknown information from the peer info hashmap if the information
    /// hasn't been updated within the last 20 seconds
    fn update_all_user_info(&mut self) {
        let now = Local::now();
        let peer_ids: Vec<PeerId> = self.peer_info_hashmap
            .iter()
            .filter(|(_, info)| {
                now.signed_duration_since(info.last_updated).num_seconds() >= 20
            })
            .map(|(peer_id, _)| peer_id.clone())
            .collect();

        for peer_id in peer_ids {
            self.request_username(peer_id.clone());
            self.request_peer_rating(peer_id.clone());

            if let Some(info) = self.peer_info_hashmap.get_mut(&peer_id) {
                info.last_updated = now;
            }
        }
    }

    /// spawn an async task for requesting a username of a given peer_id
    /// this makes a request to our client, but must await a response
    fn request_username(&mut self, peer_id: PeerId) {
        if !self.peer_info_hashmap
            .get(&peer_id)
            .and_then(|info| info.username.clone())
            .is_some()
        {
            let mut network_client = self.network_client.clone();
            spawn(async move {
                network_client.request_username(peer_id).await;
            });
        }
    }

    /// spawn an async task for subscribing to a pubsub of a given topic (pubsub)
    /// this makes a request to our client, but must await a response
    fn subscribe_to_pubsub(&mut self, topic: IdentTopic) {
        let mut network_client = self.network_client.clone();
        spawn(async move {
            network_client.subscribe_to_pubsub(topic).await;
        });
    }

    /// spawn an async task for unsubscribing from a given topic (pubsub)
    /// this makes a request to our client, but must await a response
    fn unsubscribe_from_pubsub(&mut self, topic: IdentTopic) {
        let mut network_client = self.network_client.clone();
        spawn(async move {
            network_client.unsubscribe_from_pubsub(topic).await;
        });
    }

    /// spawn an async task for storing a new pubsub in the kad DHT
    /// this makes a request to our client, but must await a response
    fn store_pubsub(&mut self, topic: IdentTopic) {
        let mut network_client = self.network_client.clone();
        let timestamped_pubsubs = self.timestamped_pubsubs.clone();
        spawn(async move {
            network_client.store_pubsub(timestamped_pubsubs.topics, topic).await;
        });
    }

    /// spawn an async task for sending a message to a given topic (pubsub)
    /// this makes a request to our client, but must await a response
    fn send_message_to_pubsub(&mut self, topic: IdentTopic) {
        let mut network_client = self.network_client.clone();
        let input = self.input.clone();
        spawn(async move {
            network_client.send_message_to_pubsub(topic, input).await;
        });
    }

    /// spawns an async task to set up direct messages (e.g. joining the same, "private" pubsub topic)
    /// this makes a request to our client, but must await a response
    fn initiate_direct_message(&mut self, other_peer_id: PeerId) {
        self.open_dm_with_peer(other_peer_id);

        let peer_id = self.peer_id;
        let mut network_client = self.network_client.clone();
        spawn(async move {
            network_client.initiate_direct_message(peer_id, other_peer_id).await;
        });
    }

    /// spawns an async task to infiltrate a direct message (debug only)
    /// this makes a request to our client, but must await a response
    fn infiltrate_direct_message(&mut self, other_peer_id1: PeerId, other_peer_id2: PeerId) {
        let mut network_client = self.network_client.clone();
        spawn(async move {
            network_client.infiltrate_direct_message(other_peer_id1, other_peer_id2).await;
        });
    }

    /// spawns an async task send a direct message to a given peer
    /// this makes a request to our client, but must await a response
    fn direct_message(&mut self, other_peer_id: PeerId, message: String) {
        let mut network_client = self.network_client.clone();
        spawn(async move {
            network_client.direct_message(other_peer_id, message).await;
        });
    }

    /// spawns an async task to request file from user
    /// this makes a request to our client, but must await a response
    fn request_file(&mut self, other_peer_id: PeerId, req: String) {
        let mut network_client = self.network_client.clone();
        spawn(async move {
            network_client.request_file(other_peer_id, req).await;
        });
    }

    /// spawns an async task to accept the sending of file to the user that requested it
    /// the user must explicitly type the file they are sending
    /// this makes a request to our client, but must await a response
    fn respond_file(&mut self, res: String, other_peer_id: PeerId) {
        let mut network_client = self.network_client.clone();
        spawn(async move {
            network_client.respond_file(res, other_peer_id).await;
        });
    }

    /// spawns an async task to give a peer a rating
    /// this makes a request to our client and must wait for a response
    fn give_peer_rating(
        &mut self,
        other_peer_id: PeerId,
        rating: RatingType
    ) {
        let mut network_client = self.network_client.clone();
        let rating = Rating {
            rating,
            rater_peer_id: self.peer_id,
            rated_peer_id: other_peer_id,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        };

        spawn(async move {
            network_client.give_peer_rating(rating).await;
        });
    }

    /// spawns a long-running async task to request a peer's rating every 10 seconds
    /// this makes a request to our client and must wait for a response
    fn request_peer_rating(
        &mut self,
        other_peer_id: PeerId,
    ) {
        let mut network_client = self.network_client.clone();
        spawn(async move {
            let mut interval = time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                network_client.request_peer_rating(other_peer_id).await;
            }
        });
    }

    /// adds some message to the hashmap vec value for the current pubsub
    fn add_message_to_current_pubsub(
        &mut self,
        message: Message,
    ) {
        // if we have a topic
        if let AppState::Chatroom(topic) = &self.state {
            // insert to old vec ot create a new vec with new message
            self.pubsub_messages_hashmap
                .entry(topic.to_string())
                .or_insert_with(Vec::new)
                .push(message);
        }
    }

    /// handle submitting enter username
    fn handle_enter_name_submit(&mut self) {
        if !self.input.is_empty() {
            self.username = self.input.clone();

            // move to select chatroom screen
            self.state.next(None, None);
            self.input.clear();
        }
    }

    /// handle selecting a chatroom submission
    fn handle_select_chatroom_submit(&mut self) {
        let input = self.input.trim().to_string();

        match input.as_str() {
            // if it starts with the new chat prefix
            new_chat if new_chat.to_lowercase().starts_with(NEW_TOPIC_PREFIX) => {
                let new_topic= input[NEW_TOPIC_PREFIX.len()..].trim();
                // then try to make a new chat if it's not empty
                if !new_topic.trim().is_empty() {
                    let topic = IdentTopic::new(new_topic);
                    self.store_pubsub(topic);
                    self.input.clear();
                }
            },
            // if it matches some topic that already exists
            topic if !topic.is_empty()
                && self.timestamped_pubsubs.topics.contains(&topic.to_string()) =>
                {
                    // then join that topic
                    let topic = IdentTopic::new(input);
                    self.state.next(Some(topic.clone()), None);
                    self.subscribe_to_pubsub(topic);
                    self.input.clear();
                },
            _ => {}
        }
    }

    fn handle_chatroom_submit(&mut self, topic: IdentTopic) {
        match self.input.to_lowercase() {
            // handle dms
            dm if dm.starts_with(DM_PREFIX) => {
                // get their other peer id depending on if they typed the username or peed_id
                let other_user = self.input[DM_PREFIX.len()..].trim();
                let other_peer_id = self.get_peer_id_by_user_input(other_user);

                // if we found a matching peer_id in their local hashmap, then initiate a DM
                if let Some(other_peer_id) = other_peer_id {
                    self.unsubscribe_from_pubsub(topic);
                    self.initiate_direct_message(other_peer_id);
                    self.state.next(None, Some(other_peer_id));
                    self.input.clear();
                }
            }

            // handle dm infiltration (for testing encryption, enabled in debug mode)
            infiltration if infiltration.starts_with(INFILTRATION_PREFIX) && self.debug_mode => {
                let mut usernames_split = self.input[DM_PREFIX.len()..].trim().split_whitespace();
                let other_peer_id1 = self.get_peer_id_by_user_input(usernames_split.next().unwrap());
                let other_peer_id2 = self.get_peer_id_by_user_input(usernames_split.next().unwrap());

                if let (Some(other_peer_id1), Some(other_peer_id2)) = (other_peer_id1, other_peer_id2) {
                    self.unsubscribe_from_pubsub(topic);
                    self.infiltrate_direct_message(other_peer_id1, other_peer_id2);
                    self.state.next(None, Some(self.peer_id));
                    self.input.clear();
                }
            }

            // handle a regular chat message
            _ => {
                if !self.input.is_empty() {
                    self.send_message_to_pubsub(topic.clone());

                    let message = Message {
                        message_type: MessageType::MyOwnMessage(self.input.clone()),
                        peer_id: self.peer_id,
                        timestamp: Local::now()
                    };
                    self.add_message_to_current_pubsub(message);
                    self.input.clear();
                }
            }
        }
    }

    fn handle_direct_message_submit(&mut self, other_peer_id: PeerId) {
        match self.input.trim() {
            req if req.to_lowercase().starts_with(REQUEST_PREFIX) => {
                let filename = self.input[REQUEST_PREFIX.len()..].trim().to_string();
                self.request_file(other_peer_id, filename.clone());

                let message = Message {
                    message_type: MessageType::FileRequestedOutgoing(filename),
                    peer_id: other_peer_id,
                    timestamp: Local::now(),
                };
                self.add_message_to_current_dm(other_peer_id, message);
                self.input.clear();
            },
            res if res.to_lowercase().starts_with(RESPONSE_PREFIX) => {
                let filename = self.input[RESPONSE_PREFIX.len()..].trim().to_string();
                self.respond_file(filename.clone(), other_peer_id);

                let message = Message {
                    message_type: MessageType::FileReceivedOutgoing(filename),
                    peer_id: other_peer_id,
                    timestamp: Local::now(),
                };
                self.add_message_to_current_dm(other_peer_id, message);
                self.input.clear();
            },
            add_rating if add_rating == "+1" => {
                self.give_peer_rating(other_peer_id, RatingType::Positive);
                self.input.clear();
            },
            remove_rating if remove_rating == "-1" => {
                self.give_peer_rating(other_peer_id, RatingType::Negative);
                self.input.clear();
            },
            _ => {
                self.direct_message(
                    other_peer_id,
                    self.input.clone(),
                );

                let message = Message {
                    message_type: MessageType::MyOwnMessage(self.input.clone()),
                    peer_id: other_peer_id,
                    timestamp: Local::now(),
                };
                self.add_message_to_current_dm(other_peer_id, message);
                self.input.clear();
            }
        }
    }

    /// Renders the screen
    fn render(&mut self) -> Result<(), Box<dyn Error>> {
        self.terminal.draw(|frame| {
            // generate our constraints, which are the sizes of our blocks of the screen
            let constraints = if self.debug_mode {
                // if we are in debug mode, allocate some room for the system messages
                vec![Constraint::Percentage(80), Constraint::Percentage(20), Constraint::Percentage(20)]
            } else {
                vec![Constraint::Percentage(80), Constraint::Percentage(20)]
            };

            // blocks which makes the screen up
            let blocks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints)
                .split(frame.area());

            let mut messages: Vec<String> = vec![];

            let main_screen_blocks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Min(20), Constraint::Length(3)])
                .split(blocks[0]);
            // match the app state to some screen rendering
            let screen: Box<dyn ScreenRenderer> = match self.state.clone() {
                AppState::EnterName => {
                    Box::new(EnterNameScreen)
                },
                AppState::SelectChatroom => {
                    Box::new(SelectChatroomScreen {
                        topics: self.timestamped_pubsubs.clone().topics
                    })
                },
                AppState::Chatroom(topic) => {
                    // updates our empty messages vec to store each of the messages for this topic
                    // maybe we can avoid this as this is done every frame...
                    if let AppState::Chatroom(topic) = self.state.clone() {
                        if let Some(pubsub_messages) =
                            self.pubsub_messages_hashmap.get(&topic.to_string())
                        {
                            pubsub_messages.clone().iter().for_each(|message| {
                                let username_option = get_username(
                                    self.peer_info_hashmap.clone(), message.peer_id);
                                messages.push(message.format(username_option.map(|s| s.clone())));
                            });
                        }
                    }
                    Box::new(ChatroomScreen { topic: topic.to_string(), messages })
                },
                AppState::DirectMessage(other_peer_id) => {
                    // Find peer's name or replace with peer_id
                    let other_user_name = match get_username(
                        self.peer_info_hashmap.clone(), other_peer_id)
                    {
                        Some(other_user_name) => other_user_name.to_string(),
                        None => other_peer_id.to_string()
                    };

                    // updates our empty messages vec to store each of the messages for this dm
                    // again, maybe we can avoid this as this is done every frame...
                    get_direct_messages(self.peer_info_hashmap.clone(), other_peer_id)
                        .clone().iter().for_each(|message| {
                        messages.push(message.format(Some(other_user_name.clone())));
                    });
                    Box::new(DirectMessageScreen { other_user_name, messages })
                },
            };
            // actually render this main screen
            screen.render(frame, main_screen_blocks[0]);

            // renders the input box
            render_input_box(self.input.clone(), frame, main_screen_blocks[1]);

            let user_info_blocks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Percentage(70), Constraint::Percentage(30)])
                .split(blocks[1]);

            // render the user's found box
            render_found_users(get_list_of_peer_data(self.peer_info_hashmap.clone()),
                               frame,
                               user_info_blocks[0]
            );

            // renders the user's pending dm's box
            render_pending_dms(get_list_of_pending_dms(self.peer_info_hashmap.clone()),
                               frame,
                               user_info_blocks[1]
            );

            // if we are in debug mode, then render the system messages
            if self.debug_mode {
                let formatted_messages = self.system_messages.clone()
                    .iter()
                    .map(|msg| msg.format())
                    .collect();
                render_system_messages(formatted_messages, frame, blocks[2]);
            }
        })?;

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let debug_mode = args.iter().any(|arg| arg == "--debug");

    // start & run app
    let mut app = App::new(debug_mode).await?;
    app.run().await?;

    // cleanup after user gracefully leaves app - can have issues if we panic
    disable_raw_mode()?;
    execute!(app.terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(())
}