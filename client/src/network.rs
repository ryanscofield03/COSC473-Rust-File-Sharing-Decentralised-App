use std::{error::Error, time::Duration};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use futures::{channel::{mpsc, oneshot}, prelude::*, StreamExt, };
use libp2p::{core::Multiaddr, gossipsub, kad, mdns, noise, ping, rendezvous, request_response, swarm::{NetworkBehaviour, Swarm, SwarmEvent}, tcp, yamux, PeerId, StreamProtocol};
use libp2p::gossipsub::{IdentTopic, TopicHash};
use libp2p::identity::{Keypair};
use libp2p::kad::{Mode, RecordKey};
use libp2p::kad::store::MemoryStore;
use libp2p::multiaddr::Protocol;
use libp2p::request_response::{Message, OutboundRequestId, ProtocolSupport, ResponseChannel};
use serde::{Deserialize, Serialize};
use tokio::time::Interval;
use crate::{Rating, TimestampedPubSubList};
use crate::security::{decrypt_bytes, decrypt_message, encrypt_bytes, encrypt_message};
use crate::file_management::{retrieve_data_from_file, write_data_to_file};
use crate::network::Event::SystemMessage;

const RENDEZVOUS_NAMESPACE: &str = "rendezvous";
const PUBLIC_KEY_PREFIX: &str = "public_key_";
const PUBSUB_TOPICS_KEY: &str = "pubsub_topics";
const USERNAME_KEY_PREFIX: &str = "name_key_";
const RATING_KEY_PREFIX: &str = "rating_";

/// stores the behaviour of our network
#[derive(NetworkBehaviour)]
struct ClientBehaviour {
    mdns: mdns::tokio::Behaviour,
    gossipsub: gossipsub::Behaviour,
    kademlia: kad::Behaviour<MemoryStore>,
    request_response: request_response::cbor::Behaviour<FileRequest, FileResponse>,
    dm_notification: request_response::cbor::Behaviour<JoinDmRequest, JoinDmResponse>,
    rendezvous: rendezvous::client::Behaviour,
    ping: ping::Behaviour
}

/// stores a file request object (filename)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRequest(String);

/// stores a file response object (file as bytes)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileResponse(Vec<u8>);

/// used to notify another peer that we are trying to DM them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinDmRequest {}

/// can be used to notify dm'er that their notification was acked
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinDmResponse {}

/// represents an event that we send to the App for it to handle
pub(crate) enum Event {
    PeerSubscribed(PeerId, TopicHash),
    PeerUnsubscribed(PeerId, TopicHash),
    ReceivedPubsubMessage(PeerId, String),
    ReceivedPubsubTopics(TimestampedPubSubList),
    UsernameFailedToSave,
    ReceivedUsername(PeerId, String),
    NewListener(PeerId),
    ReceivedDmNotification(PeerId),
    ReceivedDirectMessage(PeerId, String),
    FileRequested(PeerId, String),
    FileReceived(PeerId, String),
    ReceivedPeerRating(Rating),
    SystemMessage(String)
}

