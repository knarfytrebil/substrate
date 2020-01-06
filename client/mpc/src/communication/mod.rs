use std::{future, marker::Unpin, pin::Pin, sync::Arc};

use codec::{Decode, Encode};
use futures::channel::oneshot::{self, Canceled};
use futures::compat::{Compat, Compat01As03};
use futures::future::FutureExt;
use futures::prelude::{Future, Sink, Stream, TryStream};
use futures::stream::{FilterMap, StreamExt, TryStreamExt};
use futures::task::{Context, Poll};
use log::{error, info, trace};

use sc_network::message::generic::{ConsensusMessage, Message};
use sc_network::{NetworkService, PeerId};
use sc_network_gossip::{GossipEngine, Network, TopicNotification};
use sp_runtime::traits::{Block as BlockT, DigestFor, Hash as HashT, Header as HeaderT, NumberFor, ProvideRuntimeApi};

use sp_mpc::MPC_ENGINE_ID;

pub mod gossip;
pub mod message;
mod peer;

use crate::{Error, NodeConfig};

use gossip::{GossipMessage, GossipValidator, MessageWithReceiver, MessageWithSender, RequestId};
use message::ConfirmPeersMessage;

pub(crate) fn bytes_topic<B: BlockT>(input: &[u8]) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(input)
}

struct MessageSender<Block: BlockT> {
	network: GossipEngine<Block>,
	validator: Arc<GossipValidator<Block>>,
}

impl<Block> MessageSender<Block>
where
	Block: BlockT,
{
	fn broadcast(&self, msg: GossipMessage) {
		let raw_msg = msg.encode();
		let inner = self.validator.inner.read();
		let peers = inner.get_other_peers();
		self.network.send_message(peers, raw_msg);
	}

	fn send_message(&self, target: PeerId, msg: GossipMessage) {
		let raw_msg = msg.encode();
		self.network.send_message(vec![target], raw_msg);
	}
}

impl<Block> Sink<MessageWithReceiver> for MessageSender<Block>
where
	Block: BlockT,
{
	type Error = Error;

	fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn start_send(self: Pin<&mut Self>, item: MessageWithReceiver) -> Result<(), Self::Error> {
		let (msg, receiver_opt) = item;

		if let Some(receiver) = receiver_opt {
			self.send_message(receiver, msg);
		} else {
			self.broadcast(msg);
		}

		Ok(())
	}

	fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}
}

pub(crate) struct NetworkBridge<B: BlockT> {
	pub gossip_engine: GossipEngine<B>,
	pub validator: Arc<GossipValidator<B>>,
}

impl<B> NetworkBridge<B>
where
	B: BlockT,
{
	pub fn new<N: Network<B> + Clone + Send + 'static>(
		service: N,
		config: NodeConfig,
		local_peer_id: PeerId,
		executor: &impl futures::task::Spawn,
	) -> Self {
		let validator = Arc::new(GossipValidator::new(config, local_peer_id));
		let gossip_engine = GossipEngine::new(service, executor, MPC_ENGINE_ID, validator.clone());
		Self {
			gossip_engine,
			validator,
		}
	}

	pub fn global(
		&self,
	) -> (
		impl Stream<Item = MessageWithSender>,
		impl Sink<MessageWithReceiver, Error = Error>,
	) {
		let topic = bytes_topic::<B>(b"hash"); // related with `fn validate` in gossip.rs

		let incoming = self.gossip_engine.messages_for(topic).filter_map(|notification| async {
			let decoded = GossipMessage::decode(&mut &notification.message[..]);
			if let Err(e) = decoded {
				trace!("notification error {:?}", e);
				return None;
			}
			Some((decoded.unwrap(), notification.sender))
		});

		let outgoing = MessageSender {
			network: self.gossip_engine.clone(),
			validator: self.validator.clone(),
		};

		(incoming.boxed(), outgoing)
	}

	pub fn start_key_gen(&self, _id: RequestId) {
		let inner = self.validator.inner.read();

		let all_peers_len = inner.get_peers_len();
		let players = inner.get_players() as usize;
		if all_peers_len != players {
			return;
		}

		let our_index = inner.get_local_index() as u16;
		let all_peers_hash = inner.get_peers_hash();
		let msg = GossipMessage::ConfirmPeers(ConfirmPeersMessage::Confirming(our_index), all_peers_hash);
		let peers = inner.get_other_peers();
		self.gossip_engine.send_message(peers, msg.encode());
	}
}

impl<B: BlockT> Clone for NetworkBridge<B> {
	fn clone(&self) -> Self {
		Self {
			gossip_engine: self.gossip_engine.clone(),
			validator: Arc::clone(&self.validator),
		}
	}
}

#[cfg(test)]
mod tests;
