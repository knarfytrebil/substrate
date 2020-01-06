use std::{
	//collections::VecDeque,
	marker::PhantomData,
	str::FromStr,
	time::{Duration, Instant},
};

use codec::{Decode, Encode};
//use log::{error, info, trace, warn};

use sc_network::{config::Roles, PeerId};
use sc_network_gossip::{ MessageIntent, ValidationResult, ValidatorContext};//GossipEngine
use sp_runtime::traits::Block as BlockT;

use super::{
	message::{ConfirmPeersMessage, KeyGenMessage, SigGenMessage},
	peer::{PeerInfo, PeerState, Peers},
};
use crate::NodeConfig;

const REBROADCAST_AFTER: Duration = Duration::from_secs(30);

pub type RequestId = u64;

#[derive(Debug, Clone, Encode, Decode, PartialEq)]
pub enum GossipMessage {
	ConfirmPeers(ConfirmPeersMessage, RequestId),
	KeyGen(KeyGenMessage, RequestId),
	SigGen(SigGenMessage, RequestId),
}

impl GossipMessage {
	pub fn get_req_id(&self) -> RequestId {
		match self {
			GossipMessage::ConfirmPeers(_, id) => *id,
			GossipMessage::KeyGen(_, id) => *id,
			GossipMessage::SigGen(_, id) => *id,
		}
	}
}

pub type MessageWithSender = (GossipMessage, Option<PeerId>);
pub type MessageWithReceiver = (GossipMessage, Option<PeerId>);

pub struct Inner {
	local_peer_id: PeerId,
	local_peer_info: PeerInfo,
	peers: Peers,
	config: NodeConfig,
	next_rebroadcast: Instant,
}

#[allow(dead_code)]
impl Inner {
	fn new(config: NodeConfig, local_peer_id: PeerId) -> Self {
		let mut peers = Peers::default();
		peers.add(local_peer_id.clone());

		Self {
			config,
			local_peer_id,
			local_peer_info: PeerInfo::default(),
			peers,
			next_rebroadcast: Instant::now() + REBROADCAST_AFTER,
		}
	}

	fn add_peer(&mut self, who: PeerId) {
		self.peers.add(who);
	}

	fn del_peer(&mut self, who: &PeerId) {
		self.peers.del(who);
	}

	pub fn get_players(&self) -> u16 {
		self.config.players
	}

	pub fn get_peers_len(&self) -> usize {
		self.peers.len()
	}

	pub fn get_other_peers(&self) -> Vec<PeerId> {
		let local_id = &self.local_peer_id;
		self.peers
			.keys()
			.filter(|&pid| pid != local_id)
			.map(|x| x.clone())
			.collect()
	}

	pub fn get_peers_hash(&self) -> u64 {
		self.peers.get_hash()
	}

	pub fn get_local_index(&self) -> usize {
		self.get_peer_index(&self.local_peer_id())
	}

	pub fn get_peer_index(&self, who: &PeerId) -> usize {
		self.peers.get_position(who).unwrap()
	}

	pub fn get_peer_id_by_index(&self, index: usize) -> Option<PeerId> {
		self.peers.get_peer_id_by_index(index)
	}

	pub fn local_peer_id(&self) -> PeerId {
		self.local_peer_id.clone()
	}

	pub fn local_string_peer_id(&self) -> String {
		self.local_peer_id.to_base58()
	}

	pub fn local_state(&self) -> PeerState {
		self.local_peer_info.state.clone()
	}

	pub fn is_local_awaiting_peers(&self) -> bool {
		self.local_peer_info.state == PeerState::AwaitingPeers
	}

	pub fn is_local_generating(&self) -> bool {
		self.local_peer_info.state == PeerState::Generating
	}

	pub fn is_local_complete(&self) -> bool {
		self.local_peer_info.state == PeerState::Complete
	}

	pub fn is_local_canceled(&self) -> bool {
		self.local_peer_info.state == PeerState::Canceled
	}

	pub fn set_local_awaiting_peers(&mut self) {
		self.set_local_state(PeerState::AwaitingPeers);
	}

	pub fn set_local_generating(&mut self) {
		self.set_local_state(PeerState::Generating);
	}

	pub fn set_local_complete(&mut self) {
		self.set_local_state(PeerState::Complete);
	}

	pub fn set_local_canceled(&mut self) {
		self.set_local_state(PeerState::Canceled);
	}

	pub fn set_peer_awaiting_peers(&mut self, who: &PeerId) {
		self.set_peer_state(who, PeerState::AwaitingPeers);
	}

	pub fn set_peer_generating(&mut self, who: &PeerId) {
		self.set_peer_state(who, PeerState::Generating);
	}

	pub fn set_peer_complete(&mut self, who: &PeerId) {
		self.set_peer_state(who, PeerState::Complete);
	}

	pub fn set_peer_canceled(&mut self, who: &PeerId) {
		self.set_peer_state(who, PeerState::Canceled);
	}

	pub fn set_local_state(&mut self, state: PeerState) {
		self.set_peer_state(&self.local_peer_id(), state.clone());
		self.local_peer_info.state = state;
	}

	pub fn set_peer_state(&mut self, who: &PeerId, state: PeerState) {
		self.peers.set_state(who, state);
	}

	pub fn get_peer_state(&self, who: &PeerId) -> Option<PeerState> {
		self.peers.get_state(who)
	}
}

pub struct GossipValidator<Block: BlockT> {
	pub inner: parking_lot::RwLock<Inner>,
	_phantom: PhantomData<Block>,
}

