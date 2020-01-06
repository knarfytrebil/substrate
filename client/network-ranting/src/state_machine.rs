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

use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::sync::Arc;
use std::iter;
use std::time;
use log::{trace, debug};
use futures::channel::mpsc;
use lru::LruCache;
use libp2p::PeerId;
use codec::{Encode,Decode};
use sp_runtime::traits::{Block as BlockT, Hash, HashFor};
use sp_runtime::ConsensusEngineId;
pub use sc_network::message::generic::{Message, ConsensusMessage};
use sc_network::Context;
use sc_network::config::Roles;

use sc_peerid_wrapper::PeerIdW;

// FIXME: Add additional spam/DoS attack protection: https://github.com/paritytech/substrate/issues/1115
const KNOWN_MESSAGES_CACHE_SIZE: usize = 4096;
const MAX_NUM_SET: usize = 48;

const REBROADCAST_INTERVAL: time::Duration = time::Duration::from_secs(10);

mod rep {
	use sc_network::ReputationChange as Rep;
	/// Reputation change when a peer sends us a gossip message that we didn't know about.
	pub const GOSSIP_SUCCESS: Rep = Rep::new(1 << 4, "Successfull gossip");
	/// Reputation change when a peer sends us a gossip message that we already knew about.
	//pub const DUPLICATE_GOSSIP: Rep = Rep::new(-(1 << 2), "Duplicate gossip");
	pub const MALFORMED_GOSSIP: Rep = Rep::new(-(1 << 2), "Malformed gossip");
	/// Reputation change when a peer sends us a gossip message for an unknown engine, whatever that
	/// means.
	pub const UNKNOWN_GOSSIP: Rep = Rep::new(-(1 << 6), "Unknown gossup message engine id");
	pub fn punishment(val:i32) ->Rep
	{
		Rep::new(val, "Punishing invalid message")
	}
}

struct PeerConsensus<H> {
	known_messages:  LruCache<H, ()>,
	roles: Roles,
}

/// Topic stream message with sender.
#[derive(Debug, Eq, PartialEq)]
pub struct TopicNotification
{
  /// Message data.
  pub message: Vec<u8>,
  /// Sender if available.
  pub sender: Option<PeerId>,
}

#[derive(Clone)]
struct MessageEntry<B: BlockT>
{
  message_hash: B::Hash,
  cell: B::Hash,
  engine_id: ConsensusEngineId,
  message: RawMessage,
}

/// The reason for sending out the message.
#[derive(Eq, PartialEq, Copy, Clone)]
#[cfg_attr(test, derive(Debug))]
pub enum MessageIntent {
	/// Requested broadcast.
	Broadcast,
	/// Requested broadcast to all peers.
	ForcedBroadcast,
	/// Periodic rebroadcast of all messages to all peers.
	PeriodicRebroadcast,
}

#[derive(Eq, PartialEq, Copy, Clone)]
pub enum ValidationResult<B: BlockT>
{

  /// Keep in cache, periodically rebroadcasting
  Maintain(B::Hash),
  /// Discard message as done
  Discard,

  /// Discard message and report sending peer
  Punish(i32)
}




#[derive(Encode, Decode)]
pub enum RoutingInfo
{
	/// broadcast to all except
	BroadcastExclude(Vec<PeerIdW>),
	/// target a subset
	Targeted(Vec<PeerIdW>),
	/// Specific to a local node
	Specific
}

impl MessageIntent {
	fn broadcast() -> MessageIntent {
		MessageIntent::Broadcast
	}
}

#[derive(Encode, Decode)]
pub struct RoutedMessage
{
	pub route: RoutingInfo,
	pub msg: RawMessage,
}

/// Validation context. Allows reacting to incoming messages by sending out further messages.
pub trait ValidatorContext<B: BlockT>
{
  fn broadcast_except(&mut self, except: HashSet<PeerId>, message: Vec<u8>);
  fn send_to_set(&mut self,set:HashSet<PeerId>, message: Vec<u8>);
  fn send_single(&mut self,who:&PeerId, message: Vec<u8>);
  fn keep(&mut self,cell:B::Hash, message: Vec<u8>);
  fn get_kept(&self,cell:&B::Hash )->Option<Vec<u8>>;
  
}

pub struct NetworkContext<'g, 'p, B: BlockT>
{
  gossip: &'g mut ConsensusGossip<B>,
  protocol: &'p mut dyn Context<B>,
  engine_id: ConsensusEngineId,
}


impl<'g, 'p, B: BlockT> ValidatorContext<B> for NetworkContext<'g, 'p, B> {

