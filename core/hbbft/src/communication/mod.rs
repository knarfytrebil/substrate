// Copyright 2017-2019 Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

//! Communication streams for the polite-grandpa networking protocol.
//!
//! GRANDPA nodes communicate over a gossip network, where messages are not sent to
//! peers until they have reached a given round.
//!
//! Rather than expressing protocol rules,
//! polite-grandpa just carries a notion of impoliteness. Nodes which pass some arbitrary
//! threshold of impoliteness are removed. Messages are either costly, or beneficial.
//!
//! For instance, it is _impolite_ to send the same message more than once.
//! In the future, there will be a fallback for allowing sending the same message
//! under certain conditions that are used to un-stick the protocol.
use network::consensus_gossip::ValidatorContext;
use std::sync::Arc;
use std::iter;
use std::collections::{VecDeque};
use std::time::{Duration, Instant};
use network::consensus_gossip:: MessageIntent;
use runtime_primitives::traits::NumberFor;
use badger::dynamic_honey_badger::DynamicHoneyBadger;
use badger::queueing_honey_badger::{Batch, QueueingHoneyBadger};
use badger::sender_queue::{Message as BMessage, SenderQueue};
use badger::{ConsensusProtocol, CpStep, NetworkInfo, Step, Target,Contribution};
use rand::{distributions::Standard, rngs::OsRng, seq::SliceRandom, Rng};
use network::PeerId;
//use grandpa::{voter, voter_set::VoterSet};
//use grandpa::Message::{Prevote, Precommit, PrimaryPropose};
use futures03::prelude::*;
use futures03::channel::{oneshot, mpsc};
use log::{debug, trace};
use tokio_executor::Executor;
use parity_codec::{Encode, Decode};
use substrate_primitives::{ed25519, Pair};
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG, CONSENSUS_INFO};
use runtime_primitives::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use network::{consensus_gossip as network_gossip, NetworkService};
use network_gossip::ConsensusMessage;
use crate::communication::gossip::GreetingMessage;

use gossip::GossipMessage;
use gossip::Peers;
use gossip::Action;
use fg_primitives::{AuthorityId, AuthoritySignature};
//use substrate_primitives::ed25519::{Public as AuthorityId, Signature as AuthoritySignature};
use network::config::Roles;

pub mod gossip;

use crate::Error;

const REBROADCAST_AFTER: Duration = Duration::from_secs(60 * 5);


#[cfg(test)]
mod tests;

pub use fg_primitives::HBBFT_ENGINE_ID;
use badger::{SourcedMessage as BSM,  TargetedMessage};
// cost scalars for reporting peers.

/// A handle to the network. This is generally implemented by providing some
/// handle to a gossip service or similar.
///
/// Intended to be a lightweight handle such as an `Arc`.
pub trait Network<Block: BlockT>: Clone + Send + 'static {
	/// A stream of input messages for a topic.
	type In: Stream<Item=network_gossip::TopicNotification>;

	/// Get a stream of messages for a specific gossip topic.
	fn messages_for(&self, topic: Block::Hash) -> Self::In;

	/// Register a gossip validator.
	fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<BlockT>>);

	/// Gossip a message out to all connected peers.
	///
	/// Force causes it to be sent to all peers, even if they've seen it already.
	/// Only should be used in case of consensus stall.
	fn gossip_message(&self, topic: Block::Hash, data: Vec<u8>, force: bool);

	/// Register a message with the gossip service, it isn't broadcast right
	/// away to any peers, but may be sent to new peers joining or when asked to
	/// broadcast the topic. Useful to register previous messages on node
	/// startup.
	fn register_gossip_message(&self, topic: Block::Hash, data: Vec<u8>);

	/// Send a message to a bunch of specific peers, even if they've seen it already.
	fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>);

	/// Report a peer's cost or benefit after some action.
	fn report(&self, who: network::PeerId, cost_benefit: i32);

	/// Inform peers that a block with given hash should be downloaded.
	fn announce(&self, block: Block::Hash);
}

/// Create a unique topic for a round and set-id combo.
pub(crate) fn round_topic<B: BlockT>(round: u64, set_id: u64) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(format!("{}-{}", set_id, round).as_bytes())
}

pub fn badger_topic<B: BlockT>() -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(format!("badger-mushroom").as_bytes())
}


