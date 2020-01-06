use std::sync::Arc;

use codec::Encode;
use futures::channel::{mpsc, oneshot};
use futures::compat::{Compat, Compat01As03};
use futures::future::{FutureExt, Join, TryFutureExt};
use futures::prelude::{Future, Sink, Stream, TryFuture, TryStream};
use futures::sink::SinkExt;
use futures::stream::{FilterMap, StreamExt, TryStreamExt};
use futures::task::{Context, Poll};

// use tokio::runtime::current_thread::Runtime;

use sc_network::{config, Event as NetworkEvent, PeerId};
use sc_network_gossip::{TopicNotification, Validator};
use sc_network_test::{Block, Hash};
use sp_keyring::Ed25519Keyring;
use sp_mpc::MPC_ENGINE_ID;
use sp_runtime::ConsensusEngineId;

use super::{
	gossip::{GossipMessage, GossipValidator},
	message::ConfirmPeersMessage,
};

use crate::NodeConfig;

enum Event {
	EventStream(mpsc::UnboundedSender<NetworkEvent>),
	WriteNotification(sc_network::PeerId, Vec<u8>),
	Report(sc_network::PeerId, sc_network::ReputationChange),
	Announce(Hash),
}

#[derive(Clone)]
struct TestNetwork {
	sender: mpsc::UnboundedSender<Event>,
}

impl sc_network_gossip::Network<Block> for TestNetwork {
	fn event_stream(&self) -> Box<dyn futures01::Stream<Item = NetworkEvent, Error = ()> + Send> {
		let (tx, rx) = mpsc::unbounded();
		let _ = self.sender.unbounded_send(Event::EventStream(tx));
		Box::new(rx.map(Ok).compat())
	}

	fn report_peer(&self, who: sc_network::PeerId, cost_benefit: sc_network::ReputationChange) {
		let _ = self.sender.unbounded_send(Event::Report(who, cost_benefit));
	}

	fn disconnect_peer(&self, _: PeerId) {}

	fn write_notification(&self, who: PeerId, _: ConsensusEngineId, message: Vec<u8>) {
		let _ = self.sender.unbounded_send(Event::WriteNotification(who, message));
	}

	fn register_notifications_protocol(&self, _: ConsensusEngineId) {}

	fn announce(&self, block: Hash, _associated_data: Vec<u8>) {
		let _ = self.sender.unbounded_send(Event::Announce(block));
	}
}

impl sc_network_gossip::ValidatorContext<Block> for TestNetwork {
	fn broadcast_topic(&mut self, _: Hash, _: bool) {}

	fn broadcast_message(&mut self, _: Hash, _: Vec<u8>, _: bool) {}

	fn send_message(&mut self, who: &sc_network::PeerId, data: Vec<u8>) {
		<Self as sc_network_gossip::Network<Block>>::write_notification(self, who.clone(), MPC_ENGINE_ID, data);
	}

	fn send_topic(&mut self, _: &sc_network::PeerId, _: Hash, _: bool) {}
}

struct Tester {
	net_handle: super::NetworkBridge<Block>,
	gossip_validator: Arc<GossipValidator<Block>>,
	events: mpsc::UnboundedReceiver<Event>,
}

impl Tester {
	fn filter_network_events<F>(self, mut pred: F) -> impl TryFuture<Ok = Self, Error = ()>
	where
		F: FnMut(Event) -> bool,
	{
		let mut s = Some(self);
		futures::future::poll_fn(move |_| loop {
			match s.as_mut().unwrap().events.try_next().expect("concluded early") {
				None => panic!("concluded early"),
				Some(item) => {
					if pred(item) {
						return Poll::Ready(Ok(s.take().unwrap()));
					}
				}
			}
		})
	}
}

fn make_test_network(executor: &impl futures::task::Spawn) -> impl Future<Output = Tester> {
	let (tx, rx) = mpsc::unbounded();
	let net = TestNetwork { sender: tx };

	let config = NodeConfig {
		threshold: 1,
		players: 3,
		duration: 5,
	};

	let id = PeerId::random();
	let bridge = super::NetworkBridge::new(net.clone(), config, id, executor);

	futures::future::ready(Tester {
		gossip_validator: bridge.validator.clone(),
		net_handle: bridge,
		events: rx,
	})
}

struct NoopContext;

impl sc_network_gossip::ValidatorContext<Block> for NoopContext {
	fn broadcast_topic(&mut self, _: Hash, _: bool) {}
	fn broadcast_message(&mut self, _: Hash, _: Vec<u8>, _: bool) {}
	fn send_message(&mut self, _: &PeerId, _: Vec<u8>) {}
	fn send_topic(&mut self, _: &PeerId, _: Hash, _: bool) {}
}

#[test]
fn test_confirm_peer_message() {
	let id = PeerId::random();
	let threads_pool = futures::executor::ThreadPool::new().unwrap();

	let test = make_test_network(&threads_pool)
		.then(move |tester| {
			tester
				.gossip_validator
				.new_peer(&mut NoopContext, &id, config::Roles::AUTHORITY);
			futures::future::ready((tester, id))
		})
		.then(move |(tester, id)| {
			let (global_in, _) = tester.net_handle.global();

			let all_hash = {
				let inner = tester.gossip_validator.inner.read();
				inner.get_peers_hash()
			};
			let sender_id = id.clone();

			let msg_to_send = GossipMessage::ConfirmPeers(ConfirmPeersMessage::Confirming(0), all_hash);
			let msg_to_send_clone = msg_to_send.clone();

			let send_message = tester.filter_network_events(move |event| match event {
				Event::EventStream(sender) => {
					let _ = sender.unbounded_send(NetworkEvent::NotificationStreamOpened {
						remote: sender_id.clone(),
						engine_id: MPC_ENGINE_ID,
						roles: config::Roles::FULL,
					});

					let _ = sender.unbounded_send(NetworkEvent::NotificationsReceived {
						messages: vec![(MPC_ENGINE_ID, msg_to_send.encode().into())],
						remote: sender_id.clone(),
					});

					// let _ = sender.unbounded_send(NetworkEvent::NotificationStreamOpened {
					// 	remote: sc_network::PeerId::random(),
					// 	engine_id: MPC_ENGINE_ID,
					// 	roles: config::Roles::FULL,
					// });
					println!("send ok");
					true
				}
				_ => false,
			});

			let sender_id = id.clone();

			let handle_in = global_in
				.into_future() // get (item, tail_stream)
				.map(move |(item, _)| {
					println!("recv ok");
					let (msg, sender_opt) = item.unwrap();
					assert_eq!(msg, msg_to_send_clone);
					assert_eq!(sender_opt.unwrap(), sender_id.clone());
				});

			futures::future::join(send_message, handle_in)
		});

	let _ = futures::executor::block_on(test);
}