    /// Broadcast to all except
	fn broadcast_except(&mut self, except: HashSet<PeerId>, message: Vec<u8>)
	{
		self.gossip.broadcast_except(self.protocol, self.engine_id,except , message, false,false)
	}
	fn send_to_set(&mut self,set:HashSet<PeerId>, message: Vec<u8>)
	{
		self.gossip.send_to_set(self.protocol,self.engine_id, set, message, true,false,);
	}
	fn send_single(&mut self,who:&PeerId, message: Vec<u8>)
	{
      self.gossip.send_single(self.protocol, self.engine_id,who, message)
	}
	fn keep(&mut self,cell:B::Hash, message: Vec<u8>)
	{
    self.gossip.register_message(cell, self.engine_id.clone(),message);
	}
	fn get_kept(&self,cell:&B::Hash )->Option<Vec<u8>>
	{
		self.gossip.get_kept(&cell)
	
	}
  
}



/// Validates consensus messages.
pub trait Validator<B: BlockT>: Send + Sync
{
  /// New peer is connected.
  fn new_peer(&self, _context: &mut dyn ValidatorContext<B>, _who: &PeerId, _roles: Roles) {}

  /// New connection is dropped.
  fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<B>, _who: &PeerId) {}

  /// Validate consensus message.
  fn validate(
    &self,
    context: &mut dyn ValidatorContext<B>,
    sender: &PeerId,
    data: &[u8],
  ) -> ValidationResult<B>;

  /// Produce a closure for validating messages on a given topic.
  fn message_expired<'a>(&'a self) -> Box<dyn FnMut(B::Hash, &[u8]) -> bool + 'a>
  {
    Box::new(move |_topic, _data| false)
  }


}
pub type RawMessage=Vec<u8>;

/// Consensus network protocol handler. Manages statements and candidate requests.
pub struct ConsensusGossip<B: BlockT>
{
  peers: HashMap<PeerId, PeerConsensus<B::Hash>>,
  live_message_sinks:
    HashMap<ConsensusEngineId, Vec<mpsc::UnboundedSender<RawMessage>>>,
  messages: HashMap<B::Hash,MessageEntry<B>>,
  known_messages: LruCache<B::Hash, ()>,
  validators: HashMap<ConsensusEngineId, Arc<dyn Validator<B>>>,
  next_broadcast: time::Instant,
  self_id: PeerId,
}


impl<B: BlockT> ConsensusGossip<B> {
	/// Create a new instance.
	pub fn new(selfid:PeerId) -> Self {
		ConsensusGossip {
			peers: HashMap::new(),
			live_message_sinks: HashMap::new(),
			messages: Default::default(),
			known_messages: LruCache::new(KNOWN_MESSAGES_CACHE_SIZE),
			validators: Default::default(),
			next_broadcast: time::Instant::now() + REBROADCAST_INTERVAL,
			self_id:selfid,
		}
	}

	/// Closes all notification streams.
	pub fn abort(&mut self) {
		self.live_message_sinks.clear();
	}

	/// Register message validator for a message type.
	pub fn register_validator(
		&mut self,
		protocol: &mut dyn Context<B>,
		engine_id: ConsensusEngineId,
		validator: Arc<dyn Validator<B>>
	) {
		self.register_validator_internal(engine_id, validator.clone());
		let peers: Vec<_> = self.peers.iter().map(|(id, peer)| (id.clone(), peer.roles)).collect();
		for (id, roles) in peers {
			let mut context = NetworkContext { gossip: self, protocol, engine_id: engine_id.clone() };
			validator.new_peer(&mut context, &id, roles);
		}
	}

	fn register_validator_internal(&mut self, engine_id: ConsensusEngineId, validator: Arc<dyn Validator<B>>) {
		self.validators.insert(engine_id, validator.clone());
	}

	/// Handle new connected peer.
	pub fn new_peer(&mut self, protocol: &mut dyn Context<B>, who: PeerId, roles: Roles) {
		// light nodes are not valid targets for consensus gossip messages
		if !roles.is_full() {
			return;
		}

		trace!(target:"gossip", "Registering {:?} {}", roles, who);
		self.peers.insert(who.clone(), PeerConsensus {
			known_messages: LruCache::new(KNOWN_MESSAGES_CACHE_SIZE),
			roles,
		});
		for (engine_id, v) in self.validators.clone() {
			let mut context = NetworkContext { gossip: self, protocol, engine_id: engine_id.clone() };
			v.new_peer(&mut context, &who, roles);
		}
	}