/// represents a command that the app sends to us to handle
#[derive(Debug)]
enum Command {
    StorePublicKey {
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    RequestPublicKey {
        other_peer_id: PeerId,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    SendDmNotification {
        other_peer_id: PeerId,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    SubscribeToPubsub {
        topic: IdentTopic,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    UnsubscribeToPubsub {
        topic: IdentTopic,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    SendMessageToPubSub {
        topic: IdentTopic,
        message: String,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    RequestPubsubs {
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    StorePubsub {
        topics: Vec<String>,
        topic: IdentTopic,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    StoreUsername {
        username: String,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    RequestUsername {
        other_peer_id: PeerId,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    DirectMessage {
        other_peer_id: PeerId,
        message: String,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    RequestFile {
        other_peer_id: PeerId,
        filename: String,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    RespondFile {
        filename: String,
        other_peer_id: PeerId,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    GivePeerRating {
        rating: Rating,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    RequestPeerRating {
        other_peer_id: PeerId,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    Error {
        message: String,
    },
}

/// set up the network
pub(crate) async fn new() -> Result<(PeerId, Client, impl Stream<Item = Event>, EventLoop), Box<dyn Error>> {
    // create a keypair
    let keypair = Keypair::generate_ed25519();
    let public_key = keypair.public();
    let peer_id = public_key.to_peer_id();

    // create keypair for encryption/decryption
    let encryption_secret = x25519_dalek::StaticSecret::random_from_rng(rand::rngs::OsRng);
    let encryption_public = x25519_dalek::PublicKey::from(&encryption_secret);

    // build our swarm with this keypair, and instantiate our network behaviour
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_behaviour(|key| {
            Ok(ClientBehaviour {
                mdns: mdns::tokio::Behaviour::new(
                    mdns::Config::default(),
                    peer_id
                )?,
                gossipsub: gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub::Config::default()
                )?,
                kademlia: kad::Behaviour::new(
                    peer_id,
                    MemoryStore::new(peer_id),
                ),
                request_response: request_response::cbor::Behaviour::new(
                    [(StreamProtocol::new("/file-exchange/1"), ProtocolSupport::Full,)],
                    request_response::Config::default().with_request_timeout(Duration::from_secs(120))
                ),
                dm_notification: request_response::cbor::Behaviour::new(
                    [(StreamProtocol::new("/join-dm/1"), ProtocolSupport::Full)],
                    request_response::Config::default().with_request_timeout(Duration::from_secs(30))
                ),
                rendezvous: rendezvous::client::Behaviour::new(key.clone()),
                ping: ping::Behaviour::new(ping::Config::new().with_interval(Duration::from_secs(60))
                    .with_timeout(Duration::from_secs(10))),
            })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    let _ = swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse().unwrap());
    let _ = swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?);

    let rendezvous_point_address = "/ip4/127.0.0.1/tcp/62649"
        .parse::<Multiaddr>()
        .unwrap();
    let rendezvous_point: PeerId = "12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN"
        .parse()
        .unwrap();

    let external_address = "/ip4/127.0.0.1/tcp/62649"
        .parse::<Multiaddr>()
        .unwrap();
    swarm.add_external_address(external_address);

    swarm.dial(rendezvous_point_address.clone()).unwrap();

    // set ourselves to be a kad dht server
    swarm.behaviour_mut().kademlia.set_mode(Some(Mode::Server));

    // instantiate channels for communication from app -> network and network -> app
    let (command_sender, command_receiver) = mpsc::channel(0);
    let (event_sender, event_receiver) = mpsc::channel(0);

    Ok((
        peer_id,
        Client { sender: command_sender },
        event_receiver,
        EventLoop::new(
            peer_id,
            encryption_public,
            encryption_secret,
            swarm,
            rendezvous_point,
            command_receiver,
            event_sender
        ),
    ))
}

/// client which holds the sender which can send messages to the network
#[derive(Clone)]
pub(crate) struct Client {
    sender: mpsc::Sender<Command>,
}

/// implement different actions that the app wants to take
impl Client {
    /// Handles a command which may be dropped, if it is dropped, this
    /// will handle it gracefully (rather than panicking)
    async fn gracefully_handle_command(
        &mut self,
        function_name: String,
        command: Command,
        receiver: oneshot::Receiver<Result<(), Box<dyn Error + Send>>>,
    ) {
        if let Err(_) = self.sender.send(command).await {
            let _ = self.sender.send(Command::Error {
                message: format!("{}: Command receiver not to be dropped", function_name)
            }).await;
        }

        if let Err(_) = receiver.await {
            let _ = self.sender.send(Command::Error {
                message: format!("{}: Sender not to be dropped", function_name)
            }).await;
        };
    }

    /// store your (encryption) public key in the kademlia dht
    pub(crate) async fn store_public_key(
        &mut self,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::StorePublicKey { sender };
        self.gracefully_handle_command(
            "store_public_key".to_string(),
            command,
            receiver
        ).await;
    }

    /// subscribe to some pubsub topic - when we enter a topic
    pub(crate) async fn subscribe_to_pubsub(
        &mut self,
        topic: IdentTopic,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::SubscribeToPubsub { topic, sender};
        self.gracefully_handle_command(
            "subscribe_to_pubsub".to_string(),
            command,
            receiver
        ).await;
    }

    /// unsubscribe from some pubsub topic - when we leave a topic
    pub(crate) async fn unsubscribe_from_pubsub(
        &mut self,
        topic: IdentTopic,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::UnsubscribeToPubsub { topic, sender};
        self.gracefully_handle_command(
            "unsubscribe_from_pubsub".to_string(),
            command,
            receiver
        ).await;
    }

    /// send a message to some pubsub that we are (hopefully) already subscribed to
    pub(crate) async fn send_message_to_pubsub(
        &mut self,
        topic: IdentTopic,
        message: String,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::SendMessageToPubSub { topic, message, sender };
        self.gracefully_handle_command(
            "send_message_to_pubsub".to_string(),
            command,
            receiver
        ).await;
    }

    /// store a new pubsub topic in the kad dht
    pub(crate) async fn store_pubsub(
        &mut self,
        topics: Vec<String>,
        topic: IdentTopic,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::StorePubsub { topics, topic, sender };
        self.gracefully_handle_command(
            "store_pubsub".to_string(),
            command,
            receiver
        ).await;
    }

    /// request the current list(s) of pubsubs in the dht
    pub(crate) async fn request_pubsubs(
        &mut self,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::RequestPubsubs { sender };
        self.gracefully_handle_command(
            "request_pubsubs".to_string(),
            command,
            receiver
        ).await;
    }

    /// stores our username in the dht
    pub(crate) async fn store_username(
        &mut self,
        username: String,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::StoreUsername { username, sender };
        self.gracefully_handle_command(
            "store_username".to_string(),
            command,
            receiver
        ).await;
    }

    /// requests another username by PeerId from the dht
    pub(crate) async fn request_username(
        &mut self,
        other_peer_id: PeerId,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::RequestUsername { other_peer_id, sender };
        self.gracefully_handle_command(
            "request_username".to_string(),
            command,
            receiver
        ).await;
    }

    /// instantiated direct messaging
    pub(crate) async fn initiate_direct_message(
        &mut self,
        peer_id: PeerId,
        other_peer_id: PeerId,
    ) {
        // generate a unique but deterministic topic and subscribe to it
        let topic = generate_pubsub_topic_for_dm(
            peer_id,
            other_peer_id
        );
        self.subscribe_to_pubsub(topic).await;

        let (sender, receiver) = oneshot::channel();
        let command = Command::SendDmNotification { other_peer_id, sender };
        self.gracefully_handle_command(
            "send_dm_notification".to_string(),
            command,
            receiver
        ).await;

        let (sender, receiver) = oneshot::channel();
        let command = Command::RequestPublicKey { other_peer_id, sender };
        self.gracefully_handle_command(
            "request_public_key".to_string(),
            command,
            receiver
        ).await;
    }

    /// infiltrate a direct message
    pub(crate) async fn infiltrate_direct_message(
        &mut self,
        other_peer_id1: PeerId,
        other_peer_id2: PeerId,
    ) {
        // generate a unique but deterministic topic and subscribe to it
        let topic = generate_pubsub_topic_for_dm(
            other_peer_id1,
            other_peer_id2
        );
        self.subscribe_to_pubsub(topic).await;

        let (sender, receiver) = oneshot::channel();
        let command = Command::RequestPublicKey { other_peer_id: other_peer_id1, sender };
        self.gracefully_handle_command(
            "request_public_key1".to_string(),
            command,
            receiver
        ).await;

        let (sender, receiver) = oneshot::channel();
        let command = Command::RequestPublicKey { other_peer_id: other_peer_id2, sender };
        self.gracefully_handle_command(
            "request_public_key2".to_string(),
            command,
            receiver
        ).await;
    }

    /// send a direct message to the current DM you are in
    pub(crate) async fn direct_message(
        &mut self,
        other_peer_id: PeerId,
        message: String,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::DirectMessage { other_peer_id, message, sender };
        self.gracefully_handle_command(
            "direct_message".to_string(),
            command,
            receiver
        ).await;
    }

    /// Request a file in the current DM you are in
    pub(crate) async fn request_file(
        &mut self,
        other_peer_id: PeerId,
        file_name: String
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::RequestFile { other_peer_id, filename: file_name.to_string(), sender };
        self.gracefully_handle_command(
            "request_file".to_string(),
            command,
            receiver
        ).await;
    }

    /// Respond with a file to a request in the current DM you are in
    pub(crate) async fn respond_file(
        &mut self,
        file_name: String,
        other_peer_id: PeerId,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::RespondFile { filename: file_name.to_string(), other_peer_id, sender };
        self.gracefully_handle_command(
            "respond_file".to_string(),
            command,
            receiver
        ).await;
    }

    pub(crate) async fn give_peer_rating(
        &mut self,
        rating: Rating
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::GivePeerRating { rating, sender };
        self.gracefully_handle_command(
            "give_peer_rating".to_string(),
            command,
            receiver
        ).await;
    }

    pub(crate) async fn request_peer_rating(
        &mut self,
        other_peer_id: PeerId,
    ) {
        let (sender, receiver) = oneshot::channel();
        let command = Command::RequestPeerRating { other_peer_id, sender };
        self.gracefully_handle_command(
            "request_peer_rating".to_string(),
            command,
            receiver
        ).await;
    }
}

/// Stores useful data for the network event loop, and handles incoming events from the App and network
pub(crate) struct EventLoop {
    peer_id: PeerId,
    encryption_public: x25519_dalek::PublicKey,
    encryption_secret: x25519_dalek::StaticSecret,
    swarm: Swarm<ClientBehaviour>,
    rendezvous_point: PeerId,
    command_receiver: mpsc::Receiver<Command>,
    event_sender: mpsc::Sender<Event>,
    connection_established: bool,
    discover_tick: Interval,
    public_key_hashmap: HashMap<PeerId, x25519_dalek::PublicKey>,
    pending_id_filename_hashmap: HashMap<OutboundRequestId, String>,
    pending_filename_channel_hashmap: HashMap<String, ResponseChannel<FileResponse>>
}

impl EventLoop {
    /// creates our event loop
    fn new(
        peer_id: PeerId,
        encryption_public: x25519_dalek::PublicKey,
        encryption_secret: x25519_dalek::StaticSecret,
        swarm: Swarm<ClientBehaviour>,
        rendezvous_point: PeerId,
        command_receiver: mpsc::Receiver<Command>,
        event_sender: mpsc::Sender<Event>,
    ) -> Self {
        let discover_tick = tokio::time::interval(Duration::from_secs(5));

        Self {
            peer_id,
            encryption_public,
            encryption_secret,
            swarm,
            connection_established: false,
            rendezvous_point,
            command_receiver,
            event_sender,
            discover_tick,
            public_key_hashmap: HashMap::new(),
            pending_id_filename_hashmap: HashMap::new(),
            pending_filename_channel_hashmap: HashMap::new()
        }
    }

    /// runs our event loop with async
    pub(crate) async fn run(mut self) {
        loop {
            // handle both events (from swarm/network) and commands (from app)
            tokio::select! {
                event = self.swarm.select_next_some() => self.handle_event(event).await,
                command = self.command_receiver.next() => match command {
                    Some(c) => self.handle_command(c).await,
                    None => return,
                },
                _ = self.discover_tick.tick() => {
                    if self.connection_established {
                        self.swarm.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(RENDEZVOUS_NAMESPACE.to_string()).unwrap()),
                            None,
                            None,
                            self.rendezvous_point
                        )
                    }
                }
            }
        }
    }

    /// handle events from our network/swarm
    async fn handle_event(&mut self, event: SwarmEvent<ClientBehaviourEvent>) {
        match event {
            // handle incoming ping events
            SwarmEvent::Behaviour(ClientBehaviourEvent::Ping(event)) => {
                let _ = self.event_sender.send(Event::SystemMessage(
                    format!("ping event {:?}", event)
                )).await;
            }

            // handle rendezvous connections established
            SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == self.rendezvous_point => {
                if let Err(err) = self.swarm.behaviour_mut().rendezvous.register(
                    rendezvous::Namespace::from_static("rendezvous"),
                    self.rendezvous_point,
                    None,
                ) {
                    let _ = self.event_sender.send(Event::SystemMessage(
                        format!("Failed to register {err}")
                    )).await;
                    return;
                }
                let _ = self.event_sender.send(Event::SystemMessage(
                    format!("Connection established with rendezvous point {peer_id}")
                )).await;

                self.connection_established = true;
            }

            // rendezvous for peer discovery
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::Rendezvous(
                    rendezvous::client::Event::Discovered {
                        registrations, ..
                    })
            ) => {
                for registration in registrations {
                    for address in registration.record.addresses() {
                        let other_peer_id = registration.record.peer_id();
                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("Discovered peer {other_peer_id}")
                        )).await;
                        let _ = self.event_sender.send(Event::NewListener(other_peer_id)).await;

                        let p2p_suffix = Protocol::P2p(other_peer_id);
                        let multiaddr =
                            if !address.ends_with(&Multiaddr::empty().with(p2p_suffix.clone())) {
                                address.clone().with(p2p_suffix)
                            } else {
                                address.clone()
                            };

                        self.swarm.behaviour_mut().gossipsub.add_explicit_peer(&other_peer_id);
                        self.swarm.behaviour_mut().kademlia.add_address(&other_peer_id, multiaddr.clone());
                        self.swarm.dial(multiaddr).unwrap();
                    }
                }
            }

            // rendezvous for peer discovery
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::Rendezvous(
                    rendezvous::client::Event::Expired {
                        peer: other_peer_id
                    })
            ) => {
                let _ = self.event_sender.send(Event::SystemMessage(
                    format!("Peer {other_peer_id} expired")
                )).await;
            }

            // listen for another peer subscribing to our pubsub
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::Gossipsub(
                    gossipsub::Event::Subscribed { peer_id, topic })
            ) => {
                // notify app of subscriber so they can display a joined message
                let _ = self.event_sender.send(Event::PeerSubscribed(peer_id, topic)).await;
            }

            // listen for another peer unsubscribing from our pubsub
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::Gossipsub(
                    gossipsub::Event::Unsubscribed { peer_id, topic })
            ) => {
                // notify app of unsubscriber so they can display a left message
                let _ = self.event_sender.send(Event::PeerUnsubscribed(peer_id, topic)).await;
            }

            // handle an incoming message
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::Gossipsub(
                    gossipsub::Event::Message {
                        propagation_source: sender_peer_id, message , .. })
            ) => {
                // match to determine which kind of message we are receiving
                match message.topic.to_string() {
                    // if its a DM
                    dm_topic if dm_topic.starts_with("DM_") => {
                        // get other public key as bytes
                        let other_public_key = match self.public_key_hashmap.get(&sender_peer_id) {
                            Some(public_key) => public_key,
                            None => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    format!("Received DM but couldn't \
                                    find public key for peer {}", sender_peer_id)
                                )).await;
                                return
                            }
                        };

                        // decrypt the message
                        let decrypted_message = decrypt_message(
                            other_public_key.clone(),
                            self.encryption_secret.clone(),
                            message.data
                        );

                        // notify the app of a received DM from peer
                        let received_dm_event = Event::ReceivedDirectMessage(
                            sender_peer_id, decrypted_message.clone()
                        );
                        let _ = self.event_sender.send(received_dm_event).await;
                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("DM '{decrypted_message}' received on topic {}", message.topic)
                        )).await;
                    }

                    // otherwise its a pubsub message
                    _ => {
                        // convert the vec of u8s to a string and send this to the app
                        let received_message_event = Event::ReceivedPubsubMessage(
                            sender_peer_id, String::from_utf8_lossy(&message.data).parse().unwrap()
                        );
                        let _ = self.event_sender.send(received_message_event).await;
                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("Message sent to pubsub on topic: {}", message.topic)
                        )).await;
                    }
                }
            },

