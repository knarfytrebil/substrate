use runtime_primitives::traits::Block as BlockT;
//use network::consensus_gossip::{self as network_gossip, MessageIntent, ValidatorContext};
use badger_primitives::{AuthorityId, AuthorityPair, AuthoritySignature};
use network::PeerId; //config::Roles,
use parity_codec::{Decode, Encode};

//use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};
//use log::{trace, debug, warn};
//use futures03::prelude::*;
//use futures03::channel::mpsc;
use log::info;
use rand::{rngs::OsRng, Rng};
use std::collections::BTreeMap;
use std::collections::HashMap;
use substrate_primitives::crypto::Pair; //RuntimeAppPublic
                                        //use std::time::{ Instant};//Duration

use runtime_primitives::traits::NumberFor;
use app_crypto::RuntimeAppPublic;

//const REBROADCAST_AFTER: Duration = Duration::from_secs(60 * 5);
//const CATCH_UP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
//const CATCH_UP_PROCESS_TIMEOUT: Duration = Duration::from_secs(15);
/// Maximum number of rounds we are behind a peer before issuing a
/// catch up request.
//const CATCH_UP_THRESHOLD: u64 = 2;

//const KEEP_RECENT_ROUNDS: usize = 3;

//const BADGER_TOPIC: &str = "itsasnake";
use sc_peerid_wrapper::PeerIdW;
/// HB gossip message type.
/// This is the root type that gets encoded and sent on the network.
#[derive(Debug, Encode, Decode)]
pub enum GossipMessage<Block: BlockT>
{
  /// Raw Badger data
  BadgerData(BadgeredMessage),
  /// Initial keygen data
  KeygenData(BadgeredMessage),
  /// Session notification for peer to badger pubkey mapping
  Session(SessionMessage),
  /// Justification data emitted when the new block needs to be confirmed
  JustificationData(BadgerJustification<Block>),

  /// Full block justification data to facilitate initial sync 
  SyncGossip(BadgerSyncGossip<Block>)
}

#[derive(Encode, Decode, Debug,Clone)]
pub struct BadgerJustification<Block: BlockT> 
{
	pub hash: Block::Hash,
  pub validator:AuthorityId,
  pub sgn: AuthoritySignature,
}




#[derive(Encode, Decode, Debug,Clone)]
pub struct BadgerAuthCommit 
{
  pub validator:AuthorityId,
  pub sgn: AuthoritySignature,
}


#[derive(Encode, Decode, Debug,Clone)]
pub struct BadgerFullJustification<Block: BlockT> 
{
	pub hash: Block::Hash,
  pub commits:Vec<BadgerAuthCommit>,
}


#[derive(Encode, Decode, Debug,Clone)]
pub struct BadgerSyncData<Block: BlockT> 
{
 pub  num:NumberFor::<Block>,
  pub justification:BadgerFullJustification<Block>
}

#[derive(Encode, Decode, Debug,Clone)]
pub struct BadgerSyncGossip<Block: BlockT> 
{
 pub data:BadgerSyncData<Block>,
 pub source:AuthorityId,
 pub sgn:AuthoritySignature
}

impl<Block: BlockT> BadgerSyncGossip<Block>
{
  pub fn verify(&self) -> bool
  {
    if ! self.data.justification.verify() {return false};
    badger_primitives::app::Public::verify(&self.source, &self.data.encode(), &self.sgn)

  }
  pub fn new(originator: &AuthorityPair, just:BadgerFullJustification<Block>,num: NumberFor::<Block> ) -> BadgerSyncGossip<Block>
  {
    let bsd=BadgerSyncData
    {
      num:num,
      justification:just,
    };
    let sig=originator.sign(&bsd.encode());
    BadgerSyncGossip {
      data: bsd,
      source: originator.public(),
      sgn: sig,
    }
  }
}



impl<Block: BlockT> BadgerFullJustification<Block>
{
  pub fn verify(&self) -> bool
  {
    let enc=self.hash.encode();
    for commit in self.commits.iter(){
     if ! badger_primitives::app::Public::verify(&commit.validator, &enc, &commit.sgn)
     {
       return false;
     }
    }
    return true;
   
  }
}



impl<Block: BlockT> BadgerJustification<Block>
{
  pub fn verify(&self) -> bool
  {
    badger_primitives::app::Public::verify(&self.validator, &self.hash.encode(), &self.sgn)
  }
}

impl<B:BlockT> GossipMessage<B>
{
  pub fn verify(&self) -> bool
  {
    match self
    {
      GossipMessage::BadgerData(data) => data.verify(),
      GossipMessage::KeygenData(data) => data.verify(),
      GossipMessage::Session(data) => data.verify(),
      GossipMessage::JustificationData(data) =>data.verify(),
      GossipMessage::SyncGossip(data) =>data.verify(),
    }
  }
}