	fn register_message_hashed(
		&mut self,
		message_hash: B::Hash,
		cell: B::Hash,
		engine_id:ConsensusEngineId,
		message: RawMessage,
		
	) {
		self.messages.insert(cell.clone(),MessageEntry {
			message_hash,
			cell,
			engine_id,
			message, });
		self.known_messages.put(message_hash.clone(), ());
	}
	pub fn get_kept(&self, cell:&B::Hash) ->Option<RawMessage>
	{ 
		match self.messages.get(cell)
		{
			Some(dat) =>Some(dat.message.clone()),
			None => None
		}
	
	}
	/// Registers a message without propagating it to any peers. The message
	/// becomes available to new peers or when the service is asked to gossip
	/// the message's topic. No validation is performed on the message, if the
	/// message is already expired it should be dropped on the next garbage
	/// collection.
	pub fn register_message(
		&mut self,
		topic: B::Hash,
		engine_id:ConsensusEngineId,
		message: RawMessage,
	) {
		let message_hash = HashFor::<B>::hash(&message[..]);
		self.register_message_hashed(message_hash, topic,engine_id, message);
	}

	/// Call when a peer has been disconnected to stop tracking gossip status.
	pub fn peer_disconnected(&mut self, protocol: &mut dyn Context<B>, who: PeerId) {
		for (engine_id, v) in self.validators.clone() {
			let mut context = NetworkContext { gossip: self, protocol, engine_id: engine_id.clone() };
			v.peer_disconnected(&mut context, &who);
		}
	}

	/// Perform periodic maintenance
	pub fn tick(&mut self, protocol: &mut dyn Context<B>) {
		self.collect_garbage();
		if time::Instant::now() >= self.next_broadcast {
			self.rebroadcast(protocol);
			self.next_broadcast = time::Instant::now() + REBROADCAST_INTERVAL;
		}
	}

	/// Rebroadcast all messages to all peers.
	fn rebroadcast(&mut self, protocol: &mut dyn Context<B>) {

		let wtf:Vec<_>=self.messages.iter().map( |(_,m)| (*m).clone()).collect();

		for m in wtf.into_iter()
		{
			self.broadcast_except(protocol, m.engine_id, HashSet::new(), m.message,true,true);
		}	

	}



	/// Prune old or no longer relevant consensus messages. Provide a predicate
	/// for pruning, which returns `false` when the items with a given topic should be pruned.
	pub fn collect_garbage(&mut self) {
		self.live_message_sinks.retain(|_, sinks| {
			sinks.retain(|sink| !sink.is_closed());
			!sinks.is_empty()
		});

		let known_messages = &mut self.known_messages;
		let before = self.messages.len();
		let validators = &self.validators;

		let mut check_fns = HashMap::new();
		let mut message_expired = move |h:B::Hash,entry: &MessageEntry<B>| {
			let engine_id = entry.engine_id;
			let check_fn = match check_fns.entry(engine_id) {
				Entry::Occupied(entry) => entry.into_mut(),
				Entry::Vacant(vacant) => match validators.get(&engine_id) {
					None => return true, // treat all messages with no validator as expired
					Some(validator) => vacant.insert(validator.message_expired()),
				}
			};

			(check_fn)(h, &entry.message)
		};

		self.messages.retain(|h,entry| !message_expired(h.clone(),entry));

		trace!(target: "gossip", "Cleaned up {} stale messages, {} left ({} known)",
			before - self.messages.len(),
			self.messages.len(),
			known_messages.len(),
		);
	}