            // handle dm notification events on swarm
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::DmNotification(
                    request_response::Event::Message { peer: sender_peer_id, message, .. })
            ) => {
                match message {
                    Message::Request { .. } => {
                        // Received a join notification from another peer
                        let _ = self.event_sender.send(Event::ReceivedDmNotification(
                            sender_peer_id)).await;
                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("Received dm notification from {}", sender_peer_id)
                        )).await;
                    }
                    Message::Response { .. } => {}
                }
            }
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::DmNotification(
                    request_response::Event::OutboundFailure { peer, error, .. })
            ) => {
                let _ = self.event_sender.send(Event::SystemMessage(
                    format!("Failed to send dm notification to {}, {}", peer, error)
                )).await;
            }

            // handle requested and responded files from req-res
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::RequestResponse(
                    request_response::Event::Message {
                        peer: sender_peer_id, message, ..
                    })
            ) => {
                // attempt to match the message to either a file request of a file response
                match message {
                    Message::Request { request, channel, .. } => {
                        // if we are requesting a file, then send a FileRequested event to the event loop
                        let filename = request.0;
                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("Received request for file {}", filename)
                        )).await;

                        // insert filename, channel pair into a hashmap such that we know
                        // where to send it back if it is accepted
                        self.pending_filename_channel_hashmap.insert(
                            filename.clone(), channel
                        );

                        let _ = self.event_sender.send(Event::FileRequested(
                            sender_peer_id,
                            filename
                        )).await;
                    },
                    Message::Response { response, request_id } => {
                        // if we are responding to a received file, then get the filename
                        let filename = match self.pending_id_filename_hashmap.remove(&request_id) {
                            Some(filename) => {
                                filename
                            },
                            None => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    format!("Failed to find pending filename \
                                        with request_id {}", request_id)
                                )).await;
                                return
                            }
                        };

                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("Received file {:?}", filename)
                        )).await;

                        let other_public_key = match self.public_key_hashmap.get(&sender_peer_id) {
                            Some(public_key) => public_key,
                            None => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    "Failed to find other public key for decryption".to_string()
                                )).await;
                                return
                            }
                        };

                        // decrypt the received bytes
                        let decrypted_bytes = decrypt_bytes(
                            other_public_key.clone(),
                            self.encryption_secret.clone(),
                            response.0
                        );

                        // write the received data to a file with filename
                        match write_data_to_file(self.peer_id, filename.clone(), decrypted_bytes).await {
                            Ok(_) => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    "Successfully wrote data to file".to_string()
                                )).await;

                                let _ = self.event_sender.send(Event::FileReceived(
                                    sender_peer_id, filename
                                )).await;
                            },
                            Err(err) => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    format!("Failed to write data to file, {}", err)
                                )).await;
                            }
                        }
                    }
                }
            }

            // notify the app that there was an outbound failure
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::RequestResponse(
                    request_response::Event::OutboundFailure { error, .. })
            ) => {
                let _ = self.event_sender.send(Event::SystemMessage(
                    format!("Outgoing request failed, {}", error)
                )).await;
            }

            // notify the app that there was an inbound failure
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::RequestResponse(
                    request_response::Event::InboundFailure { error, .. })
            ) => {
                let _ = self.event_sender.send(Event::SystemMessage(
                    format!("Incoming request failed, {}", error)
                )).await;
            }

            // handle kad dht outbound query progressed events
            SwarmEvent::Behaviour(
                ClientBehaviourEvent::Kademlia(
                    kad::Event::OutboundQueryProgressed { result, .. })
            ) => {
                // match the query result to the ones we care about
                match result {
                    // if we put a record successfully, then inform the system messages
                    kad::QueryResult::PutRecord(Ok(kad::PutRecordOk { key })) => {
                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("Successfully stored {} in DHT",
                                    std::str::from_utf8(key.as_ref()).unwrap())
                        )).await;
                    }

                    // if we failed to put a message, then inform the system messages
                    kad::QueryResult::PutRecord(Err(err)) => {
                        // match the type of key such that we can give a custom error message if the
                        // username fails to save
                        match std::str::from_utf8(err.key().as_ref()).unwrap() {
                            username_key if username_key.starts_with(USERNAME_KEY_PREFIX) => {
                                let _ = self.event_sender.send(Event::UsernameFailedToSave).await;
                            }
                            _ => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    format!("Failed to store record in DHT, {}", err)
                                )).await;
                            }
                        }
                    }

                    // handle if we request a record and it is found
                    kad::QueryResult::GetRecord(
                        Ok(kad::GetRecordOk::FoundRecord(
                               kad::PeerRecord {
                                   record: kad::Record { key, value, .. },
                                   ..
                               }))
                    ) => {
                        // match the type of found record
                        match std::str::from_utf8(key.as_ref()).unwrap() {
                            public_key_key if public_key_key.starts_with(PUBLIC_KEY_PREFIX) => {
                                // get peer id
                                let peer_id = match generate_peer_id_from_record_key(
                                    public_key_key, PUBLIC_KEY_PREFIX
                                ) {
                                    Some(peer_id) => peer_id,
                                    None => {
                                        let _ = self.event_sender.send(Event::SystemMessage(
                                            "Could not generate peer id from public-key key".to_string()
                                        )).await;
                                        return
                                    }
                                };

                                // get other public key as bytes
                                let public_key_as_bytes: [u8; 32] = match value.try_into() {
                                    Ok(bytes) => bytes,
                                    Err(_) => {
                                        let _ = self.event_sender.send(Event::SystemMessage(
                                            "Found public key bytes was not 32 bytes".to_string()
                                        )).await;
                                        return
                                    }
                                };

                                // convert to public key and store in hashmap
                                let public_key = x25519_dalek::PublicKey::from(public_key_as_bytes);
                                self.public_key_hashmap.insert(peer_id, public_key);
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    format!("Received public key for {}", peer_id)
                                )).await;
                            }

                            // if it is the pubsubs topic key
                            PUBSUB_TOPICS_KEY => {
                                // convert bytes to TimestampedPubSubList object and try
                                // to send this to the App
                                match serde_cbor::from_slice::<TimestampedPubSubList>(&value) {
                                    Ok(timestamped_topic_list) => {
                                        let _ = self.event_sender
                                            .send(Event::ReceivedPubsubTopics(
                                                timestamped_topic_list.clone()))
                                            .await;
                                    }
                                    Err(err) => {
                                        let _ = self.event_sender.send(Event::SystemMessage(
                                            format!("Failed to deserialise stored topic \
                                            list of TimeStampedPubSubList, {}", err)
                                        )).await;
                                    }
                                }
                            }
                            // if it is some username
                            username_key if username_key.starts_with(USERNAME_KEY_PREFIX) => {
                                // get the peer_id from the key itself
                                let other_peer_id = match generate_peer_id_from_record_key(
                                    username_key, USERNAME_KEY_PREFIX)
                                {
                                    Some(other_peer_id) => other_peer_id,
                                    None => {
                                        let _ = self.event_sender.send(Event::SystemMessage(
                                            "Failed to generate peer id from key".to_string()
                                        )).await;
                                        return
                                    }
                                };

                                // get the username from the bytes
                                let username = match String::from_utf8(value.clone()) {
                                    Ok(username) => username,
                                    Err(_err) => {
                                        let _ = self.event_sender.send(Event::SystemMessage(
                                            "Failed to parse username from DHT".to_string()
                                        )).await;
                                        return
                                    }
                                };

                                // send this information to the app for handling
                                let _ = self.event_sender
                                    .send(Event::ReceivedUsername(
                                        other_peer_id,
                                        username.clone()
                                    ))
                                    .await;

                                let _ = self.event_sender.send(Event::SystemMessage(
                                    format!("Received username {username} for {other_peer_id}", )
                                )).await;
                            }

                            // if it's a rating record
                            rating_key if rating_key.starts_with(RATING_KEY_PREFIX) => {
                                match serde_cbor::from_slice::<Rating>(&value) {
                                    Ok(rating) => {
                                        let _ = self.event_sender
                                            .send(Event::ReceivedPeerRating(
                                                rating.clone())
                                            )
                                            .await;
                                    }
                                    Err(err) => {
                                        let _ = self.event_sender.send(Event::SystemMessage(
                                            format!("Failed to deserialised received Rating, {}", err)
                                        )).await;
                                    }
                                }
                            }

                            _ => {}
                        }
                    }

                    // handle if we try to get the record and there is an error
                    kad::QueryResult::GetRecord(Err(err)) => {
                        // inform the app of this error (only handle some, as specific errors
                        // will constantly happen and spam the system messages
                        match std::str::from_utf8(err.key().as_ref()).unwrap() {
                            PUBSUB_TOPICS_KEY => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    format!("Failed to get pubsub topics, {}", err)
                                )).await;
                            },
                            username_key if username_key.starts_with(USERNAME_KEY_PREFIX) => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    format!("Failed to get username record, {}", err)
                                )).await;
                            },
                            public_key_key if public_key_key.starts_with(PUBLIC_KEY_PREFIX) => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    format!("Failed to get public key record, {}", err)
                                )).await;
                            },
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// handles commands from the app
    async fn handle_command(&mut self, command: Command) {
        match command {
            // ask the swarm to store our public key in dht
            Command::StorePublicKey { sender } => {
                // create record with public key
                let record = kad::Record {
                    key: RecordKey::new(&generate_record_key(self.peer_id, PUBLIC_KEY_PREFIX)),
                    value: self.encryption_public.as_bytes().to_vec(),
                    publisher: None,
                    expires: None,
                };

                // store it in dht
                match self.swarm.behaviour_mut().kademlia.put_record(record, kad::Quorum::One) {
                    Ok(_) => {
                        let _ = self.event_sender.try_send(Event::SystemMessage(
                            "Successfully stored public key in DHT".to_string()
                        ));
                    }
                    Err(e) => {
                        let _ = self.event_sender.try_send(Event::SystemMessage(
                            format!("Failed to store public key in DHT: {}", e)
                        ));
                    }
                };

                let _ = sender.send(Ok(()));
            }

            // request a user's public key from the dht
            Command::RequestPublicKey { other_peer_id, sender } => {
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .get_record(generate_record_key(other_peer_id, PUBLIC_KEY_PREFIX));
                let _ = sender.send(Ok(()));
            }

            // ask the swarm to subscribe to some pubsub
            Command::SubscribeToPubsub { topic, sender } => {
                // subscribe to pubsub with topic
                if let Err(err) = self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .subscribe(&topic)
                {
                    let _ = self.event_sender.try_send(Event::SystemMessage(
                        format!("Failed to subscribe to pubsub '{}'", err.to_string())
                    ));
                };

                let _ = sender.send(Ok(()));
            }

            // ask the swarm to unsubscribe from some topic
            Command::UnsubscribeToPubsub { topic, sender } => {
                // unsubscribe from pubsub with topic
                let _ = match self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .unsubscribe(&topic)
                {
                    true => {},
                    false => {
                        let _ = self.event_sender.try_send(Event::SystemMessage(
                            format!("Tried to unsubscribe from '{}' which we were not subscribed to", topic)
                        ));
                    },
                };

                // notify the client that we have handled this command
                let _ = sender.send(Ok(()));
            }

            // ask the swarm to send a message to some pubsub that we should be subscribed to
            Command::SendMessageToPubSub { topic, message, sender } => {
                // publish to pubsub with topic
                let _ = match self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), message.as_bytes())
                {
                    Ok(_) => {},
                    Err(e) => {
                        let _ = self.event_sender.try_send(Event::SystemMessage(
                            format!("Failed to send message to pubsub '{}'", e.to_string())
                        ));
                    },
                };

                // notify the client that we have handled this command
                let _ = sender.send(Ok(()));
            }

            // ask the swarm to store our username in the dht
            Command::StoreUsername { username, sender } => {
                // create a record for our username with a specific username + peer_id key
                let record = kad::Record {
                    key: RecordKey::new(&generate_record_key(self.peer_id, USERNAME_KEY_PREFIX)),
                    value: username.as_bytes().to_vec(),
                    publisher: None,
                    expires: None
                };

                // store in dht
                let _ = match self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .put_record(record, kad::Quorum::One)
                {
                    Ok(_) => {},
                    Err(e) => {
                        let _ = self.event_sender.try_send(Event::SystemMessage(
                            format!("Failed to store username in DHT '{}'", e.to_string())
                        ));
                    },
                };

                // notify the client that we have handled this command
                let _ = sender.send(Ok(()));
            }

            // ask the swarm to store a pubsub topic in the DHT
            Command::StorePubsub { topics, topic, sender } => {
                // take our current topics and add our new topic (assumes that this is up to date!)
                let mut topics = topics.clone();
                topics.push(topic.to_string());

                // take the current time (not based on timezone as this might have issues)
                let timestamped_pubsub_list = TimestampedPubSubList {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                    topics
                };

                // turn our data into bytes
                let data_as_bytes = match serde_cbor::to_vec(&timestamped_pubsub_list) {
                    Ok(data_as_bytes) => data_as_bytes,
                    Err(_) => {
                        let _ = self.event_sender.try_send(Event::SystemMessage(
                            "Failed to serialise our timestamped pubsub list".to_string()
                        ));
                        let _ = sender.send(Ok(()));
                        return
                    }
                };

                // create a record and save in dht
                let record = kad::Record {
                    key: RecordKey::new(&PUBSUB_TOPICS_KEY),
                    value: data_as_bytes,
                    publisher: None,
                    expires: None
                };
                if let Err(e) = self
                    .swarm.behaviour_mut()
                    .kademlia
                    .put_record(record, kad::Quorum::One)
                {
                    let _ = self.event_sender.try_send(Event::SystemMessage(
                        format!("Failed to store pubsub in DHT '{}'", e.to_string())
                    ));
                };

                // notify the client that we have handles this command
                let _ = sender.send(Ok(()));
            }

            // ask the swarm to request pubsub topics from the dht
            Command::RequestPubsubs { sender } => {
                self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .get_record(RecordKey::new(&PUBSUB_TOPICS_KEY));

                // notify the client that we have handles this command
                let _ = sender.send(Ok(()));
            },

            // ask the swarm to request a username for a given peer_id from the dht
            Command::RequestUsername { other_peer_id: peer_id, sender } => {
                // get record with the generated record key
                self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .get_record(generate_record_key(peer_id, USERNAME_KEY_PREFIX));

                // notify the client that we have handles this command
                let _ = sender.send(Ok(()));
            },

            // asks the swarm to request a file of filename from peer with PeerId
            Command::RequestFile { other_peer_id: peer_id, filename, sender } => {
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer_id, FileRequest(filename.clone()));

                // stores req_id, filename locally so that we can
                // fetch the filename when it is received
                self.pending_id_filename_hashmap.insert(request_id, filename);

                // notify the client that we have handles this command
                let _ = sender.send(Ok(()));
            },

            // asks the swarm to respond with some file of filename (which was requested)
            Command::RespondFile {filename, other_peer_id: peer_id, sender } => {
                // get the file_bytes from our filename, if we have this file
                let file_bytes = match retrieve_data_from_file(self.peer_id, filename.clone()).await {
                    Ok(file_bytes) => file_bytes,
                    Err(err) => {
                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("Failed to retrieve data from file, {}", err)
                        )).await;
                        let _ = sender.send(Ok(()));
                        return;
                    }
                };

                match self.pending_filename_channel_hashmap.remove(&filename) {
                    // if we have a channel to send this back over, then send it over that channel
                    Some(channel) => {
                        let other_public_key = match self.public_key_hashmap.get(&peer_id) {
                            Some(public_key) => public_key,
                            None => {
                                let _ = self.event_sender.send(Event::SystemMessage(
                                    "Failed to find other public key for encryption".to_string()
                                )).await;
                                let _ = sender.send(Ok(()));
                                return
                            }
                        };

                        // decrypt the received bytes
                        let encrypted_bytes = encrypt_bytes(
                            other_public_key.clone(),
                            self.encryption_secret.clone(),
                            file_bytes
                        );

                        let _ = self.swarm.behaviour_mut().request_response.send_response(
                            channel, FileResponse(encrypted_bytes)
                        );
                    }

                    // otherwise it is likely that this wasn't requested
                    None => {
                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("Failed to find channel for file {}", filename)
                        )).await;
                    }
                }

                // notify the client that we have handles this command
                let _ = sender.send(Ok(()));
            }

            // request the swarm to send a dm notification to our intended peer
            Command::SendDmNotification { other_peer_id, sender } => {
                let request = JoinDmRequest {};
                self.swarm.behaviour_mut().dm_notification.send_request(&other_peer_id, request);
                let _ = sender.send(Ok(()));
            }

            // request the swarm to send a direct message to some peer
            Command::DirectMessage { other_peer_id, message, sender } => {
                // find the public key of peer_id
                let other_public_key = match self.public_key_hashmap.get(&other_peer_id) {
                    Some(public_key) => {
                        public_key.to_owned()
                    }
                    None => {
                        let _ = self.event_sender.send(Event::SystemMessage(
                            format!("Could not find {}'s public key", other_peer_id)
                        )).await;
                        let _ = sender.send(Ok(()));
                        return
                    }
                };

                // encrypt our message with their peer id and our keypair
                let encrypted_message_as_bytes = encrypt_message(
                    other_public_key.clone(),
                    self.encryption_secret.clone(),
                    &message.clone(),
                );

                // generate the topic we are using for DMing said peer
                let topic = generate_pubsub_topic_for_dm(
                    self.peer_id,
                    other_peer_id
                );

                // send encrypted message over that pubsub topic
                let _ = match self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic, encrypted_message_as_bytes)
                {
                    Ok(_) => {
                        self.event_sender.send(Event::SystemMessage(
                            "Successfully sent DM".to_string()
                        ))
                    }
                    Err(err) => {
                        self.event_sender.send(Event::SystemMessage(
                            format!("Failed to send DM, {}", err)
                        ))
                    }
                };

                let _ = sender.send(Ok(()));
            }

            // Handles user rating a peer
            Command::GivePeerRating { rating, sender } => {
                // serialise our rating
                let data_as_bytes = match serde_cbor::to_vec(&rating) {
                    Ok(data_as_bytes) => data_as_bytes,
                    Err(_) => {
                        let _ = self.event_sender.try_send(Event::SystemMessage(
                            "Failed to serialise our rating".to_string()
                        ));
                        let _ = sender.send(Ok(()));
                        return
                    }
                };

                // make a record and store it
                let record = kad::Record {
                    key: RecordKey::new(&generate_record_key(rating.rated_peer_id, RATING_KEY_PREFIX)),
                    value: data_as_bytes,
                    publisher: None,
                    // expire after 1 hour
                    expires: Some(Instant::now() + Duration::from_secs(60 * 60 * 24 * 7)),
                };
                match self.swarm.behaviour_mut().kademlia.put_record(record, kad::Quorum::One) {
                    Ok(_) => {
                        let _ = self.event_sender.try_send(Event::SystemMessage(
                            "Successfully put peer rating record in dht".to_string()
                        ));
                    }
                    Err(err) => {
                        let _ = self.event_sender.try_send(Event::SystemMessage(
                            format!("Failed to put peer rating record in dht, {err}")
                        ));
                    }
                };

                let _ = sender.send(Ok(()));
            }

            // Handles getting user rating from kad dht
            Command::RequestPeerRating { other_peer_id: peer_id, sender } => {
                self.swarm.behaviour_mut().kademlia.get_record(
                    generate_record_key(peer_id, RATING_KEY_PREFIX)
                );

                let _ = sender.send(Ok(()));
            }

            // Generic error command (client errored) which will send the error message
            // along as a system message for the App to handle
            Command::Error { message } => {
                let _ = self.event_sender.try_send(Event::SystemMessage(message));
            }
        }
    }
}

/// generates a record key from our peer_id, useful for storing our name in the DHT
fn generate_record_key(
    peer_id: PeerId,
    prefix: &str,
) -> RecordKey {
    RecordKey::new(&(prefix.to_string() + peer_id.to_string().as_str()))
}

/// generates a record key from our peer_id
fn generate_peer_id_from_record_key(
    record_key: &str,
    prefix: &str,
) -> Option<PeerId> {
    match record_key.strip_prefix(prefix) {
        Some(peer_id) => PeerId::from_str(peer_id).ok(),
        None => None
    }
}

/// takes two peer_id and deterministically generates a pubsub topic
/// (such that both peers can connect to the same pubsub topic)
fn generate_pubsub_topic_for_dm(
    peer_id: PeerId,
    other_peer_id: PeerId,
) -> IdentTopic {
    let mut ids = vec![peer_id.to_string(), other_peer_id.to_string()];
    ids.sort();

    let topic = format!("DM_{}{}", ids[0], ids[1]);
    IdentTopic::new(topic)
}