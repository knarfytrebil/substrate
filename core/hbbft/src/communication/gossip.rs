//use runtime_primitives::traits::Block as BlockT;
//use network::consensus_gossip::{self as network_gossip, MessageIntent, ValidatorContext};
use badger_primitives::{AuthorityId, AuthoritySignature};
use network::PeerId; //config::Roles,
use parity_codec::{Decode, Encode};

//use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};
//use log::{trace, debug, warn};
//use futures03::prelude::*;
//use futures03::channel::mpsc;

use log::info;
use std::collections::HashMap;
//use std::time::{ Instant};//Duration

//const REBROADCAST_AFTER: Duration = Duration::from_secs(60 * 5);
//const CATCH_UP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
//const CATCH_UP_PROCESS_TIMEOUT: Duration = Duration::from_secs(15);
/// Maximum number of rounds we are behind a peer before issuing a
/// catch up request.
//const CATCH_UP_THRESHOLD: u64 = 2;

//const KEEP_RECENT_ROUNDS: usize = 3;

//const BADGER_TOPIC: &str = "itsasnake";
use crate::communication::PeerIdW;
/// HB gossip message type.
/// This is the root type that gets encoded and sent on the network.
#[derive(Debug, Encode, Decode)]
pub enum GossipMessage
{
  /// Greeting to register peer id. Probably should be replaced by Session aspects
  Greeting(GreetingMessage),
  /// Raw Badger data
  BadgerData(BadgeredMessage),

  KeygenData(BadgeredMessage),
  RequestGreeting,
}

#[derive(Debug, Encode, Decode)]
pub struct BadgeredMessage
{
  pub uid: u64,
  pub data: Vec<u8>,
  pub originator: PeerIdW,
}

#[derive(Debug, Encode, Decode)]
pub struct GreetingMessage
{
  pub my_pubshare: Option<AuthorityId>,
  /// the badger ID of the peer
  pub my_id: AuthorityId,
  /// Signature to verify id
  pub my_sig: AuthoritySignature,
}

impl From<GreetingMessage> for GossipMessage
{
  fn from(greet: GreetingMessage) -> Self
  {
    GossipMessage::Greeting(greet)
  }
}

#[derive(Debug, Encode, Decode, PartialEq, Eq)]
pub enum PeerConsensusState
{
  Unknown,
  Connected,
  GreetingReceived,
  Disconnected,
}
pub struct PeerInfo
{
  //view: View<N>,
  pub id: Option<AuthorityId>, //public key
  pub state: PeerConsensusState,
}

impl PeerInfo
{
  fn new() -> Self
  {
    PeerInfo {
      id: None,
      state: PeerConsensusState::Unknown,
    }
  }
  fn new_id(id: AuthorityId) -> Self
  {
    PeerInfo {
      id: Some(id),
      state: PeerConsensusState::Unknown,
    }
  }
}

/// The peers we're connected to in gossip.
pub struct Peers
{
  inner: HashMap<PeerId, PeerInfo>,
}

impl Default for Peers
{
  fn default() -> Self
  {
    Peers {
      inner: HashMap::new(),
    }
  }
}

impl Peers
{
  pub fn new() -> Self
  {
    Peers {
      inner: HashMap::new(),
    }
  }
  pub fn new_peer(&mut self, who: PeerId)
  {
    match self.inner.get(&who)
    {
      Some(_) =>
      {}
      None =>
      {
        let ti = PeerInfo::new();
        self.inner.insert(who, ti);
      }
    }
  }
  pub fn update_peer_state(&mut self, who: &PeerId, state: PeerConsensusState)
  {
    match self.inner.get_mut(&who)
    {
      Some(dat) =>
      {
        dat.state = state;
      }
      None =>
      {
        let mut ti = PeerInfo::new();
        ti.state = state;
        self.inner.insert(who.clone(), ti);
      }
    };
  }
  pub fn peer_list(&self) -> Vec<PeerId>
  {
    self.inner.iter().map(|(k, _)| k.clone()).collect()
  }
  pub fn connected_peer_list(&self) -> Vec<PeerId>
  {
    self
      .inner
      .iter()
      .filter(|v| {
        v.1.state == PeerConsensusState::Connected ||
          v.1.state == PeerConsensusState::GreetingReceived
      })
      .map(|(k, _)| k.clone())
      .collect()
  }

  pub fn peer_disconnected(&mut self, who: &PeerId)
  {
    match self.inner.get_mut(&who)
    {
      Some(dat) =>
      {
        dat.state = PeerConsensusState::Disconnected;
      }
      None =>
      {}
    };
    //	self.inner.remove(who);
    info!(
      "Peer disconnected {:?} ! now {:?} peers known about",
      who,
      self.inner.len()
    );
  }

  pub fn update_id(&mut self, who: &PeerId, auth_id: AuthorityId)
  {
    let peer = match self.inner.get_mut(who)
    {
      None =>
      {
        self.inner.insert(who.clone(), PeerInfo::new_id(auth_id));
        return;
      }
      Some(p) => p,
    };
    peer.id = Some(auth_id);
  }

  pub fn peer<'a>(&'a self, who: &PeerId) -> Option<&'a PeerInfo>
  {
    self.inner.get(who)
  }
}

#[derive(Debug, PartialEq)]
pub enum Action<H>
{
  // repropagate under given topic, to the given peers, applying cost/benefit to originator.
  Keep,
  // discard and process.
  ProcessAndDiscard,
  // discard
  Discard,
  Useless(H),
}