	/// Get data of valid, incoming messages for a topic (but might have expired meanwhile)
	pub fn messages_for(&mut self, engine_id: ConsensusEngineId)
		-> mpsc::UnboundedReceiver<RawMessage>
	{
		let (tx, rx) = mpsc::unbounded();
		for (_, entry) in self.messages.iter_mut()
			.filter(|(_,e)|  e.engine_id == engine_id)
		{
			tx.unbounded_send(
					 entry.message.clone(),
				)
				.expect("receiver known to be live; qed");
		}

		self.live_message_sinks.entry(engine_id).or_default().push(tx);

		rx
	}
	pub fn broadcast_except(&mut self,protocol: &mut dyn Context<B>,engine_id: ConsensusEngineId,mut  except: HashSet<PeerId>, message: Vec<u8>,do_expand:bool,send_to_known:bool)
	{

		let message_hash = HashFor::<B>::hash(&message[..]);

		let mut targets=Vec::new();
		for (id,  peer) in self.peers.iter_mut()
		{
			if !except.contains(&id) &&  *id!=self.self_id
		  {
		   if send_to_known || !peer.known_messages.contains(&message_hash)
		    {
			targets.push(id.clone());
			peer.known_messages.put(message_hash.clone(),());
		    }
		  }
		}
		if do_expand
		{
			if except.len()<MAX_NUM_SET
			{
				except.insert(self.self_id.clone());
			}
			for peer in targets.iter()
			{   
				if except.len()>=MAX_NUM_SET
				{
					break;
				}
				except.insert(peer.clone());
			}
   
		 }
		 let outgoing=RoutedMessage
		 {
			 route: RoutingInfo::BroadcastExclude
			 (
				except.into_iter().map(|x| PeerIdW{0:x}).collect()
			 ),
			 msg: message,
		 }.encode();
		 let compound=ConsensusMessage {
			engine_id: engine_id.clone(),
			data:outgoing.clone(),
		};
	   for peer in targets.into_iter()
	   {
		protocol.send_consensus(peer.clone(), vec![compound.clone()]);  
	   }
	}
	pub fn send_single(&mut self,protocol: &mut dyn Context<B>,engine_id: ConsensusEngineId,peer:&PeerId, message: Vec<u8>,)
	{
		let outgoing= RoutedMessage
		{
			route: RoutingInfo::Specific,
			msg: message,
		}.encode();
		let compound=ConsensusMessage {
			engine_id: engine_id.clone(),
			data:outgoing,
		};
		protocol.send_consensus(peer.clone(), vec![compound]);  
	}