impl<Block: BlockT> GossipValidator<Block> {
	pub fn new(config: NodeConfig, local_peer_id: PeerId) -> Self {
		Self {
			inner: parking_lot::RwLock::new(Inner::new(config, local_peer_id)),
			_phantom: PhantomData,
		}
	}
}

impl<Block: BlockT> sc_network_gossip::Validator<Block> for GossipValidator<Block> {
	fn new_peer(&self, _context: &mut dyn ValidatorContext<Block>, who: &PeerId, roles: Roles) {
		if roles != Roles::AUTHORITY {
			return;
		}

		let mut inner = self.inner.write();
		inner.add_peer(who.clone());
		inner.set_local_awaiting_peers();
	}

	fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<Block>, who: &PeerId) {
		let mut inner = self.inner.write();
		inner.del_peer(who);

		let players = inner.config.players as usize;
		if inner.get_peers_len() < players {
			inner.set_local_canceled();
		}
	}

	fn validate(
		&self,
		_context: &mut dyn ValidatorContext<Block>,
		_who: &PeerId,
		mut data: &[u8],
	) -> ValidationResult<Block::Hash> {
		let gossip_msg = GossipMessage::decode(&mut data);
		if let Ok(gossip_msg) = gossip_msg {
			let req_id = gossip_msg.get_req_id();
			let topic = super::bytes_topic::<Block>(&req_id.to_le_bytes());
			return ValidationResult::ProcessAndKeep(topic);
		}
		ValidationResult::Discard
	}

	fn message_allowed<'a>(&'a self) -> Box<dyn FnMut(&PeerId, MessageIntent, &Block::Hash, &[u8]) -> bool + 'a> {
		// rebroadcasted message
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

			(RwLockWriteGuard::downgrade(inner), do_rebroadcast)
		};

		Box::new(move |who, intent, _topic, mut data| {
			println!("In `message_allowed` rebroadcast: {:?}", do_rebroadcast);

			if let MessageIntent::PeriodicRebroadcast = intent {
				return do_rebroadcast;
			}

			let players = inner.config.players as usize;
			if inner.peers.len() < players {
				return true;
			}

			if !inner.peers.contains_peer_id(&who) {
				return false;
			}

			let gossip_msg = GossipMessage::decode(&mut data);
			if let Ok(gossip_msg) = gossip_msg {
				let our_hash = inner.get_peers_hash();

				let is_awaiting_peers = inner.is_local_awaiting_peers();
				let is_generating = inner.is_local_generating();

				match gossip_msg {
					GossipMessage::ConfirmPeers(_, all_peers_hash) => {
						return is_awaiting_peers && our_hash == all_peers_hash;
					}
					GossipMessage::KeyGen(_, all_peers_hash) => {
						let is_valid = is_awaiting_peers || is_generating;
						return is_valid && our_hash == all_peers_hash;
					}
					_ => return false,
				}
			}
			false
		})
	}

	fn message_expired<'a>(&'a self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'a> {
		Box::new(move |_topic, mut data| {
			let inner = self.inner.read();
			let is_complete = inner.is_local_complete();
			let is_canceled = inner.is_local_canceled();

			if is_complete || is_canceled {
				return true;
			}

			let players = inner.config.players as usize;
			if inner.peers.len() < players {
				return false;
			}

			let gossip_msg = GossipMessage::decode(&mut data);
			if let Ok(gossip_msg) = gossip_msg {
				println!("In `message_expired` of {:?}", inner.get_local_index());
				let gmsg = gossip_msg.clone();
				match gmsg {
					GossipMessage::ConfirmPeers(cpm, _all_peers_hash) => match cpm {
						ConfirmPeersMessage::Confirming(from) => {
							println!("confirming from {:?}", from);
						}
						_ => {}
					},
					GossipMessage::KeyGen(kgm, _all_peers_hash) => match kgm {
						KeyGenMessage::CommitAndDecommit(from, _, _) => {
							println!("com decom from {:?}", from);
						}
						KeyGenMessage::VSS(from, _) => {
							println!("VSS from {:?}", from);
						}
						KeyGenMessage::SecretShare(from, _) => {
							println!("Secret share from {:?}", from);
						}
						KeyGenMessage::Proof(from, _) => {
							println!("proof from {:?}", from);
						}
					},
					_ => {}
				}
				let our_hash = inner.get_peers_hash();

				println!("exit `message_expired` of {:?}", inner.get_local_index());

				match gossip_msg {
					GossipMessage::ConfirmPeers(cpm, all_peers_hash) => {
						match cpm {
							ConfirmPeersMessage::Confirming(from_index) => {
								let sender_id = inner.get_peer_id_by_index(from_index as usize);
								if sender_id.is_none() {
									return true;
								}
							}
							ConfirmPeersMessage::Confirmed(sender_string_id) => {
								let sender = PeerId::from_str(&sender_string_id);
								if sender.is_err() {
									return true;
								}
								if !inner.peers.contains_peer_id(&sender.unwrap()) {
									return true;
								}
							}
						}
						let is_awaiting_peers = inner.is_local_awaiting_peers();

						return !is_awaiting_peers || is_canceled || our_hash != all_peers_hash;
					}
					GossipMessage::KeyGen(kgm, all_peers_hash) => {
						let from_index = kgm.get_index() as usize;
						let sender_id = inner.get_peer_id_by_index(from_index);

						return our_hash != all_peers_hash || sender_id.is_none();
					}
					GossipMessage::SigGen(_, _) => return false,
				}
			}
			true
		})
	}
}