#[derive(Debug, Encode, Decode)]
pub struct BadgeredMessage
{
  pub uid: u64,
  pub data: Vec<u8>,
  pub originator: AuthorityId,
  pub datasig: AuthoritySignature,
}

impl BadgeredMessage
{
  pub fn new(originator: AuthorityPair, data: &[u8]) -> BadgeredMessage
  {
    let uuid = OsRng::new().unwrap().gen::<u64>();
    BadgeredMessage {
      uid: uuid,
      data: data.iter().cloned().collect(),
      originator: originator.public(),
      datasig: originator.sign(data),
    }
  }
  pub fn verify(&self) -> bool
  {
    badger_primitives::app::Public::verify(&self.originator, &self.data, &self.datasig)
  }
}

#[derive(Debug, Encode, Decode)]
pub struct SessionData
{
  pub ses_id: u32,
  pub session_key: AuthorityId,
  pub peer_id: PeerIdW,
}

#[derive(Debug, Encode, Decode)]
pub struct SessionMessage
{
  pub ses: SessionData,
  pub sgn: AuthoritySignature,
}
impl SessionMessage
{
  pub fn verify(&self) -> bool
  {
    badger_primitives::app::Public::verify(&self.ses.session_key, &self.ses.encode(), &self.sgn)
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
  pub id: Option<AuthorityId>, //public key
  pub net_id: PeerId,
  pub state: PeerConsensusState,
}

impl PeerInfo
{
  fn new(n_id: PeerId) -> Self
  {
    PeerInfo {
      id: None,
      state: PeerConsensusState::Unknown,
      net_id: n_id,
    }
  }
  fn new_id(id: AuthorityId, n_id: PeerId) -> Self
  {
    PeerInfo {
      id: Some(id),
      net_id: n_id,
      state: PeerConsensusState::Unknown,
    }
  }
}

/// The peers we're connected to in gossip.
pub struct Peers
{
  pub inner: HashMap<PeerId, PeerInfo>,
  pub inverse: BTreeMap<AuthorityId, PeerId>,
}

impl Default for Peers
{
  fn default() -> Self
  {
    Peers::new()
  }
}

impl Peers
{
  pub fn new() -> Self
  {
    Peers {
      inner: HashMap::new(),
      inverse: BTreeMap::new(),
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
        let ti = PeerInfo::new(who.clone());
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
        let mut ti = PeerInfo::new(who.clone());
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
      .filter(|v| v.1.state == PeerConsensusState::Connected || v.1.state == PeerConsensusState::GreetingReceived)
      .map(|(k, _)| k.clone())
      .collect()
  }
  pub fn connected_badgerid_list(&self) -> Vec<AuthorityId>
  {
    self
      .inverse
      .iter()
      .filter(|(_k, v)| {
        let kk = self.inner.get(v);
        if kk.is_none()
        {
          return false;
        }

        kk.unwrap().state == PeerConsensusState::Connected || kk.unwrap().state == PeerConsensusState::GreetingReceived
      })
      .map(|(k, _)| k.clone())
      .collect()
  }
  pub fn badgerid_to_peerid(&self, id: &AuthorityId) -> Option<PeerId>
  {
    if let Some(pi) = self.inverse.get(id)
    {
      Some(self.inner.get(pi).unwrap().net_id.clone())
    }
    else
    {
      None
    }
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
    let _peer = match self.inner.get_mut(who)
    {
      None =>
      {
        self
          .inner
          .insert(who.clone(), PeerInfo::new_id(auth_id.clone(), who.clone()));
        self.inverse.insert(auth_id.clone(), who.clone());
        return;
      }
      Some(p) =>
      {
        if let Some(authority) = &p.id
        {
          if *authority != auth_id
          {
            self.inverse.remove(&authority);
          }
        }
        if !self.inverse.contains_key(&auth_id)
        {
          self.inverse.insert(auth_id.clone(), who.clone());
        }

        p.id = Some(auth_id);
      }
    };
  }

  pub fn peer_by_id<'a>(&'a self, who: &AuthorityId) -> Option<&'a PeerInfo>
  {
    match self.inverse.get(who)
    {
      None => None,
      Some(id) => self.inner.get(id),
    }
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

/*
#[derive(Debug, PartialEq, Clone)]
pub enum SAction
{
  // propagate to neigbors if unknown
  PropagateOnce,

  //Keep in slot and propagate occasionally
  RepropagateKeep,

  // Discard immediately
  Discard,

  //Queue locally and retry later, contains relevance/topic?
  QueueRetry(u64),
}
*/