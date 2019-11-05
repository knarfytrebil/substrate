use std::sync::Arc;

use codec::Encode;
use futures::prelude::{Future as Future01, Sink as Sink01, Stream as Stream01};
use futures::sync::mpsc as mpsc01;
use futures03::channel::{mpsc, oneshot};
use futures03::compat::{Compat, Compat01As03};
use futures03::future::{FutureExt, Join};
use futures03::prelude::{Future, Sink, Stream, TryFuture, TryStream};
use futures03::sink::SinkExt;
use futures03::stream::{FilterMap, StreamExt, TryStreamExt};
use futures03::task::{Context, Poll};

use tokio02::runtime::current_thread::Runtime;

use keyring::Ed25519Keyring;
use network::{
	consensus_gossip as network_gossip,
	test::{Block, Hash},
};
use network_gossip::{TopicNotification, Validator};

use super::{
	gossip::{GossipMessage, GossipValidator},
	message::{ConfirmPeersMessage, KeyGenMessage, SignMessage},
};

use crate::NodeConfig;

enum Event {
	MessagesFor(Hash, mpsc::UnboundedSender<TopicNotification>),
	RegisterValidator(Arc<dyn Validator<Block>>),
	GossipMessage(Hash, Vec<u8>, bool),
	SendMessage(Vec<network::PeerId>, Vec<u8>),
	Report(network::PeerId, i32),
	Announce(Hash),
}

#[derive(Clone)]
struct TestNetwork {
	sender: mpsc::UnboundedSender<Event>,
}

impl super::Network<Block> for TestNetwork {
	type In = mpsc::UnboundedReceiver<TopicNotification>;

	fn messages_for(&self, topic: Hash) -> Self::In {
		let (tx, rx) = mpsc::unbounded();
		let _ = self.sender.unbounded_send(Event::MessagesFor(topic, tx));

		rx
	}

	fn register_validator(&self, validator: Arc<dyn Validator<Block>>) {
		let _ = self
			.sender
			.unbounded_send(Event::RegisterValidator(validator));
	}

	fn gossip_message(&self, topic: Hash, data: Vec<u8>, force: bool) {
		let _ = self
			.sender
			.unbounded_send(Event::GossipMessage(topic, data, force));
	}

	fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>) {
		let _ = self.sender.unbounded_send(Event::SendMessage(who, data));
	}

	fn register_gossip_message(&self, _topic: Hash, _data: Vec<u8>) {
		// NOTE: only required to restore previous state on startup
		//       not required for tests currently
	}

	fn report(&self, who: network::PeerId, cost_benefit: i32) {
		let _ = self.sender.unbounded_send(Event::Report(who, cost_benefit));
	}

	/// Inform peers that a block with given hash should be downloaded.
	fn announce(&self, block: Hash, _associated_data: Vec<u8>) {
		let _ = self.sender.unbounded_send(Event::Announce(block));
	}
}

impl network_gossip::ValidatorContext<Block> for TestNetwork {
	fn broadcast_topic(&mut self, _: Hash, _: bool) {}

	fn broadcast_message(&mut self, _: Hash, _: Vec<u8>, _: bool) {}

	fn send_message(&mut self, who: &network::PeerId, data: Vec<u8>) {
		<Self as super::Network<Block>>::send_message(self, vec![who.clone()], data);
	}

	fn send_topic(&mut self, _: &network::PeerId, _: Hash, _: bool) {}
}

struct Tester {
	net_handle: super::NetworkBridge<Block, TestNetwork>,
	gossip_validator: Arc<GossipValidator<Block>>,
	events: mpsc::UnboundedReceiver<Event>,
}

impl Tester {
	fn filter_network_events<F>(self, mut pred: F) -> impl TryFuture<Ok = Self, Error = ()>
	where
		F: FnMut(Event) -> bool,
	{
		let mut s = Some(self);
		futures03::future::poll_fn(move |_| loop {
			match s
				.as_mut()
				.unwrap()
				.events
				.try_next()
				.expect("concluded early")
			{
				None => panic!("concluded early"),
				Some(item) => {
					if pred(item) {
						return Poll::Ready(Ok(s.take().unwrap()));
					}
				} // Poll::Pending => return Poll::Pending,
			}
		})
	}
}

fn make_test_network() -> impl Future<Output = Tester> {
	let (tx, rx) = mpsc::unbounded();
	let net = TestNetwork { sender: tx };

	let config = NodeConfig {
		threshold: 1,
		players: 3,
		keystore: None,
		duration: 5,
	};

	let id = network::PeerId::random();
	let bridge = super::NetworkBridge::new(net.clone(), config, id);

	futures03::future::lazy(move |_| Tester {
		gossip_validator: bridge.validator.clone(),
		net_handle: bridge,
		events: rx,
	})
}
struct NoopContext;

impl network_gossip::ValidatorContext<Block> for NoopContext {
	fn broadcast_topic(&mut self, _: Hash, _: bool) {}
	fn broadcast_message(&mut self, _: Hash, _: Vec<u8>, _: bool) {}
	fn send_message(&mut self, _: &network::PeerId, _: Vec<u8>) {}
	fn send_topic(&mut self, _: &network::PeerId, _: Hash, _: bool) {}
}

#[test]
fn test_confirm_peer_message() {
	let id = network::PeerId::random();

	let global_topic = super::string_topic::<Block>("hash");

	let test = make_test_network()
		.then(async move |tester| {
			tester.gossip_validator.new_peer(
				&mut NoopContext,
				&id,
				network::config::Roles::AUTHORITY,
			);

			(tester, id)
		})
		.then(async move |(tester, id)| {
			let (global_in, _) = tester.net_handle.global();

			let all_hash = {
				let inner = tester.gossip_validator.inner.read();
				inner.get_peers_hash()
			};
			let sender_id = id.clone();
			let msg_to_send =
				GossipMessage::ConfirmPeers(ConfirmPeersMessage::Confirming(0), all_hash);

			let send_message = tester.filter_network_events(move |event| match event {
				Event::MessagesFor(topic, sender) => {
					if topic != global_topic {
						return false;
					}
					let _ = sender.unbounded_send(TopicNotification {
						message: msg_to_send.encode(),
						sender: Some(sender_id.clone()),
					});

					true
				}
				_ => false,
			});

			let sender_id = id.clone();

			let handle_in = global_in
				.map(move |(msg, sender_opt)| {
					match msg {
						GossipMessage::ConfirmPeers(
							ConfirmPeersMessage::Confirming(from),
							hash,
						) => {
							assert_eq!(from, 0);
							assert_eq!(hash, all_hash);
						}
						_ => panic!("invalid msg"),
					}

					assert_eq!(sender_opt.unwrap(), sender_id.clone());
				})
				.into_future();

			futures03::future::join(send_message, handle_in)
			// .map_err(|_| panic!("could not watch for gossip message"))
			// .map_ok(|_| ())
		});

	let mut runtime = Runtime::new().unwrap();
	let _ = runtime.block_on(test);
}