/// Create a unique topic for global messages on a set ID.
pub(crate) fn global_topic<B: BlockT>(set_id: u64) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(format!("{}-GLOBAL", set_id).as_bytes())
}

#[derive(Eq, PartialEq, Debug, Encode, Decode)]
struct SourcedMessage<D: ConsensusProtocol> {
    sender_id: D::NodeId,
    target: Target<D::NodeId>,
    message: Vec<u8>,
}


pub struct BadgerNode<B: BlockT,D,R: Rng> 
where 
D: ConsensusProtocol<NodeId=PeerId> //specialize to avoid some of the confusion
{
    /// This node's own ID.
    id: PeerId,
    /// The instance of the broadcast algorithm.
    algo: D,

	main_rng:R,

	peers: Peers<NumberFor<BlockT>>,
	authorities: Vec<AuthorityId>,
	config: crate::Config,
	//next_rebroadcast: Instant,
	/// Incoming messages from other nodes that this node has not yet handled.
    in_queue: VecDeque<SourcedMessage<D>>,
    /// Outgoing messages to other nodes.
    out_queue: VecDeque<SourcedMessage<D>>,
    /// The values this node has output so far, with timestamps.
    outputs: Vec<D::Output>,

}

impl<B: BlockT,D: ConsensusProtocol,R: Rng> BadgerNode<B,D,R>
{
fn register_peer_public_key(&mut self,who :&PeerId, auth:AuthorityId)
 {
  self.peers.update_id(who,auth)
 }

 fn is_authority(&self, who :&PeerId) -> bool
  {
	  let auth=self.peers.peer(who);
	  match auth
	  {
		  Some(info) =>
		  {
			  self.authorities.contains(&info.id)
		  },
           None => false
	  }  
  }
}



pub type BadgerNodeStepResult<D> = CpStep<D>;
pub type TransactionSet = Vec<Vec<u8>>; //agnostic?
impl<B: BlockT,D: ConsensusProtocol,R: Rng> BadgerNode<B,D,R>
{
	pub fn new(config: crate::Config, self_id:PeerId) -> BadgerNode<B,D,R>
	{
	let rng=OsRng::new();
	let ni=NetworkInfo::<D::NodeId>::new(self_id.clone(),config.secret_key_share,config.public_key_set,config.node_id.1.clone(),config.initial_validators.clone());
    let dhb = DynamicHoneyBadger::builder().build(ni);
	let (qhb, qhb_step) = QueueingHoneyBadger::builder(dhb)
            .batch_size(config.batch_size)
            .build( rng).expect("instantiate QueueingHoneyBadger");
	let peer_ids: Vec<_> = ni
            .all_ids()
            .filter(|&&them| them != self_id)
            .cloned()
            .collect();
	 let (sq, mut step) = SenderQueue::builder(qhb, peer_ids.into_iter()).build(self_id.clone());
     let output = step.extend_with(qhb_step, |fault| fault, BMessage::from);
     let out_queue = output
            .messages
            .into_iter()
            .map(|msg| {
                let ser_msg = bincode::serialize(&msg.message).expect("serialize");
                SourcedMessage {
                    sender_id: sq.our_id().clone(),
                    target: msg.target,
                    message: ser_msg,
                }
            }).collect();
	 let outputs = output
            .output
            .into_iter()
            .collect();
     let mut node=BadgerNode {
		 id: self_id,
		 algo: sq,
         main_rng: OsRng::new(),
         peers:  Peers::<NumberFor<BlockT>>::new(),
		 authorities : config.initial_validators.clone().to_inter().map(|_,val| val).collect(),
		 config: config.clone(),
         in_queue:  VecDeque::new(),
		    out_queue: out_queue,
			outputs:outputs
	 };
	 	for (k,v) in config.initial_validators.clone()
		 {
			 node.register_peer_public_key(k,v)
		 }
		 node 
	}
	pub fn  handle_message(&mut self, who: AuthorityId, msg:  D::Message) -> Result<(),&'static str>
	{
		match   self.algo.handle_message(&who, msg, &self.main_rng) 
		{
			Some(step) => 
			 {
              let out_msgs: Vec<_> = step
            .messages
            .into_iter()
            .map(|mmsg| {
                let ser_msg = bincode::serialize(&mmsg.message).expect("serialize");
                (mmsg.target, ser_msg)
                }).collect();
		    self.outputs.extend(step.output.into_iter());	
			    for (target, message) in out_msgs 
				{
                 self.out_queue.push_back(SourcedMessage {
                   sender_id: self.id.clone(),
                   target,
                   message,});
				}
				
				Ok(())
			 }
			None => return Err("Cannot handle message")
		}
	}
}

