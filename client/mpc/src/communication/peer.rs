use std::cmp::{min, Ordering};
use std::collections::{hash_map::DefaultHasher, BTreeSet, HashMap};
use std::convert::From;
use std::hash::{Hash, Hasher};
use std::iter::Iterator;
use std::str::FromStr;

use sc_network::{config::Roles, PeerId};

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum PeerState {
	AwaitingPeers,
	Generating,
	Complete,
	Canceled,
}

impl Default for PeerState {
	fn default() -> Self {
		PeerState::AwaitingPeers
	}
}

#[derive(Debug, Clone)]
pub struct PeerInfo {
	pub state: PeerState,
}

impl Default for PeerInfo {
	fn default() -> Self {
		Self {
			state: PeerState::default(),
		}
	}
}

#[derive(Debug)]
pub struct Peers {
	map: HashMap<PeerId, PeerInfo>,
	set: BTreeSet<String>,
}

impl Default for Peers {
	fn default() -> Self {
		Self {
			map: HashMap::default(),
			set: BTreeSet::default(),
		}
	}
}

impl Peers {
	pub fn add(&mut self, who: PeerId) {
		let base58_id = who.to_base58();
		self.map.insert(who, PeerInfo::default());
		self.set.insert(base58_id);
	}

	pub fn del(&mut self, who: &PeerId) {
		self.map.remove(who);
		self.set.remove(&who.to_base58());
	}

	pub fn len(&self) -> usize {
		self.map.len()
	}

	pub fn iter(&self) -> impl Iterator<Item = (&PeerId, &PeerInfo)> {
		self.map.iter()
	}

	pub fn contains_peer_id(&self, who: &PeerId) -> bool {
		self.map.contains_key(who)
	}

	pub fn keys(&self) -> impl Iterator<Item = &PeerId> {
		self.map.keys()
	}

	pub fn get_state(&self, who: &PeerId) -> Option<PeerState> {
		let peer = self.map.get(who);
		peer.map(|p| p.state)
	}

	pub fn set_state(&mut self, who: &PeerId, state: PeerState) {
		let peer = self.map.get_mut(who).expect("Peer not found!");
		peer.state = state;
	}

	pub fn get_hash(&self) -> u64 {
		let mut hasher = DefaultHasher::new();
		self.set.hash(&mut hasher);
		hasher.finish()
	}

	pub fn get_position(&self, who: &PeerId) -> Option<usize> {
		self.set.iter().position(|x| *x == who.to_base58())
	}

	pub fn get_peer_id_by_index(&self, index: usize) -> Option<PeerId> {
		let s = self.set.iter().nth(index);
		if let Some(s) = s {
			if let Ok(who) = PeerId::from_str(s) {
				return Some(who);
			}
		}
		None
	}
}
