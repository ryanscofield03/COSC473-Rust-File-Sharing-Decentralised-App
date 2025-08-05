use std::{error::Error, time::Duration};

use futures::StreamExt;
use libp2p::{identify, noise, ping, rendezvous, swarm::{NetworkBehaviour, SwarmEvent}, tcp, yamux};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    // Results in PeerId("12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN")
    let keypair = libp2p::identity::Keypair::ed25519_from_bytes([0; 32]).unwrap();
    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_behaviour(|key| ServerBehaviour {
            identify: identify::Behaviour::new(identify::Config::new(
                "rendezvous/1.0.0".to_string(),
                key.public(),
            )),
            rendezvous: rendezvous::server::Behaviour::new(rendezvous::server::Config::default()),
            ping: ping::Behaviour::new(ping::Config::new().with_interval(Duration::from_secs(10))
                .with_timeout(Duration::from_secs(10))),
        })?
        .build();

    let _ = swarm.listen_on("/ip4/0.0.0.0/tcp/62649".parse().unwrap());
    let _ = swarm.listen_on("/ip4/0.0.0.0/udp/62649/quic-v1".parse()?);

    while let Some(event) = swarm.next().await {
        match event {
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                println!("Connected to {}", peer_id);
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                println!("Disconnected from {}", peer_id);
            }
            SwarmEvent::Behaviour(
                ServerBehaviourEvent::Rendezvous(
                    rendezvous::server::Event::PeerRegistered {
                        peer, registration
                    })
            ) => {
                println!(
                    "Peer {} registered for namespace '{}'",
                    peer,
                    registration.namespace
                );
            }
            SwarmEvent::Behaviour(
                ServerBehaviourEvent::Rendezvous(
                    rendezvous::server::Event::DiscoverServed {
                        enquirer: other_peer_id, registrations
                    })
            ) => {
                println!(
                    "Served peer {} with {} registrations",
                    other_peer_id,
                    registrations.len()
                );
            }
            _ => {}
        }
    }

    Ok(())
}

#[derive(NetworkBehaviour)]
struct ServerBehaviour {
    identify: identify::Behaviour,
    rendezvous: rendezvous::server::Behaviour,
    ping: ping::Behaviour,
}