pub(super) struct BadgerGossipValidator<Block: BlockT,D: ConsensusProtocol,R: Rng> {
	inner: parking_lot::RwLock<BadgerNode<Block,D,R>>,
}
impl<Block: BlockT,D: ConsensusProtocol,R: Rng> BadgerGossipValidator<Block,D,R> {
	   fn flush_messages(&self,context: &mut dyn ValidatorContext<BlockT>)
   {
    let locked= self.inner.write();
	let topic = badger_topic::<Block>();    
		 for msg in locked.out_msgs.drain()
		 {
			 let vdata=GossipMessage::BadgerData(msg.message).encode();
			 match &msg.target
			 {
				 Target::All  => context.broadcast_message(topic,vdata,true),
				Target::Node(to_id) => 
				  {
				  context.send_message(to_id, vdata);

				  },
              Target::AllExcept(exclude) => {
				    
                    for pid in self.peers.peer_list().filter(|n| !exclude.contains(&n)) {
                        if pid != msg.sender_id {
                            context.send_message(pid, vdata);
                        }
                    }
                   }

			 }
			
		 }
   }

	/// Create a new gossip-validator. 
	pub(super) fn new(config: crate::Config, self_id:PeerId)
		-> BadgerGossipValidator<BlockT>
	{
		let val = BadgerGossipValidator {
			inner: parking_lot::RwLock::new(BadgerNode::new(config,self_id)),
		};

		val
	}
    /// collect outputs from 
    pub fn pop_output(&mut self) -> Option<D::Output>
	{
      let locked = self.inner.write();
	  locked.outputs.pop()
	}
	/// Note that we've processed a catch up message.
	pub(super) fn note_catch_up_message_processed(&self)	{
		self.inner.write().note_catch_up_message_processed();
	}
    
	pub  fn is_validator(&self) ->bool
	{
		let rd = self.inner.read();
		rd.is_authority(&rd.id)
	}
    pub fn push_transaction<T:Contribution>(&mut self,tx: T) ->Result<(),Error>
	{
       let locked=self.inner.write();
	   match locked.algo.push_transaction(tx,locked.main_rng)
	   {
		   Ok(step) => {
			let out_msgs: Vec<_> = step
             .messages
             .into_iter()
             .map(|mmsg| {
                let ser_msg = bincode::serialize(&mmsg.message).expect("serialize");
                (mmsg.target, ser_msg)
                }).collect();
		    locked.outputs.extend(step.output.into_iter());	
			    for (target, message) in out_msgs 
				{
                 locked.out_queue.push_back(SourcedMessage {
                   sender_id: self.id.clone(),
                   target,
                   message,});
				}
				//send messages out
				self.flush_messages();
		   },
		   Err(e) => return e
	   }

	}

	pub(super) fn do_validate(&self, who: &PeerId, mut data: &[u8])
		-> (Action<Block::Hash>,  Option<GossipMessage<Block>>)
	{

		let mut peer_reply = None;


		let action = {
			match GossipMessage::<Block>::decode(&mut data) {
					Some(GossipMessage::Greeting(msg))  =>
					{
						if msg.mySig.verify(msg.myId.to_bytes()) 
						 {
							self.inner.write().register_peer_public_key(who,msg.myId);
							Action::ProcessAndKeep()
						 } 
						 else
						 {
						 Action::Discard(-1)
						 }
					},
					Some(GossipMessage::RequestGreeting) =>
					{
						let rd=self.inner.read().unwrap();
						let msrep=GreetingMessage { myId: rd.id.clone(),mySig: rd.config.node_id.1.sign(rd.id.clone().to_bytes())} ;
                    peer_reply = Some(GossipMessage::Greeting(msrep));
					Action::ProcessAndDiscard()
					},
					Some(GossipMessage::BadgerData(badger_msg)) => 
					{
						
					let locked=self.inner.write();
					if locked.is_authority(who) 
					 {
						if let Some(msg) = bincode::deserialize::<D::Message>(&badger_msg)
						{
						match locked.handle_message(locked.peers.peer(who).unwrap(),msg)
						{
							Ok(_) => 
							{
                            //send is handled separately. trigger propose? or leave it for stream
                            if locked.outputs.len()>0 
						 	 {
							  

							 }
							}
							Err(e) =>
							{
							telemetry!(CONSENSUS_DEBUG; "afg.err_handling_msg"; "err" => ?format!("{}", e));
							Action::Discard(-1)
							}
						}
						}
						else
						{
							Action::Discard(-1)
						}
					 } 
					else
					 { 
						Action::Discard(-1) 
					 }
					},

				None => {
					debug!(target: "afg", "Error decoding message");
					telemetry!(CONSENSUS_DEBUG; "afg.err_decoding_msg"; "" => "");

					let len = std::cmp::min(i32::max_value() as usize, data.len()) as i32;
					Action::Discard(-len)
				}
			}
		};

		(action,  peer_reply)
	}
}


