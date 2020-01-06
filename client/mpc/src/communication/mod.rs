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
use sp_runtime::traits::{
	Block as BlockT, DigestFor, Hash as HashT, Header as HeaderT, NumberFor, ProvideRuntimeApi,
};

use sp_mpc::MPC_ENGINE_ID;

pub mod gossip;
pub mod message;
mod peer;

use crate::{NodeConfig, Error};

use gossip::{GossipMessage, GossipValidator, MessageWithReceiver, MessageWithSender};
use message::{ConfirmPeersMessage, KeyGenMessage, SignMessage};

pub struct NetworkStream<R> {
	inner: Option<R>,
	outer: oneshot::Receiver<R>,
}

impl<R> Stream for NetworkStream<R>
where
	R: Stream<Item = TopicNotification> + Unpin,
{
	type Item = R::Item;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
		if let Some(ref mut inner) = self.as_mut().inner {
			return inner.poll_next_unpin(cx);
		}

		match self.outer.poll_unpin(cx) {
			Poll::Ready(r) => match r {
				Ok(mut inner) => {
					let poll_result = inner.poll_next_unpin(cx);
					self.inner = Some(inner);
					poll_result
				}
				Err(Canceled) => panic!("Oneshot cancelled"),
			},
			Poll::Pending => Poll::Pending,
		}
	}
}

pub(crate) fn hash_topic<B: BlockT>(hash: u64) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(&hash.to_be_bytes())
}

pub(crate) fn string_topic<B: BlockT>(input: &str) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(input.as_bytes())
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
		let topic = string_topic::<B>("hash"); // related with `fn validate` in gossip.rs

		let incoming = self
			.gossip_engine
			.messages_for(topic)
			.filter_map(|notification| {
				async {
					let decoded = GossipMessage::decode(&mut &notification.message[..]);
					if let Err(e) = decoded {
						trace!("notification error {:?}", e);
						println!("NOTIFICATION ERROR");
						return None;
					}
					println!("sender in global {:?}", notification.sender);
					Some((decoded.unwrap(), notification.sender))
				}
			});

		let outgoing = MessageSender::<B> {
			network: self.gossip_engine.clone(),
			validator: self.validator.clone(),
		};

		(incoming.boxed(), outgoing)
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

// #[cfg(test)]
// mod tests;