	pub fn send_to_set(&mut self,protocol: &mut dyn Context<B>,engine_id: ConsensusEngineId,mut  set: HashSet<PeerId>, message: Vec<u8>,do_constrict:bool,send_to_known:bool)
	{
		let message_hash = HashFor::<B>::hash(&message[..]);

		let mut targets=Vec::new();
		for (id,  peer) in self.peers.iter_mut()
		{
			if set.contains(&id) && *id!=self.self_id
		  {
		   if send_to_known || !peer.known_messages.contains(&message_hash)
		    {
			targets.push(id);
			peer.known_messages.put(message_hash.clone(),());
		    }
		  }
		}
		if do_constrict
		{
			set.remove(&self.self_id);
			for peer in targets.iter()
			{   
			set.remove(peer);
			}
		 }

		 let outgoing;
		 
		 if set.len()==0
		 {
			 outgoing= RoutedMessage
			 {
				 route: RoutingInfo::Specific,
				 msg: message,
			 }.encode();
		 }
		 else
		 {
			 outgoing= RoutedMessage
			 {
				 route: RoutingInfo::Targeted(set.into_iter().map(|x| PeerIdW{0:x} ).collect()),
				 msg: message,
			 }.encode();
		 }
		 let compound=ConsensusMessage {
			engine_id: engine_id.clone(),
			data:outgoing,
		};
	   for peer in targets.into_iter()
	   {
		protocol.send_consensus(peer.clone(), vec![compound.clone()]);  
	   }
	}
	/// Handle an incoming ConsensusMessage for topic by who via protocol. Discard message if topic
	/// already known, the message is old, its source peers isn't a registered peer or the connection
	/// to them is broken. Return `Some(topic, message)` if it was added to the internal queue, `None`
	/// in all other cases.
	pub fn on_incoming(
		&mut self,
		protocol: &mut dyn Context<B>,
		who: PeerId,
		messages: Vec<ConsensusMessage>,
	) {
		trace!(target:"gossip", "Received {} messages from peer {}", messages.len(), who);
		for message in messages {
			let pre_message_hash = HashFor::<B>::hash(&message.data);
			if self.known_messages.contains(&pre_message_hash) {
				trace!(target:"gossip", "Ignored already known pre-message from {}", who);
				continue;
			}

			let engine_id = message.engine_id;
			if !self.validators.contains_key(&engine_id)
			{
				trace!(target:"gossip", "Unregistered engine {:?} from {}", &engine_id,who);
				continue;
			}
			// deroute message...
			let rmsg:RoutedMessage= match Decode::decode(&mut &message.data[..])
			{
			  Ok(dat) =>{dat},
			  Err(e) =>
			  {
			trace!(target:"gossip", "Ignored malformed message from {}", who);
			protocol.report_peer(who.clone(), rep::MALFORMED_GOSSIP); 
			continue;
			
			  }
			};

			let message_hash = HashFor::<B>::hash(&rmsg.msg[..]);

			if self.known_messages.contains(&message_hash) {
				trace!(target:"gossip", "Ignored already known message from {}", who);
				//protocol.report_peer(who.clone(), rep::DUPLICATE_GOSSIP); - duplicates are expected
				continue;
			}



			let msgdata=match &rmsg.route
			{
            // broadcast to all except
			RoutingInfo::BroadcastExclude(ref excluded) =>
			{
			 if excluded.iter().find(|x| x.0==self.self_id ).is_some()
			 {
				 //just propagate...
				 self.broadcast_except(protocol,engine_id.clone(),excluded.iter().map(|x| x.0.clone()).collect(),rmsg.msg.clone(), true,false);	
                 continue;
			 }
			 else
			 {
               rmsg.msg
			 }
			}
	        // target a subset
			RoutingInfo::Targeted(ref set) => 
			{
				if set.iter().find(|x| x.0==self.self_id ).is_none()
				{
					self.send_to_set(protocol,engine_id.clone(),set.iter().map(|x| x.0.clone()).collect(),rmsg.msg.clone(),true,false);   
					continue;
				}
				else
				{
				  rmsg.msg
				}
			},
	        // Specific to a local node
	        RoutingInfo::Specific =>  { rmsg.msg},
			};

			// validate the message
			let validation = self.validators.get(&engine_id)
				.cloned()
				.map(|v| {
					let mut context = NetworkContext { gossip: self, protocol, engine_id };
					v.validate(&mut context, &who, &msgdata)
				});

			let validation_result = match validation {

				Some(ValidationResult::Maintain(hash)) => Some(hash),
				Some(ValidationResult::Discard) => None,
				Some(ValidationResult::Punish(num)) => 
				{
					trace!(target:"gossip", "Punishing message for {:?} from {}", &num, who);
					protocol.report_peer(who.clone(), rep::punishment(num));
					continue;
				},
				None => {
					trace!(target:"gossip", "Unknown message engine id {:?} from {}", engine_id, who);
					protocol.report_peer(who.clone(), rep::UNKNOWN_GOSSIP);
					protocol.disconnect_peer(who.clone());
					continue;
				}
			};
			match rmsg.route
			{
 // broadcast to all except
 RoutingInfo::BroadcastExclude(excluded) =>
 {
  	  self.broadcast_except(protocol,engine_id.clone(),excluded.iter().map(|x| x.0.clone()).collect(),msgdata.clone(), true,false);	
 }
 // target a subset
 RoutingInfo::Targeted(set) => 
 {
		 self.send_to_set(protocol,engine_id.clone(),set.iter().map(|x| x.0.clone()).collect(),msgdata.clone(),true,false);   
 },
 // Specific to a local node
 RoutingInfo::Specific =>  { },
			}
			if let Entry::Occupied(mut entry) = self.live_message_sinks.entry(engine_id) {
				debug!(target: "gossip", "Pushing consensus message to sinks.");
				entry.get_mut().retain(|sink| {
					if let Err(e) = sink.unbounded_send(msgdata.clone()) {
						trace!(target: "gossip", "Error broadcasting message notification: {:?}", e);
					}
					!sink.is_closed()
				});
				if entry.get().is_empty() {
					entry.remove_entry();
				}
			}
         	protocol.report_peer(who.clone(), rep::GOSSIP_SUCCESS);
			if let Some(cell) = validation_result {
				if let Some(ref mut peer) = self.peers.get_mut(&who) {
					peer.known_messages.put(message_hash,());
				} else {
					trace!(target:"gossip", "Ignored statement from unregistered peer {}", who);
				}
				self.register_message_hashed(message_hash, cell, engine_id,message.data,);
			} else {
				trace!(target:"gossip", "Handled valid terminating message from peer {}", who);
			}
		}
	}

	
	/// Send addressed message to a peer. The message is not kept or multicast
	/// later on.
	pub fn send_message(
		&mut self,
		protocol: &mut dyn Context<B>,
		who: &PeerId,
		engine_id:ConsensusEngineId,
		message: RawMessage,
	) {
		let _peer = match self.peers.get_mut(who) {
			None => return,
			Some(peer) => peer,
		};


		trace!(target: "gossip", "Sending direct to {}: {:?}", who, message);
		self.send_single(protocol,engine_id,who,message);
	}
}

/// A gossip message validator that discards all messages.
pub struct DiscardAll;

impl<B: BlockT> Validator<B> for DiscardAll
{
  fn validate(
    &self,
    _context: &mut dyn ValidatorContext<B>,
    _sender: &PeerId,
    _data: &[u8],
  ) -> ValidationResult<B>
  {
    ValidationResult::Discard
  }

  fn message_expired<'a>(&'a self) -> Box<dyn FnMut(B::Hash, &[u8]) -> bool + 'a>
  {
    Box::new(move |_topic, _data| true)
  }

}