impl<Block: BlockT> network_gossip::Validator<BlockT> for BadgerGossipValidator<BlockT> {
	fn new_peer(&self, context: &mut dyn ValidatorContext<BlockT>, who: &PeerId, _roles: Roles) 
	{
		{
			let mut inner = self.inner.write();
			inner.peers.new_peer(who.clone());
		};
		let packet = {
			let mut inner = self.inner.write();
			inner.peers.new_peer(who.clone());
            GreetingMessage
			{
				myId: self.inner.config.node_id.0,
				mySig: self.inner.config.node_id.1.sign(self.inner.config.node_id.0.clone())
			}
		};

		if let Some(packet) = packet {
			let packet_data = GossipMessage::<BlockT>::from(packet).encode();
			context.send_message(who, packet_data);
		}
	}

	fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<BlockT>, who: &PeerId) 
	{
		self.inner.write().peers.peer_disconnected(who);
	}

	fn validate(&self, context: &mut dyn ValidatorContext<BlockT>, who: &PeerId, data: &[u8])
		-> network_gossip::ValidationResult<Block::Hash>
	{
		let (action,  peer_reply) = self.do_validate(who, data);
        let topic = badger_topic::<BlockT>();
		// not with lock held!
		if let Some(msg) = peer_reply {
			context.send_message(who, msg.encode());
		}
		context.send_topic(who, topic, false);

		{
        self.flush_messages();
		}
		
		match action {
			Action::Keep() => {
				context.broadcast_message(topic, data.to_vec(), false);
				network_gossip::ValidationResult::ProcessAndKeep(topic)
			}
			Action::ProcessAndDiscard() => {
				
				network_gossip::ValidationResult::ProcessAndDiscard(topic)
			}
			Action::Discard(cb) => {
				self.report(who.clone(), cb);
				network_gossip::ValidationResult::Discard(0)
			}
		}
	}

	fn message_allowed<'a>(&'a self) //todo: not sure what this is for
		-> Box<dyn FnMut(&PeerId, MessageIntent, &Block::Hash, &[u8]) -> bool + 'a>
	{
		let (inner, do_rebroadcast) = {
			use parking_lot::RwLockWriteGuard;

			let mut inner = self.inner.write();
			let now = Instant::now();
			let do_rebroadcast = if now >= inner.next_rebroadcast {
				inner.next_rebroadcast = now + REBROADCAST_AFTER;
				true
			} else {
				false
			};

			// downgrade to read-lock.
			(RwLockWriteGuard::downgrade(inner), do_rebroadcast)
		};

		Box::new(move |who, intent, topic, mut data| {
			if let MessageIntent::PeriodicRebroadcast = intent {
				return false; //rebroadcast not needed, I hope?
			}

			let peer = match inner.peers.peer(who) {
				None => return false,
				Some(x) => x,
			};

            if topic!= badger_topic::<BlockT>() //only one topic, i guess we may add epochs eventually
			{
				return false;
			}
			// if the topic is not something we're keeping at the moment,
			// do not send.
	
			match GossipMessage::<BlockT>::decode(&mut data) {
				None => false,
				Some(GossipMessage::BadgerData(_)) => {
					return  false
				}
				Some(GossipMessage::Greeting(_)) => true,
				Some(GossipMessage::RequestGreeting) => false,
			}
		})
	}

	fn message_expired<'a>(&'a self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'a> {
		let inner = self.inner.read();
		Box::new(move |topic, mut data| {
			// if the topic is not one of the ones that we are keeping at the moment,
			// it is expired.
			if topic!= badger_topic::<BlockT>() //only one topic, i guess we may add epochs eventually
			{
				return true;
			}

			match GossipMessage::<BlockT>::decode(&mut data) {
				None => true,
				Some(GossipMessage::Greeting(_))
					=> false,
				Some(_) => true,
			}
		})
	}
}



impl<B, S, H> Network<B> for Arc<NetworkService<B, S, H>> where
	B: BlockT,
	S: network::specialization::NetworkSpecialization<B>,
	H: network::ExHashT,
{
	type In = NetworkStream;

	fn messages_for(&self, topic: B::Hash) -> Self::In {
		let (tx, rx) = oneshot::channel();
		self.with_gossip(move |gossip, _| {
			let inner_rx = gossip.messages_for(HBBFT_ENGINE_ID, topic);
			let _ = tx.send(inner_rx);
		});
		NetworkStream { outer: rx, inner: None }
	}

	fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<B>>) {
		self.with_gossip(
			move |gossip, context| gossip.register_validator(context, HBBFT_ENGINE_ID, validator)
		)
	}

	fn gossip_message(&self, topic: B::Hash, data: Vec<u8>, force: bool) {
		let msg = ConsensusMessage {
			engine_id: HBBFT_ENGINE_ID,
			data,
		};

		self.with_gossip(
			move |gossip, ctx| gossip.multicast(ctx, topic, msg, force)
		)
	}

	fn register_gossip_message(&self, topic: B::Hash, data: Vec<u8>) {
		let msg = ConsensusMessage {
			engine_id: HBBFT_ENGINE_ID,
			data,
		};

		self.with_gossip(move |gossip, _| gossip.register_message(topic, msg))
	}

	fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>) {
		let msg = ConsensusMessage {
			engine_id: HBBFT_ENGINE_ID,
			data,
		};

		self.with_gossip(move |gossip, ctx| for who in &who {
			gossip.send_message(ctx, who, msg.clone())
		})
	}

	fn report(&self, who: network::PeerId, cost_benefit: i32) {
		self.report_peer(who, cost_benefit)
	}

	fn announce(&self, block: B::Hash) {
		self.announce_block(block)
	}
}

/// A stream used by NetworkBridge in its implementation of Network.
pub struct NetworkStream {
	inner: Option<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>,
	outer: oneshot::Receiver<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>
}
use std::{pin::Pin, task::Context, task::Poll};

impl Stream for NetworkStream {
	type Item = network_gossip::TopicNotification;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> 
	{
		if let Some(ref mut inner) = self.inner {
			return inner.poll_next(cx);
		}
		match self.outer.poll_next(cx) {
		     futures03::Poll::Ready(mut inner) => {
				let poll_result = inner.poll_next(cx);
				self.inner = Some(inner);
				poll_result
			},
			futures03::Poll::Pending => futures03::Poll::Pending,
		}
	}
}

/// Bridge between the underlying network service, gossiping consensus messages and Grandpa
pub(crate) struct NetworkBridge<B: BlockT, N: Network<B>,D: ConsensusProtocol, R: Rng> {
	service: N,
	node: Arc<BadgerGossipValidator<B,D,R>>,
}

impl<B: BlockT, N: Network<B>,D: ConsensusProtocol<NodeId=PeerId>, R: Rng> NetworkBridge<B, N, D,R> {
	/// Create a new NetworkBridge to the given NetworkService. Returns the service
	/// handle and a future that must be polled to completion to finish startup.
	/// If a voter set state is given it registers previous round votes with the
	/// gossip service.
	pub(crate) fn new(
		service: N,
		config: crate::Config,
		on_exit: impl Future<Item=(),Error=()> + Clone + Send + 'static,
	) -> (
		Self,
		impl futures03::Future<Item = (), Error = ()> + Send + 'static,
	) {

		let validator= BadgerGossipValidator::new(config, service.local_peer_id().clone());
		let validator_arc = Arc::new(validator);
		service.register_validator(validator_arc.clone());



	
		let bridge = NetworkBridge { service, validator_arc };

		let startup_work = futures03::future::lazy(move || {
			// lazily spawn these jobs onto their own tasks. the lazy future has access
			// to tokio globals, which aren't available outside.
		//	let mut executor = tokio_executor::DefaultExecutor::current();
		//	executor.spawn(Box::new(reporting_job.select(on_exit.clone()).then(|_| Ok(()))))
		//		.expect("failed to spawn grandpa reporting job task");
			Ok(())
		});

		(bridge, startup_work)
	}
pub  fn is_validator(&self) ->bool
{
	self.node.is_validator()
}


	/// Set up the global communication streams. blocks out transaction in. Maybe reverse of grandpa... 
	pub(crate) fn global_communication<D: ConsensusProtocol>(
		&self,
		is_voter: bool,
	) -> (
		impl Stream<Item = D::Output, Error = Error>,
		impl Sink<Item = TransactionSet, Error = Error>,
	)
	 {


		let service = self.service.clone();
		let topic = badger_topic::<BlockT>();
		let incoming = incoming_global( self.validator.clone());

		let outgoing = TransactionFeed::<B, N,D,R>::new(
			self.service.clone(),
			is_voter,
			self.validator.clone(),
		);

	
		(incoming, outgoing)
	}
}

fn incoming_global<B: BlockT, N: Network<B>,D: ConsensusProtocol, R: Rng>(
	gossip_validator: Arc<BadgerGossipValidator<B,D,R>>,
) -> impl Stream<Item = D::Output, Error = Error> 
{
	BadgerStream::new(gossip_validator.clone())
}

impl<B: BlockT, N: Network<B>> Clone for NetworkBridge<B, N> {
	fn clone(&self) -> Self {
		NetworkBridge {
			service: self.service.clone(),
			validator: Arc::clone(&self.validator),
			neighbor_sender: self.neighbor_sender.clone(),
		}
	}
}



pub struct BadgerStream<Block,D :ConsensusProtocol, R:Rng>  {
	validator:Arc<BadgerGossipValidator<Block,D,R>>,
}


impl<Block,D :ConsensusProtocol, R:Rng> BadgerStream<Block,D,R>
{
  fn new( gossip_validator: Arc<BadgerGossipValidator<Block,D,R>>) ->Self
  {
	  BadgerStream{
		validator:gossip_validator,
	  }

  }
}

impl<Block,D,R> Stream for BadgerStream <Block,D :ConsensusProtocol, R:Rng>{
	type Item = D::Output;
	
	fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>>
	{
		match self.validator.pop_output()
		{
			Some(data) => Poll::Ready(Some(data)),
			None => Poll::Pending
		}
		
	}
}


/// An output sink for commit messages.
struct TransactionFeed<Block: BlockT, N: Network<BlockT>,D :ConsensusProtocol, R:Rng> {
	network: N,
	is_voter: bool,
	gossip_validator: Arc<BadgerGossipValidator<Block,D,R>>,
}

impl<Block: BlockT, N: Network<BlockT>,D :ConsensusProtocol, R:Rng> TransactionFeed<Block, N,D,R> {
	/// Create a new commit output stream.
	pub(crate) fn new(
		network: N,
		is_voter: bool,
		gossip_validator: Arc<BadgerGossipValidator<Block,D,R>>,
	) -> Self {
		TransactionFeed {
			network,
			is_voter,
			gossip_validator,
		}
	}
}

impl<Block: BlockT, N: Network<Block>,D :ConsensusProtocol, R:Rng, Item: iter::IntoIterator> Sink<Item> for TransactionFeed<Block, N,D,R> 
where 
  Item::IntoIterator::Item: badger::Contribution 
{
	type Error = Error;
     
    fn poll_ready( self: Pin<&mut Self>,  cx: &mut Context) -> Poll<Result<(), Self::Error>>
	{
		 Poll::Ready(Ok(()))
	}

	fn start_send(self: Pin<&mut Self>, input:Item) ->  Result<(), Self::Error>
	{
		
       for tx in input.into_iter().enumerate()
	   {
        let locked=self.gossip_validator.inner.write();
        match locked.push_transaction(tx)
		{
			Ok(_) => {},
			Err(e) =>  return Err(e)
		}
	   }
	   Ok(())
	}
 fn poll_flush(
        self: Pin<&mut Self>, 
        cx: &mut Context
    ) -> Poll<Result<(), Self::Error>>
	{
		Ok(())
	}
    fn poll_close(
        self: Pin<&mut Self>, 
        cx: &mut Context
    ) -> Poll<Result<(), Self::Error>>
	{
	Ok(())
	}

}
