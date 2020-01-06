use badger::dynamic_honey_badger::DynamicHoneyBadger;
use badger::queueing_honey_badger::QueueingHoneyBadger;
use badger::sender_queue::{Message as BMessage, SenderQueue};
use badger::sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, PubKeyMap, SyncKeyGen};//AckFault
use keystore::KeyStorePtr;
use runtime_primitives::generic::BlockId;
use sc_api::{Backend,AuxStore};
//use runtime_primitives::app_crypto::RuntimeAppPublic;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
//use std::convert::TryInto;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use gossip::BadgerSyncGossip;
use gossip::BadgerJustification;
use runtime_primitives::Justification;
use consensus_common::{self, BlockImportParams, BlockOrigin, ForkChoiceStrategy, SelectChain, Error as ConsensusError,};
use runtime_primitives::{
	generic::{self,DigestItem ,OpaqueDigestItemId},
};
use sc_network_ranting::RawMessage;
use badger::dynamic_honey_badger::ChangeState;
use badger_primitives::BadgerPreRuntime;
use runtime_primitives::traits::{NumberFor,One};
use consensus_common::evaluation;
use sc_network_ranting::{ Network as RantingNetwork};
use client::{
	 Client, well_known_cache_keys //
};
use sc_network_ranting::ValidationResult;
use badger_primitives::ConsensusLog;
use gossip::BadgerSyncData;
use gossip::{BadgerFullJustification,BadgerAuthCommit};
use substrate_primitives::{Blake2Hasher,};
use hash_db::Hasher;
use badger_primitives::AuthoritySignature;
//use badger::dynamic_honey_badger::KeyGenMessage::Ack;
use crate::aux_store::BadgerPersistentData;
use badger::crypto::{PublicKey, PublicKeySet, SecretKey, SecretKeyShare}; //PublicKeyShare, Signature
use badger::{dynamic_honey_badger::Change, ConsensusProtocol, CpStep, NetworkInfo, Target};
use futures03::channel::{mpsc, oneshot};
use futures03::prelude::*;
use futures03::{task::Context, task::Poll};
//use hex_fmt::HexFmt;
use badger::honey_badger::EncryptionSchedule;
use log::{debug, info, trace, warn,error};
use parity_codec::{Decode, Encode};
use parking_lot::RwLock;
use rand::{rngs::OsRng, Rng};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
//
//};//
pub use badger_primitives::HBBFT_ENGINE_ID;
use badger_primitives::{AuthorityId, AuthorityList, AuthorityPair};
use gossip::{BadgeredMessage, GossipMessage, Peers,  SessionData, SessionMessage};
use network::config::Roles;
use sc_network_ranting::ValidatorContext;
use sc_network_ranting::RantingEngine;
use network::PeerId;
use network::{ NetworkService,ReputationChange};
use network::message::generic::{ConsensusMessage, Message};

//use runtime_primitives::app_crypto::hbbft_thresh::Public as bPublic;
use runtime_primitives::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use substrate_primitives::crypto::Pair;
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};

pub mod gossip;

use crate::Error;

pub const MAX_DELAYED_JUSTIFICATIONS:u64=10 ;
//use badger_primitives::NodeId;
use sc_peerid_wrapper::PeerIdW;

//use badger::{SourcedMessage as BSM,  TargetedMessage};
pub type NodeId = PeerIdW; //session index? -> might be difficult. but it looks like nodeId for observer does not matter?  but it does for restarts

pub enum ExtractedLogs<Block:BlockT>
{
 ValidatorVote(Vec<AuthorityId>),
 NewSessionMessage(GossipMessage<Block>),
}

pub enum BatchProcResult<Block:BlockT>
{
  /// Need to emit justification for additional block
  EmitJustification(Block::Hash,AuthorityList,Vec<ExtractedLogs<Block>>),
  /// Processed, nothing happened
  Nothing,
  /// Completed justification, no additional block emitted
  Completed(Vec<ExtractedLogs<Block>>)
}

pub enum BlockPushResult
{
  InvalidData,
  Pushed,
  BlockFull,
  BlockError

}



pub trait BlockPusherMaker<B:BlockT> :Send+Sync
{
  fn process_all(&mut self,is: BlockId<B>,digest:generic::Digest<B::Hash>,locked: &mut VecDeque<BadgerTransaction>,batched: impl Iterator<Item=BadgerTransaction>) ->Result<B,()>;
  //fn make_pusher<'a>(&self,is: BlockId<B> , digest: generic::Digest<B::Hash> )->Result<Self::Pusher,()>;
  fn best_chain(&self)->Result< <B as BlockT>::Header,()>;
  fn import_block(&self,import_block: BlockImportParams<B>) ->Result<(),()>;
}




pub trait BatchProcessor<Block:BlockT>
{
  fn process_batch(&self,batch:<QHB as ConsensusProtocol>::Output);
}


pub fn badger_topic<B: BlockT>() -> B::Hash
{
  <<B::Header as HeaderT>::Hashing as HashT>::hash(format!("badger-mushroom").as_bytes())
}

pub fn badger_session<B: BlockT>() -> B::Hash
{
  <<B::Header as HeaderT>::Hashing as HashT>::hash(format!("badger-mushroom-session-data").as_bytes())
}

pub fn badger_sync<B: BlockT>() -> B::Hash
{
  <<B::Header as HeaderT>::Hashing as HashT>::hash(format!("badger-mushroom-sync-data").as_bytes())
}
pub fn badger_justification<B: BlockT>(bh: B::Hash) -> B::Hash
{
 // <<B::Header as HeaderT>::Hashing as HashT>::hash(format!("badger-mushroom-block_justification").as_bytes())
 bh
}


#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum LocalTarget<B:BlockT>
{
  /// The message must be sent to the node with the given ID.
  Nodes(BTreeSet<NodeId>),
  /// The message must be sent to all remote nodes except the passed nodes.
  /// Useful for sending messages to observer nodes that aren't
  /// present in a node's `all_ids()` list.
  AllExcept(BTreeSet<NodeId>),
  Keep(B::Hash),
}

impl<B:BlockT> From<Target<NodeId>> for LocalTarget<B>
{
  fn from(t: Target<NodeId>) -> Self
  {
    match t
    {
      // Target::All => LocalTarget::All,
      Target::Nodes(n) => LocalTarget::Nodes(n),
      Target::AllExcept(set) => LocalTarget::AllExcept(set),
    }
  }
}

impl<B:BlockT> Into<Target<NodeId>> for LocalTarget<B>
{
  fn into(self) -> Target<NodeId>
  {
    match self
    {
      // LocalTarget::All => Target::All,
      LocalTarget::Nodes(n) => Target::Nodes(n),
      LocalTarget::AllExcept(set) => Target::AllExcept(set),
      LocalTarget::Keep(_) => Target::AllExcept(BTreeSet::new()),
    }
  }
}

#[derive(Eq, PartialEq, Debug, Encode, Decode)]
pub struct SourcedMessage<D: ConsensusProtocol,B:BlockT>
where
  D::NodeId: Encode + Decode + Clone,
{
  sender_id: D::NodeId,
  target: LocalTarget<B>,
  message: Vec<u8>,
}



pub type BadgerTransaction = Vec<u8>;
pub type QHB = SenderQueue<QueueingHoneyBadger<BadgerTransaction, NodeId, Vec<BadgerTransaction>>>;
pub type BatchType=<QHB as ConsensusProtocol>::Output;

pub struct BadgerNode<B: BlockT, D>
where
  D: ConsensusProtocol<NodeId = NodeId>, //specialize to avoid some of the confusion
  D::Message: Serialize + DeserializeOwned,
{
  //id: PeerId,
  node_id: NodeId,
  algo: D,
  main_rng: OsRng,

  //peers: Peers,
  //authorities: Vec<AuthorityId>,
  //config: crate::Config,
  //next_rebroadcast: Instant,
  /// Incoming messages from other nodes that this node has not yet handled.
  // in_queue: VecDeque<SourcedMessage<D>>,
  /// Outgoing messages to other nodes.
  pub out_queue: VecDeque<SourcedMessage<D,B>>,
  /// The values this node has output so far, with timestamps.
  pub outputs: VecDeque<D::Output>,
  _block: PhantomData<B>,
}

pub enum KeyGenStep
{
  Init,
  PartGenerated,
  PartSent,
  ProcessingAcks,
  Done,
}

pub struct KeyGenState
{
  pub is_observer: bool,
  pub keygen: SyncKeyGen<NodeId>,
  pub part: Option<Part>,
  pub threshold: usize,
  pub expected_parts: VecDeque<NodeId>,
  pub expected_acks: VecDeque<(NodeId, VecDeque<NodeId>)>,
  pub buffered_messages: Vec<(NodeId, SyncKeyGenMessage)>,
  pub is_done: bool,
  //pub ack: Option<(NodeId,AckOutcome)>,
  //pub step:KeyGenStep,
  //pub keyset:PublicKeySet,
  //pub secret_share: Option<SecretKeyShare>,
}

impl KeyGenState
{
  pub fn clean_queues(&mut self)
  {
    if !self.expected_acks.is_empty()
    {
      let rf = &mut self.expected_acks[0];
      // let mut exp_src=&mut rf.0;
      let mut eacks = &mut rf.1;
      while eacks.is_empty()
      {
        self.expected_acks.pop_front();
        if self.expected_acks.is_empty()
        {
          self.is_done = true;
          return;
        }
        //exp_src=&mut self.expected_acks[0].0;
        eacks = &mut self.expected_acks[0].1;
      }
    }
  }
  pub fn is_next_sender(&self, sender: &NodeId) -> bool
  {
    if !self.expected_parts.is_empty()
    {
      return self.expected_parts[0] == *sender;
    }
    if !self.expected_acks.is_empty()
    {
      let (exp_src, _) = &self.expected_acks[0];
      return *exp_src == *sender;
    }
    return false;
  }
  pub fn is_next_message(&self, sender: &NodeId, msg: &SyncKeyGenMessage) -> bool
  {
    if !self.is_next_sender(sender)
    {
      return false;
    }
    match msg
    {
      SyncKeyGenMessage::Part(_) =>
      {
        if !self.expected_parts.is_empty()
        {
          return self.expected_parts[0] == *sender;
        }
        else
        {
          return false;
        }
      }
      SyncKeyGenMessage::Ack(src, _) =>
      {
        if !self.expected_parts.is_empty()
        {
          return false;
        }
        if !self.expected_acks.is_empty()
        {
          return self.expected_acks[0].0 == *sender &&
            !self.expected_acks[0].1.is_empty() &&
            self.expected_acks[0].1[0] == *src;
        }
        else
        {
          return false;
        }
      }
    }
  }
  pub fn process_part(&mut self, sender: &NodeId, part: Part) -> Vec<SyncKeyGenMessage>
  {
    let mut rng = rand::rngs::OsRng::new().expect("Could not open OS random number generator.");
    let outcome = match self.keygen.handle_part(sender, part, &mut rng)
    {
      Ok(outcome) => outcome,
      Err(e) =>
      {
        warn!("Failed processing part from {:?} {:?}", sender, e);
        return Vec::new();
      }
    };
    let mut ret: Vec<SyncKeyGenMessage> = Vec::new();
    match outcome
    {
      PartOutcome::Valid(Some(ack)) =>
      {
        self.expected_parts.pop_front();
        ret.push(SyncKeyGenMessage::Ack(sender.clone(), ack));
      }
      PartOutcome::Invalid(fault) =>
      {
        warn!("Faulty Part from {:?} :{:?}", sender, fault);
      }
      PartOutcome::Valid(None) =>
      {
        info!("We might be an observer");
        self.expected_parts.pop_front();
      }
    }
    ret
  }

  pub fn process_ack(&mut self, sender: &NodeId, _src: &NodeId, ack: Ack)
  {
    //let mut rng = rand::rngs::OsRng::new().expect("Could not open OS random number generator.");
    let ack_res = match self.keygen.handle_ack(sender, ack.clone())
    {
      Ok(outcome) => outcome,
      Err(e) =>
      {
        info!("Invalid Ack from {:?} : {:?}", sender, e);
        return;
      }
    };
    match ack_res
    {
      AckOutcome::Valid =>
      {}
      AckOutcome::Invalid(fault) =>
      {
        info!("Could not process Ack: {:?}", fault);
        return;
      }
    }
    self.expected_acks[0].1.pop_front();
    if self.expected_acks[0].1.is_empty()
    {
      self.expected_acks.pop_front();
      if self.expected_acks.is_empty()
      {
        self.is_done = true;
      }
    }
  }
  pub fn maybe_buffer(&mut self, sender: &NodeId, msg: SyncKeyGenMessage)
  {
    if let SyncKeyGenMessage::Part(_) = msg
    {
      if self.expected_parts.is_empty()
      {
        return;
      }
    }

    self.buffered_messages.push((sender.clone(), msg));
  }
  /// process incoming message and return generated acks, if any
  pub fn process_message(&mut self, sender: &NodeId, msg: SyncKeyGenMessage) -> Vec<SyncKeyGenMessage>
  {
    self.clean_queues();
    //check if we are done;
    let mut ret: Vec<SyncKeyGenMessage> = Vec::new();
    if self.expected_acks.is_empty() || self.is_done
    {
      self.is_done = true;
      return Vec::new();
    }
    if !self.is_next_message(sender, &msg)
    {
      self.maybe_buffer(sender, msg);
      return Vec::new();
    }
    let mut spl = sender.clone();
    let mut nmsg = msg;
    //let mut unproc = true;

loop
    {
      match nmsg
      {
        SyncKeyGenMessage::Part(parted) => ret.append(&mut self.process_part(&spl, parted)),
        SyncKeyGenMessage::Ack(src, ack) => self.process_ack(&spl, &src, ack),
      };
      let index = self
        .buffered_messages
        .iter()
        .position(|(snd, msg)| self.is_next_message(snd, msg));
      match index
      {
        Some(i) =>
        {
          let (aspl, anmsg) = self.buffered_messages.remove(i);
          spl = aspl;
          nmsg = anmsg;
          //unproc = true;
        }
        None =>
        {
         // unproc = false;
          break;
        }
      }
    }
    info!(
      "Processed keygen, still expecting {:?} Parts and {:?} base Acks",
      self.expected_parts.len(),
      self.expected_acks.len()
    );
    ret
  }
}

//#[derive(Encode,Decode)]
pub struct SharedConfig
{
  pub is_observer: bool,
  pub secret_share: Option<SecretKeyShare>,
  pub keyset: Option<PublicKeySet>,
  pub my_peer_id: PeerId,
  pub my_auth_id: AuthorityId,
  pub batch_size: u64,
}

pub enum BadgerState<B: BlockT, D>
where
  D: ConsensusProtocol<NodeId = NodeId>, //specialize to avoid some of the confusion
  D::Message: Serialize + DeserializeOwned,
{
  /// Waiting for all initial validators to be connected
  AwaitingValidators,

  /// Block numbers should be aligned to start batch processing
  InitialSync,
  /// Running genesis keygen
  KeyGen(KeyGenState),
  /// Running completed Badger node
  Badger(BadgerNode<B, D>),

  /// need a Join Plan to become observer
  AwaitingJoinPlan,
}

pub struct JustificationCollector<B:BlockT>
{
 /// contemporary authorities for this block hash
 pub contemp_auth:Option<Vec<AuthorityId>>,
 pub justification:Vec<BadgerJustification<B>>
}
use network::ClientHandle as NetClient;
pub use sp_blockchain::{HeaderBackend,HeaderMetadata};

pub struct BatchBlockMechanics<B:BlockT>
{
  queued_block:Option<B>, //block awaiting finalization
  overflow:VecDeque<BadgerTransaction>,
  queued_batches:VecDeque<BatchType>,
  pending_batch:Option<BatchType>,

}
pub struct ValidatorSync<B:BlockT>
{
  pub best_block_num: NumberFor<B>,
  pub best_block_hash: B::Hash,
  pub id:AuthorityId,
}
pub struct BadgerSyncState<B:BlockT>
{
//  pub total_best_block_num: NumberFor<B>,
//  pub total_best_block_hash: B::Hash,
//  pub our_best_block_num: NumberFor<B>,//we might not be a validator... 
//  pub our_best_block_hash: B::Hash,
  pub initial_sync_done:bool,
  pub validators:Vec<ValidatorSync<B>>
}


pub struct BadgerStateMachine<B: BlockT, D,Cl,BPM,Aux>
where
  D: ConsensusProtocol<NodeId = NodeId>, //specialize to avoid some of the confusion
  D::Message: Serialize + DeserializeOwned,
  B::Hash :Ord,
  Cl:NetClient<B>,
  BPM:BlockPusherMaker<B>,
  Aux:AuxStore,
{
  pub state: BadgerState<B, D>,
  pub peers: Peers,
  pub config: SharedConfig,
  pub queued_transactions: Vec<Vec<u8>>,
  pub persistent: BadgerPersistentData,
  pub keystore: KeyStorePtr,
  pub cached_origin: Option<AuthorityPair>,
  pub queued_messages: Vec<(u64, GossipMessage<B>)>,
  pub justification_collector: BTreeMap<B::Hash,JustificationCollector<B>>,
  pub delayed_justifications:Vec<(u64,BadgerJustification<B> )>,
  pub client: Arc<Cl>,
  pub block_maker:BPM,
  pub aux_backend:Aux,
  pub mech:BatchBlockMechanics<B>,
  pub sync_state:BadgerSyncState<B>,
  pub output_message_buffer:Vec<(LocalTarget<B>, GossipMessage<B>)>,
  pub finalizer: Box<dyn FnMut( &B::Hash,Option<Justification>)->bool+Send+Sync>,
}

const MAX_QUEUE_LEN: usize = 1024;

use threshold_crypto::serde_impl::SerdeSecret;

#[derive(Serialize, Deserialize, Debug)]
pub struct BadgerAuxCrypto
{
  pub secret_share: Option<SerdeSecret<SecretKeyShare>>,
  pub key_set: PublicKeySet,
  pub set_id: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SyncKeyGenMessage
{
  Part(Part),
  Ack(NodeId, Ack),
}

/* pub struct BadgerContainer<Block: BlockT, N: Network<Block>,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  pub val: Arc<BadgerGossipValidator<Block,Cl>>,
  pub network: N,
}*/
use crate::aux_store;

/*
impl<B, N,Cl> BadgerHandler<B> for BadgerContainer<B, N,Cl>
where
  B: BlockT,
  N: Network<B>,
  Cl:NetClient<B>,
  B::Hash:Ord,
{

  fn get_current_authorities(&self)->AuthorityList
  {
    let cloned:Vec<_>;
    {
    let locked=self.val.inner.read();
    cloned=locked.persistent.authority_set.inner.read().current_authorities.iter().cloned().collect();
    }
    cloned
  }

  fn emit_justification(&self,hash:&B::Hash,auth_list:AuthorityList)
  {
  {
    let locked=self.val.inner.read();
    if !locked.is_authority() { return;}
  }
  self.val.do_emit_justification(hash, &self.network,auth_list)
  }
  fn vote_for_validators<SBe>(&self, auths: Vec<AuthorityId>, backend: &SBe) -> Result<(), Error>
  where
    SBe: AuxStore,
  {
    {
      let lock = self.val.inner.read();
      let aset = lock.persistent.authority_set.inner.read();
      let opt = aux_store::AuthoritySet {
        current_authorities: auths.clone(),
        self_id: aset.self_id.clone(),
        set_id: aset.set_id,
      };
      match aux_store::update_vote(
        &Some(opt),
        |insert| backend.insert_aux(insert, &[]),
        |delete| backend.insert_aux(&[], delete),
      )
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          warn!("Couldn't save vote to disk {:?}, might not be important", e);
        }
      }
    }

    self.val.do_vote_validators(auths, &self.network)
  }
  fn vote_change_encryption_schedule(&self, e: EncryptionSchedule) -> Result<(), Error>
  {
    self.val.do_vote_change_enc_schedule(e, &self.network)
  }
  fn notify_node_set(&self, _v: Vec<NodeId>) {}
  fn update_validators<SBe>(&self, new_validators: BTreeMap<NodeId, AuthorityId>, backend: &SBe)
  where
    SBe: AuxStore,
  {
    let nex_set;
    let aux: BadgerAuxCrypto;
    let self_id;
    {
      let mut lock = self.val.inner.write();
      lock.load_origin();
      {
        nex_set = lock.persistent.authority_set.inner.read().set_id + 1;
        self_id = lock.persistent.authority_set.inner.read().self_id.clone();
      }
      match lock.state
      {
        BadgerState::Badger(ref mut node) =>
        {
          let secr = node.algo.inner().netinfo().secret_key_share().clone();
          let ikset = node.algo.inner().netinfo().public_key_set();

          // let kset=Some(ikset.clone());
          aux = BadgerAuxCrypto {
            secret_share: match secr
            {
              Some(s) => Some(SerdeSecret(s.clone())),
              None => None,
            },
            key_set: ikset.clone(),
            set_id: (nex_set) as u32,
          };
        }
        _ =>
        {
          warn!("Not in BADGER state, odd.  Bailing");
          return;
        }
      };
    }
    {
      let lock = self.val.inner.write();
      lock
        .keystore
        .write()
        .insert_aux_by_type(app_crypto::key_types::HB_NODE, &self_id.encode(), &aux)
        .expect("Couldn't save keys");
    }

    {
      let mut lock = self.val.inner.write();

      lock.config.keyset = Some(aux.key_set);
      lock.config.secret_share = match aux.secret_share
      {
        Some(s) => Some(s.0.clone()),
        None => None,
      };

      let mut aset = lock.persistent.authority_set.inner.write();
      aset.current_authorities = new_validators.iter().map(|(_, v)| v.clone()).collect();
      aset.set_id = nex_set;
      match aux_store::update_authority_set(&aset, |insert| backend.insert_aux(insert, &[]))
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          warn!("Couldn't write to disk, potentially inconsistent state {:?}", e);
        }
      };

      let topic = badger_topic::<B>();
      let packet = {
        let ses_mes = SessionData {
          ses_id: aset.set_id,
          session_key: lock.cached_origin.as_ref().unwrap().public(),
          peer_id: lock.config.my_peer_id.clone().into(),
        };
        let sgn = lock.cached_origin.as_ref().unwrap().sign(&ses_mes.encode());
        SessionMessage { ses: ses_mes, sgn: sgn }
      };
      let packet_data = GossipMessage::<B>::Session(packet).encode();
      self.network.register_gossip_message(topic, packet_data);
    }
  }
}*/

pub struct WHash<A:AsRef<[u8]>>(A);
impl<A:AsRef<[u8]>> std::cmp::Ord  for WHash<A> 
{
  fn cmp(&self, other: &Self) -> std::cmp::Ordering 
  {
    let our_bytes=self.0.as_ref();
    let other_bytes=other.0.as_ref();
    if our_bytes.len()>other_bytes.len() {return  std::cmp::Ordering::Greater;}
    if our_bytes.len()<other_bytes.len() {return  std::cmp::Ordering::Less;}
    let miter=our_bytes.iter().zip(other_bytes.iter());
    for (a,b) in miter
    {
       let c=a.cmp(b);
       if c!= std::cmp::Ordering::Equal
       {
         return c;
       }
    }
    std::cmp::Ordering::Equal
  }
}
impl<A:AsRef<[u8]>> std::cmp::Eq  for WHash<A> 
{
}

impl<A:AsRef<[u8]>> From<A> for WHash<A>
{
  fn from(inp:A) -> Self
  {
  WHash{0:inp}
  }
}

impl<A:AsRef<[u8]>> std::cmp::PartialEq  for WHash<A> 
{
  fn eq(&self, other: &Self) -> bool 
  {
    let our_bytes=self.0.as_ref();
    let other_bytes=other.0.as_ref();
    if our_bytes.len()>other_bytes.len() {return false;}
    if our_bytes.len()<other_bytes.len() {return  false;}
    let miter=our_bytes.iter().zip(other_bytes.iter());
    for (a,b) in miter
    {
       let c=a.cmp(b);
       if c!= std::cmp::Ordering::Equal
       {
         return false;
       }
    }
    true
  }
}


impl<A:AsRef<[u8]>> std::cmp::PartialOrd  for WHash<A> 
{
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering>
  {
    let our_bytes=self.0.as_ref();
    let other_bytes=other.0.as_ref();
    if our_bytes.len()>other_bytes.len() {return  Some(std::cmp::Ordering::Greater);}
    if our_bytes.len()<other_bytes.len() {return  Some(std::cmp::Ordering::Less);}
    let miter=our_bytes.iter().zip(other_bytes.iter());
    for (a,b) in miter
    {
       let c=a.cmp(b);
       if c!= std::cmp::Ordering::Equal
       {
         return Some(c);
       }
    }
    Some(std::cmp::Ordering::Equal)
  }
}


impl<B: BlockT,Cl,BPM,Aux> BadgerStateMachine<B, QHB,Cl,BPM,Aux>
where
Cl:NetClient<B>,
B::Hash:Ord,
BPM:BlockPusherMaker<B>,
Aux:AuxStore+Send+Sync+'static,
{
  pub fn new(
    keystore: KeyStorePtr, self_peer: PeerId, batch_size: u64, persist: BadgerPersistentData,
    client:Arc<Cl>,finalizer: Box<dyn FnMut( &B::Hash,Option<Justification>)->bool+Send+Sync>,bbld:BPM,astore:Aux
  ) -> BadgerStateMachine<B, QHB,Cl,BPM,Aux>
  {
    let ap: AuthorityPair;
    let is_ob: bool;

    {
      let aset = persist.authority_set.inner.read();
      ap = keystore
        .read()
        .key_pair_by_type::<AuthorityPair>(&aset.self_id.clone().into(), app_crypto::key_types::HB_NODE)
        .expect("Needs private key to work (bsm:new)");
      is_ob = !aset.current_authorities.contains(&aset.self_id);
      info!("SELFID: {:?} {:?}", &aset.self_id, &ap.public());
    }
    BadgerStateMachine {
      state: BadgerState::AwaitingValidators,
      peers: Peers::new(),
      config: SharedConfig {
        is_observer: is_ob,
        secret_share: None,
        keyset: None,
        my_peer_id: self_peer.clone(),
        batch_size: batch_size,
        my_auth_id: ap.public(),
      },
      queued_transactions: Vec::new(),
      keystore: keystore.clone(),
      cached_origin: Some(ap),
      persistent: persist,
      queued_messages: Vec::new(),
      justification_collector:BTreeMap::new(),
      delayed_justifications:Vec::new(),
      client:client.clone(),
      aux_backend:astore,
      block_maker:bbld,
      mech: BatchBlockMechanics {
        queued_block:None,
        overflow:VecDeque::new(),
        queued_batches:VecDeque::new(),
        pending_batch:None,
      },
      sync_state:
        BadgerSyncState {
          initial_sync_done:false,
          validators:Vec::new()
        }
      ,
      finalizer:finalizer,
      output_message_buffer:Vec::new(),
    }
    /*pub struct ValidatorSync<B:BlockT>
{
  pub best_block_num: NumberFor<B>,
  pub best_block_hash: B::Hash,
}
pub struct BadgerSyncState<B:BlockT>
{
  pub total_best_block_num: NumberFor<B>,
  pub total_best_block_hash: B::Hash,
  pub initial_sync_done:bool,
  pub validators:Vec<ValidatorSync<B>>
}*/
  }
  pub fn process_extracted(&mut self,logs:Vec<ExtractedLogs<B>>,)
  {
  for elog in logs.into_iter()
   {
   match elog
   {
     ExtractedLogs::ValidatorVote(authids) =>
     {
     info!("Voting for: {:?}",authids);
     match self.vote_for_validators(authids)
     {
       Ok(_) => {},
       Err(e) =>
       {info!("Error voting for validators: {:?}",e);}
     }
     },
     ExtractedLogs::NewSessionMessage(nsm) =>
     {
     
      self.output_message_buffer.push((LocalTarget::AllExcept(BTreeSet::new()),nsm));
       //net.register_gossip_message(topic,nsm.encode());
     }
   }
   }
  }


  pub fn notify_node_set(&self, _v: Vec<NodeId>)
  {
    //TODO
  }
  pub fn update_validators(&mut self, new_validators: BTreeMap<NodeId, AuthorityId>) ->Option<GossipMessage<B>>
  {
    let nex_set;
    let aux: BadgerAuxCrypto;
    let self_id;
    {
      //let mut lock = self.val.inner.write();
      self.load_origin();
      {
        nex_set = self.persistent.authority_set.inner.read().set_id + 1;
        self_id = self.persistent.authority_set.inner.read().self_id.clone();
      }
      match self.state
      {
        BadgerState::Badger(ref mut node) =>
        {
          let secr = node.algo.inner().netinfo().secret_key_share().clone();
          let ikset = node.algo.inner().netinfo().public_key_set();

          // let kset=Some(ikset.clone());
          aux = BadgerAuxCrypto {
            secret_share: match secr
            {
              Some(s) => Some(SerdeSecret(s.clone())),
              None => None,
            },
            key_set: ikset.clone(),
            set_id: (nex_set) as u32,
          };
        }
        _ =>
        {
          warn!("Not in BADGER state, odd.  Bailing");
          return None;
        }
      };
    }
    {
      //let lock = self.val.inner.write();
      self
        .keystore
        .write()
        .insert_aux_by_type(app_crypto::key_types::HB_NODE, &self_id.encode(), &aux)
        .expect("Couldn't save keys");
    }
    {
      

      self.config.keyset = Some(aux.key_set);
      self.config.secret_share = match aux.secret_share
      {
        Some(s) => Some(s.0.clone()),
        None => None,
      };

      let mut aset = self.persistent.authority_set.inner.write();
      aset.current_authorities = new_validators.iter().map(|(_, v)| v.clone()).collect();
      aset.set_id = nex_set;
      match aux_store::update_authority_set(&aset, |insert| self.aux_backend.insert_aux(insert, &[]))
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          warn!("Couldn't write to disk, potentially inconsistent state {:?}", e);
        }
      };

      let topic = badger_topic::<B>();
      let packet = {
        let ses_mes = SessionData {
          ses_id: aset.set_id,
          session_key: self.cached_origin.as_ref().unwrap().public(),
          peer_id: self.config.my_peer_id.clone().into(),
        };
        let sgn = self.cached_origin.as_ref().unwrap().sign(&ses_mes.encode());
        SessionMessage { ses: ses_mes, sgn: sgn }
      };
      return Some(GossipMessage::<B>::Session(packet));
      //let packet_data = GossipMessage::<B>::Session(packet).encode();
     // network.register_gossip_message(topic, packet_data);
    }
  }
  pub fn consume_batch(&mut self, batch:BatchType)
  {
    {
      self.mech.queued_batches.push_back(batch);
      if  self.mech.queued_block.is_some()
      {
        return;
      }
      let mbatch=self.mech.queued_batches.pop_front();
      if let Some(batch)=mbatch
      {
     
        match self.batch_to_block(batch) 
        {
        BatchProcResult::EmitJustification(hash,alist,logs) => 
         {   
          self.initiate_block_justification(hash,alist);
          self.process_extracted(logs);
           },
         BatchProcResult::Completed(logs) =>{ self.process_extracted(logs); },
         _ =>{}
          }
      }
    }

  }

  pub fn pre_finalize(&mut self, hash: &B::Hash,justne: BadgerFullJustification<B>) -> BatchProcResult<B>
  {
   
    let lmech=&mut self.mech;
	  let mblock;
	  {
	  mblock=std::mem::replace(&mut lmech.queued_block,None);
	  }
     

	  if let Some(block)=mblock
	  {
		if *hash== block.header().hash()
		{
      let mut ret=Vec::<ExtractedLogs<B>>::new();
      
      let (header, body) = block.deconstruct();

      let header_num = header.number().clone();
      let mut parent_hash = header.parent_hash().clone();
      let id = OpaqueDigestItemId::Consensus(&HBBFT_ENGINE_ID);

      /*let filter_log = |log: ConsensusLog<NumberFor<B>>| match log {
        ConsensusLog::ScheduledChange(change) => Some(change),
        _ => None, convert_f
      };*/
    
      // find the consensus digests with the right ID which converts to
      // the right kind of consensus log.
      let badger_logs:Vec<_>=header.digest().logs().iter().map(
        |x| x.try_to(id)).filter( |x:&Option<ConsensusLog>| x.is_some()).map(|x| x.unwrap()).collect();
      let  sid;	
      {
        sid=self.persistent.authority_set.inner.read().self_id.clone();
      }
      for log in badger_logs
      {
        //TODO!
        match log
        {
          ConsensusLog::VoteChangeSet(my_id,changeset) =>
          {
          if sid==my_id
           {
             info!("Log detected, VOTING {:?}",&changeset);
             ret.push(ExtractedLogs::ValidatorVote(changeset));
             info!("VOTE REGISTERED");
           }
          },
          ConsensusLog::NotifyChangedSet(newset) =>
          {
            info!("Notified of new set {:?}, doing nothing for now",&newset);
          }

        }
      }
        	let import_block: BlockImportParams<B> = BlockImportParams {
				origin: BlockOrigin::Own,
				header,
				justification: None,
				post_digests: vec![],
				body: Some(body),
				finalized: false,
				allow_missing_state:true,
				auxiliary: Vec::new(),
				fork_choice: ForkChoiceStrategy::LongestChain,
				import_existing:false,
			};
			let parent_hash = import_block.post_header().hash();
			let pnumber = *import_block.post_header().number();
			let parent_id = BlockId::<B>::hash(parent_hash);
			// go on to next block
			{
				let eh = import_block.header.parent_hash().clone();
        if let Err(e) =  self.block_maker.import_block(import_block)
				{
					warn!(target: "badger", "Error with block built on {:?}: {:?}",eh, e);
				 }
			}
			{
			if !(self.finalizer)(hash,Some(justne.encode()))
			{
        warn!("Failed finalization...");
				return BatchProcResult::Nothing;
      }
      let  pair = self.cached_origin.clone().unwrap();
      let gsp=BadgerSyncGossip::new(&pair,justne,pnumber);
      self.output_message_buffer.push((LocalTarget::AllExcept(BTreeSet::new()),GossipMessage::SyncGossip(gsp.clone())));
      self.output_message_buffer.push((LocalTarget::Keep(badger_sync::<B>()),GossipMessage::SyncGossip(gsp)));

		   }
    
			{
        let mbatch;
        {
          mbatch=lmech.queued_batches.pop_front();
        }
			if let Some(batch)=mbatch
			{
				return  self.batch_to_block(batch)
			}
	     	}
			return BatchProcResult::Completed(ret)
		}
	  }
    BatchProcResult::Nothing
  }

pub fn batch_to_block(&mut self, batch:BatchType ) ->BatchProcResult<B>
    {  
      //let lmech=&mut self.mech;

    let mut inherent_digests = generic::Digest { logs: vec![] };
    {
    self.mech.pending_batch=Some(batch.clone());
    }
    {
    info!(
      "Processing batch with epoch {:?} of {:?} transactions  and {:?} overflow into block",
      batch.epoch(),
      batch.len(),			
      self.mech.overflow.len());
       }
     let auth_list;
     {
      auth_list=self.persistent.authority_set.inner.read().current_authorities.clone();
     }
     let mut elogs=Vec::<ExtractedLogs<B>>::new();
     match batch.change()
     {
       ChangeState::None => {},
       ChangeState::InProgress(Change::NodeChange(pubkeymap)) => 
       {
        //change in progress, broadcast messages have to be sent to new node as well
        info!("CHANGE: in progress");
        self.notify_node_set(pubkeymap.iter().map(|(v,_)| v.clone().into() ).collect() )
       },
       ChangeState::InProgress(Change::EncryptionSchedule(_)) => {},//don't care?
       ChangeState::Complete(Change::NodeChange(pubkeymap)) => {
         info!("CHANGE: complete {:?}",&pubkeymap);
         let digest=BadgerPreRuntime::ValidatorsChanged(pubkeymap.iter().map(|(_,v)| v.clone().into() ).collect());
         inherent_digests.logs.push(DigestItem::PreRuntime(HBBFT_ENGINE_ID, digest.encode()));
         //update internals before block is created?
         if let Some(msg)= self.update_validators(pubkeymap.iter().map(| (c,v)| (c.clone().into(),v.clone().into()) ).collect() )
         {
         elogs.push(ExtractedLogs::NewSessionMessage(msg));
         }
       },
       ChangeState::Complete(Change::EncryptionSchedule(_)) => {},//don't care?
     }
     match batch.join_plan()
     {
       Some(plan) =>{}, //todo: emit plan if there are waiting nodes
       None =>{}
     }
     let chain_head = match self.block_maker.best_chain() {
      Ok(x) => x,
      Err(e) => {
        //warn!(target: "formation", "Unable to author block, no best block header: {:?}", e);
        return BatchProcResult::Nothing ;
      }
    };
    let parent_hash = chain_head.hash();
    let pnumber = *chain_head.number();
    let parent_id = BlockId::hash(parent_hash);

    {
      let mut locked=&mut self.mech.overflow;
    let block=  match self.block_maker.process_all(parent_id,inherent_digests,&mut locked,batch.into_tx_iter())
      {
        Ok(val) => val,
        Err(_) => {
          warn!("Error in block creation");
          return BatchProcResult::Nothing;
        }
      };
       
      info!("Prepared block for proposing at {} [hash: {:?}; parent_hash: {};",
            block.header().number(),
            <B as BlockT>::Hash::from(block.header().hash()),
            block.header().parent_hash(),);
  
      if Decode::decode(&mut block.encode().as_slice()).as_ref() != Ok(&block) {
              error!("Failed to verify block encoding/decoding");
            }
      
      if let Err(err) =
            evaluation::evaluate_initial(&block, &parent_hash, pnumber)
          {
            error!("Failed to evaluate authored block: {:?}", err);
          }	

      let header_hash = block.header().hash();
      //self.handler.emit_justification(&header_hash,auth_list.clone());
      {
        self.mech.queued_block=Some(block)
        
      }
      BatchProcResult::EmitJustification(header_hash,auth_list,elogs)
    }
  }


  #[inline]
  pub fn load_origin(&mut self)
  {
    if self.cached_origin.is_none()
    {
      let pair: AuthorityPair;
      {
        pair = self
          .keystore
          .read()
          .key_pair_by_type::<AuthorityPair>(&self.config.my_auth_id.clone().into(), app_crypto::key_types::HB_NODE)
          .expect("Needs private key to work");
      }
      self.cached_origin = Some(pair);
    }
  }
  pub fn imported_block_number(&mut self,num: NumberFor::<B>)
  {
    {
      
  
      
      if let Some(ref block)= &self.mech.queued_block
      {
        let cur_num=block.header().number();
        if num<*cur_num
        {
          //we have imported block that overwrote already finalized one....
           panic!("Refinalizing the block should not happen");
        }
          //need to shift up one... discard currrent block and rebuild...
        //otherwise, recreate the block and hope for the best...

        let extr_batch=std::mem::replace(&mut self.mech.pending_batch,None);
        if extr_batch.is_none()
        {
          panic!("Invalid state: block pending but batch unset");
        }
        info!("Imported block number {:?}, shifting up... ",&num);
      }
    }
        
  }
  pub fn process_justification(n_jst:BadgerJustification<B>, existing:&mut JustificationCollector<B>)
  {
    //self.load_origin();

    if existing.contemp_auth.is_none()
    {
      warn!("Should never be the case! - no authorities for block");
      return;
    }
    if existing.justification.iter().find(|x| x.validator==n_jst.validator).is_none()
     {
      if existing.contemp_auth.as_ref().unwrap().iter().find(|&x| *x==n_jst.validator).is_some()
      {
      existing.justification.push(n_jst);
      }
    }
  }
 pub fn verify_full_justification(&self,just:&BadgerSyncData<B> ) ->bool
 {
  
   if just.num>self.client.info().chain.best_number+1.into()
   {
     info!("Cannot validate justification due to possible authority set change..");
     return false;
   }
   if just.num+1.into()<self.client.info().chain.best_number
   {
     info!("Justification too old, too lazy to validate");
     return false;
   }
   if !just.justification.verify()
   {
     return false;
   }

  let mut count_total=0;
  let mut count_accepted=0;
  for authority in self.persistent.authority_set.inner.read().current_authorities.iter()
  {
    count_total+=1;
    if just.justification.commits.iter().find( |x| 
      {
       x.validator==*authority
    } ).is_some()
    {
      count_accepted+=1;
    }
  }
  let tolerated= count_total - badger::util::max_faulty(count_total);
   
  count_accepted>=tolerated
   }
  pub fn check_justification_completion(&mut self,hkey: &B::Hash)->BatchProcResult<B>
  {
    if !self.is_authority()
    {
      //an observer uses a different mechanism... 
      return  BatchProcResult::Nothing;
    }
    let  existing= match self.justification_collector.get_mut(hkey)
    {
      Some(k) => k,
      None => {return  BatchProcResult::Nothing;}
    };

    let authn=existing.contemp_auth.as_ref().unwrap().len();
    let tolerated= authn - badger::util::max_faulty(authn);
    let collected=existing.justification.len();
    info!("justificatioN collected: {:?} tolerated: {:?} hash: {:?}",collected,tolerated,hkey);
    if collected>=tolerated
    {//justification complete
      let hash=existing.justification[0].hash.clone();
      info!("Justification complete for {:?}",&hash);
      let full = BadgerFullJustification::<B>
      {
        hash:hash.clone(),
        commits: existing.justification.drain(..).map(|x| BadgerAuthCommit{validator:x.validator,sgn:x.sgn}).collect()
      };

      let res=self.pre_finalize(hkey,full);

      info!("Justification complete for {:?}",hkey);
      
      for tpl in self.delayed_justifications.iter_mut()
      {
        (*tpl).0+=1;
      }
      //retain only justifications for some recent blocks
      self.delayed_justifications.retain(|x| x.0<MAX_DELAYED_JUSTIFICATIONS);
      res
    }
    else
    {
      BatchProcResult::Nothing
    }
  }

  pub fn extract_state(&mut self)->Vec<(LocalTarget<B>,GossipMessage<B>)>
  {
//self.output_message_buffer
std::mem::replace(&mut self.output_message_buffer,Vec::new())
  }
  pub fn  flush_state(&mut self)
  {
    self.load_origin();
   let  sid = self.config.my_peer_id.clone();
   let  pair = self.cached_origin.clone().unwrap();
    let mut drain=Vec::new();
      if let BadgerState::Badger(ref mut state) = &mut self.state
      {
        debug!("BaDGER!! Flushing {} messages_net", &state.out_queue.len());
        drain = state
          .out_queue
          .drain(..)
          .map(|x| {
            (
              x.target,
              GossipMessage::BadgerData(BadgeredMessage::new(pair.clone(), &x.message)),
            )
          })
          .collect();
      }
      self.output_message_buffer.append(&mut drain);

  }

  //pub fn initiate_block_justification(&mut self,n_jst:BadgerJustification<B>,auth_list:AuthorityList) ->BatchProcResult<B>
  pub fn initiate_block_justification(&mut self,hkey: B::Hash,auth_list:AuthorityList) 
  {
    self.load_origin();
    let pair=self.cached_origin.as_ref().unwrap().clone();
    let authid=pair.public();
    
//////////////////////////////////////
    let mut n_hash=hkey;
    let mut n_list=auth_list;
    
      //let locked=self.
    loop 
    {
    info!("Emitting Justification {:?}",&n_hash);
    let sgn;
    sgn = pair.sign(&n_hash.encode());
      
    
  let n_jst= BadgerJustification::<B>
   {
    hash:n_hash.clone(),
     validator:authid.clone(),
     sgn:sgn 
   };
   let  existing=self.justification_collector.entry(hkey.clone()).or_insert(
    JustificationCollector {
    contemp_auth:Some(n_list),
    justification:Vec::new(),
     });
     if existing.contemp_auth.is_none()
     {
       warn!("Should never be the case!");
       return ; 
     }


     {
      let mut remaining:Vec<_>=Vec::new();     
      for jst in self.delayed_justifications.drain(..)
       {
       if jst.1.hash==n_jst.hash
        {
        //justification
        Self::process_justification(jst.1,existing)
        }
        else
        {
        remaining.push(jst);
        }
       }
       self.delayed_justifications=remaining;
       Self::process_justification(n_jst.clone(), existing);
      }
      if self.is_authority() //don't emit if we are not authority
      {
      self.output_message_buffer.push((LocalTarget::Keep(n_jst.hash.clone()), GossipMessage::JustificationData(n_jst.clone())) );
     
      self.output_message_buffer.push((LocalTarget::AllExcept(BTreeSet::new()), GossipMessage::JustificationData(n_jst)) );
      }
     
      let cres =self.check_justification_completion(&hkey);
      match cres
      {
       BatchProcResult::Completed(vc) =>  { self.justification_collector.remove(&hkey); self.process_extracted(vc); break},
       BatchProcResult::EmitJustification(hash,alist,logs) => {  
                       self.process_extracted(logs); 
                       n_hash=hash;
                       n_list=alist;
                       },
       BatchProcResult::Nothing =>{ break; }
      }
      
    
    }

/////////////////////////


  }
  pub fn queue_transaction(&mut self, tx: Vec<u8>) -> Result<(), Error>
  {
    if self.queued_transactions.len() >= MAX_QUEUE_LEN
    {
      return Err(Error::Badger("Too many transactions queued".to_string()));
    }
    self.queued_transactions.push(tx);
    Ok(())
  }

  pub fn push_transaction(
    &mut self, tx: Vec<u8>,
  ) ->  Result<(), Error>
  
  //Result<CpStep<QHB>, badger::sender_queue::Error<badger::queueing_honey_badger::Error>>
  {
    match &mut self.state
    {
      BadgerState::AwaitingValidators | BadgerState::AwaitingJoinPlan | BadgerState::KeyGen(_) | BadgerState::InitialSync =>
      {
        match self.queue_transaction(tx)
        {
          Ok(_) => Ok(()),
          Err(_) => Err(Error::Badger("Transaction Queue full".to_owned())),
        }
      }
      BadgerState::Badger(ref mut node) =>
      {
          match node.push_transaction(tx)
          {
            Ok(step) =>
            {
              match node.process_step(step)
              {
                Ok(_) =>
                { Ok(()) }
                Err(e) => return Err(Error::Badger(e.to_string())),
              }
            }
            Err(e) => return Err(Error::Badger(e.to_string())),
          }
       
      } 
    }
  }

  pub fn proceed_to_badger(&mut self) -> Vec<(LocalTarget<B>, GossipMessage<B>)>
  {
    self.load_origin();

    let node: BadgerNode<B, QHB> = BadgerNode::<B, QHB>::new(
      self.config.batch_size as usize,
      self.config.secret_share.clone(),
      self.persistent.authority_set.inner.read().current_authorities.clone(),
      self.config.keyset.clone().unwrap(),
      self.cached_origin.as_ref().unwrap().public(),
      self.config.my_peer_id.clone(),
      self.keystore.clone(),
      &self.peers,
    );
    self.state = BadgerState::Badger(node);
    let bypass: Vec<_> = self.queued_transactions.drain(..).collect();
    for tx in bypass.into_iter()
    {
      match self.push_transaction(tx)
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          info!("Error pushing queued {:?}", e);
        }
      }
    }
    Vec::new()
  }
  pub fn proceed_to_sync(&mut self) -> Vec<(LocalTarget<B>, GossipMessage<B>)>
  {
    let ln = self.persistent.authority_set.inner.read().current_authorities.len();
    let mut cur;
    {
      let iaset = self.persistent.authority_set.inner.read();
      cur = iaset
        .current_authorities
        .iter()
        .filter(|&n| self.peers.inverse.contains_key(n))
        .count();
      if self.is_authority() &&
        !self
          .peers
          .inverse
          .contains_key(&self.cached_origin.as_ref().unwrap().public())
      {
        cur = cur + 1;
      }
    }
    let info = self.client.info().chain;
    let best_number = info.best_number;
    let b_id=BlockId::Hash(info.best_hash);
    let hd=self.client.justification(&b_id);
    	/// Get block justification.
	//fn justification(&self, id: &BlockId<Block>) -> Result<Option<Justification>, Error>;
   let cur_just= match hd
    {
      Ok(opt) =>
      {
        if let Some(just)=opt
        {
           let our_just:Result<BadgerFullJustification<B>,_>=Decode::decode(&mut &just[..]);
           match our_just
           {
            Ok(jst) =>{jst},
            Err(_) =>
            {
              panic!("We have invalid justification encoding in non-genesis block");
            }
           }
        }
        else
        {
          if best_number==0.into()
          {
             //genesis...
             BadgerFullJustification
             {
               hash:info.best_hash,
               commits:Vec::new(),
             }
          }
          else
          {
            panic!("We managed to import unjustified block");
          }
        }
      }
      Err(_) =>{ panic!("We don't know our best block!");}
     };
     let pair=self.cached_origin.as_ref().unwrap();
     let gossip=BadgerSyncGossip::new(pair,cur_just,best_number,);
    
    self.state = BadgerState::InitialSync;
  vec![ (LocalTarget::Keep(badger_sync::<B>()), GossipMessage::SyncGossip(gossip.clone())),(LocalTarget::AllExcept(BTreeSet::new()), GossipMessage::SyncGossip(gossip)) ] 
  }

  pub fn proceed_to_keygen(&mut self) -> Vec<(LocalTarget<B>, GossipMessage<B>)>
  {
    self.load_origin();

    //check if we already have the necessary keys
    let aset = self.persistent.authority_set.inner.read().clone();
    let mut should_badger = false;
    let should_regen = match self
      .keystore
      .read()
      .get_aux_by_type::<BadgerAuxCrypto>(app_crypto::key_types::HB_NODE, &aset.self_id.encode())
    {
      Ok(data) =>
      {
        if aset.set_id == data.set_id
        {
          self.config.keyset = Some(data.key_set);
          self.config.secret_share = match data.secret_share
          {
            Some(ss) => Some(ss.0),
            None => None,
          };
          should_badger = true;
          false
        }
        else
        {
          self
            .keystore
            .write()
            .delete_aux(app_crypto::key_types::HB_NODE, &aset.self_id.encode())
            .expect("Could not delete keystore");

          true
        }
      }
      Err(_) => true,
    };
    if should_badger
    {
      return self.proceed_to_badger();
    }
    info!("Keygen: should regen :{:?}", &should_regen);
    if should_regen
    {
      let mut rng = rand::rngs::OsRng::new().expect("Could not open OS random number generator.");
      let secr: SecretKey = bincode::deserialize(&self.cached_origin.as_ref().unwrap().to_raw_vec()).unwrap();
      info!("Our secret key : {:?} pub: {:?}", &secr, &secr.public_key());
      let thresh = badger::util::max_faulty(aset.current_authorities.len());
      let val_pub_keys: PubKeyMap<NodeId> = Arc::new(
        aset
          .current_authorities
          .iter()
          .map(|x| {
            (if *x == self.cached_origin.as_ref().unwrap().public()
            {
              (
                std::convert::Into::<PeerIdW>::into(self.config.my_peer_id.clone()),
                x.clone().into(),
              )
            }
            else
            {
              (
                std::convert::Into::<PeerIdW>::into(
                  self
                    .peers
                    .inverse
                    .get(x)
                    .expect("All validators should be mapped at this point")
                    .clone(),
                ),
                x.clone().into(),
              )
            })
          })
          .collect(),
      );
      info!("VAL_PUB {:?} {:?}", &val_pub_keys, &self.config.my_peer_id);
      let (skg, part) = SyncKeyGen::new(
        self.config.my_peer_id.clone().into(),
        secr,
        val_pub_keys.clone(),
        thresh,
        &mut rng,
      )
      .expect("Failed to create SyncKeyGen! ");
      let mut template_pre: Vec<NodeId> = val_pub_keys.iter().map(|(k, _)| k.clone()).collect();
      template_pre.sort();
      let template: VecDeque<_> = template_pre.into_iter().collect();
      let mut state = KeyGenState {
        is_observer: self.config.is_observer,
        keygen: skg,
        part: None,
        threshold: thresh as usize,
        buffered_messages: Vec::new(),
        expected_parts: template.clone(),
        expected_acks: template.iter().map(|x| (x.clone(), template.clone())).collect(),
        is_done: false,
      };
      let mut ret = vec![];
      if let Some(parted) = part
      {
        let pid = self.config.my_peer_id.clone().into();
        let mut acks = state.process_message(&pid, SyncKeyGenMessage::Part(parted.clone()));
        info!("Generated {:?} ACKS", acks.len());
        if acks.len() > 0
        {
          acks.append(&mut state.process_message(&pid, acks[0].clone()));
        }
        acks.push(SyncKeyGenMessage::Part(parted.clone()));
        ret = acks
          .into_iter()
          .map(|msg| {
            (
              LocalTarget::AllExcept(BTreeSet::new()),
              GossipMessage::KeygenData(BadgeredMessage::new(
                self.cached_origin.as_ref().unwrap().clone(),
                &bincode::serialize(&msg).expect("Serialize error in inital keygen processing"),
              )),
            )
          })
          .collect();
      }

      self.state = BadgerState::KeyGen(state);
      info!("Returning {:?} values", ret.len());
      ret
    }
    else
    {
      //no need to regenerate, use existing saved data...
      Vec::new()
    }
  }

  pub fn is_authority(&self) -> bool
  {
    let aset = self.persistent.authority_set.inner.read();
    aset.current_authorities.contains(&aset.self_id)
  }
  pub fn is_sync_complete(&self,our_num:NumberFor<B>) ->bool
  {
    let mut cnt=0;
for vrf in self.sync_state.validators.iter()
{
  if our_num!=vrf.best_block_num 
  {

    return false;
  }
  cnt=cnt+1;
}
//TODO: make sync possible if some authorities are missing? 
return self.persistent.authority_set.inner.read().current_authorities.len()==cnt;

  }
pub fn process_sync_message (&mut self, sync:BadgerSyncGossip<B>) ->bool
{
  let info = self.client.info().chain;
  let our_best=info.best_number;
let our_hash=info.best_hash;
  if !sync.verify()
  {
  return self.is_sync_complete(our_best);
  }
  {
  let aset = self.persistent.authority_set.inner.read();
    if !aset.current_authorities.contains(&sync.source)
    {
      /// sync from non-authority, ignoring... 
      return self.is_sync_complete(our_best);
    }
  }
  let info=self.client.info().chain;

if sync.data.num<our_best
{
  info!("Discarding older sync theirs: {:?} ours: {:?}",&sync.data.num,&our_best);
  return self.is_sync_complete(our_best);
}

if self.verify_full_justification(&sync.data)
{
   if let Some(ref mut vrf)=self.sync_state.validators.iter_mut().find( |x| x.id==sync.source )
   {
    if vrf.best_block_num<sync.data.num
    {
      vrf.best_block_num=sync.data.num;
      vrf.best_block_hash=sync.data.justification.hash;
    }
   }
   else
   {
    self.sync_state.validators.push(
      ValidatorSync
      {
    best_block_hash: sync.data.justification.hash,
      best_block_num:sync.data.num,
      id:sync.source,
      }
    )
   }
   return self.is_sync_complete(our_best);
}
else
{
  info!("Discarding invalid justification");
  return self.is_sync_complete(our_best);
}
  
  //self.sync_state
  /*  pub total_best_block_num: NumberFor<B>,
  pub total_best_block_hash: B::Hash,
  pub initial_sync_done:bool,
  pub validators:Vec<ValidatorSync<B>>*/
}
  pub fn process_decoded_message(&mut self, message: &GossipMessage<B>) -> (ValidationResult<B>,bool)
  {
    let cset_id;
    {
      cset_id = self.persistent.authority_set.inner.read().set_id;
    }
    match message
    {
      GossipMessage::Session(ses_msg) =>
      {
        info!(
          "Session received from {:?} of {:?}",
          &ses_msg.ses.peer_id, &ses_msg.ses.session_key
        );
        if ses_msg.ses.ses_id == cset_id
        //??? needs session versions
        {
          let mut ret_msgs: Vec<(LocalTarget<B>, GossipMessage<B>)> = Vec::new();
          self
            .peers
            .update_id(&ses_msg.ses.peer_id.0, ses_msg.ses.session_key.clone());
          //debug!("Adding session key for {:?} :{:}")
          if let BadgerState::AwaitingValidators = self.state
          {
            let ln = self.persistent.authority_set.inner.read().current_authorities.len();
            let mut cur;
            {
              let iaset = self.persistent.authority_set.inner.read();
              cur = iaset
                .current_authorities
                .iter()
                .filter(|&n| self.peers.inverse.contains_key(n))
                .count();
              if self.is_authority() &&
                !self
                  .peers
                  .inverse
                  .contains_key(&self.cached_origin.as_ref().unwrap().public())
              {
                cur = cur + 1;
              }
            }
            info!(
              "Currently {:?} validators of {:?}, {:?} inverses",
              cur,
              ln,
              &self.peers.inverse.len()
            );
            if cur == ln
            {
              ret_msgs = self.proceed_to_keygen();
            }
          }
          self.output_message_buffer.append(&mut ret_msgs);
          //repropagate -automatic now
          return (ValidationResult::Discard,false);
        }
        else if ses_msg.ses.ses_id < cset_id
        {
          //discard
          return (ValidationResult::Punish(0),false);
        }
        else if ses_msg.ses.ses_id > cset_id + 1
        {
          //?? cache/propagate?
          return (ValidationResult::Discard,false);
        }
        else
        {
          return (ValidationResult::Discard,true);
        }
      },
      GossipMessage::SyncGossip(sync) =>
      {
        match &self.state
        {
          BadgerState::AwaitingValidators =>
          {
          //hmm. might as well register it?
          self.process_sync_message(sync.clone());
          return (ValidationResult::Discard,false);
          },
          BadgerState::AwaitingJoinPlan =>
          {
            self.process_sync_message(sync.clone());
            return (ValidationResult::Discard,false);
          },
          BadgerState::InitialSync => 
          {
            if self.process_sync_message(sync.clone())
            {
            let mut ret_msgs = self.proceed_to_keygen();
            self.output_message_buffer.append(&mut ret_msgs);
            }
            return (ValidationResult::Discard,false);
          },
          BadgerState::KeyGen(_)=>
          {
            return (ValidationResult::Discard,true);
          },
          BadgerState::Badger(_node)=>
          {
            if !self.is_authority()
            {
               //for observers, use this as justifier...
              if  self.verify_full_justification(&sync.data)
              {
              if let Some(ref block)=&self.mech.queued_block
               {
                   if block.header().hash()==sync.data.justification.hash 
                   {
                     let bhash=block.header().hash().clone();

                    self.pre_finalize(&bhash, sync.data.justification.clone());
                   }
               }
              }
            }
            return (ValidationResult::Discard,false);

          }
        }

      },

      GossipMessage::KeygenData(wkgen) =>
      {
        info!("Got Keygen message!");

        /*
        Got data from key generation. If we are still waiting for peers, cache it for the moment.
        If we are done with keygen and are in badger state... hmm, this is gossip. Ignore and propagate?
        In the keygen state, try to consume it
        */
        let orid: PeerIdW = match self.peers.inverse.get(&wkgen.originator)
        {
          Some(dat) => dat.clone().into(),
          None =>
          {
            if wkgen.originator==self.config.my_auth_id
            {
              self.config.my_peer_id.clone().into()
            }
            else
             {
            info!("Unknown originator for {:?}", &wkgen.originator);
            return (ValidationResult::Discard,true);
             }
          }
        };
        info!("Originator: {:?}, Peer: {:?}", &wkgen.originator, &orid);
        if let BadgerState::KeyGen(ref mut step) = &mut self.state
        {
          let k_message: SyncKeyGenMessage = match bincode::deserialize(&wkgen.data)
          {
            Ok(data) => data,
            Err(_) =>
            {
              warn!("Keygen message should be correct");
              return (ValidationResult::Punish(-2),false);
            }
          };
          info!("Msg: {:?}",&k_message);
        
          let acks = step.process_message(&orid, k_message);
          if step.is_done
          {
            info!("Initial keygen ready, generating... ");
            info!("Pub keys: {:?}", step.keygen.public_keys());
            let (kset, shr) = step.keygen.generate().expect("Initial key generation failed!");

            let aux = BadgerAuxCrypto {
              secret_share: match shr
              {
                Some(ref sec) =>
                {
                  info!("Gensec {:?}", sec.public_key_share());
                  Some(SerdeSecret(sec.clone()))
                }

                None => None,
              },
              key_set: kset.clone(),
              set_id: cset_id as u32,
            };

            self.config.keyset = Some(kset);
            self.config.secret_share = shr;
            {
              self
                .keystore
                .write()
                .insert_aux_by_type(
                  app_crypto::key_types::HB_NODE,
                  &self.persistent.authority_set.inner.read().self_id.encode(),
                  &aux,
                )
                .expect("Couldn't save keys");
            }
            let mut msgs=self.proceed_to_badger();
            self.output_message_buffer.append(&mut msgs);
            (ValidationResult::Discard,false)
          }
          else
          {
            let mut ret = acks
              .into_iter()
              .map(|msg| {
                (
                  LocalTarget::AllExcept(BTreeSet::new()),
                  GossipMessage::KeygenData(BadgeredMessage::new(
                    self.cached_origin.as_ref().unwrap().clone(),
                    &bincode::serialize(&msg).expect("Serialize error in  keygen processing"),
                  )),
                )
              })
              .collect();
              self.output_message_buffer.append(&mut ret);
              (ValidationResult::Discard,false)
          }
        }
        else
        {
          info!("Keygen data received while out of keygen");
          if let BadgerState::AwaitingValidators = &mut self.state
          {
            // should buffer here?
           return  (ValidationResult::Discard,true);
          }
          // propagate once?
          return (ValidationResult::Discard,false);
        }
      }
      GossipMessage::BadgerData(bdat) =>
      {
        let orid: PeerIdW = match self.peers.inverse.get(&bdat.originator)
        {
          Some(dat) => dat.clone().into(),
          None =>
          {
            if bdat.originator==self.config.my_auth_id
            {
              self.config.my_peer_id.clone().into()
            }
            else
            {
            info!("Unknown originator for {:?}", &bdat.originator);
            return (ValidationResult::Discard,true);
            }
          }
        };
        //we actually need to process observer state updates if we want to use SendQueue

        match self.state
        {
          BadgerState::Badger(ref mut badger) =>
          {
            info!("BadGER: got gossip message uid: {}", &bdat.uid,);
            if let Ok(msg) = bincode::deserialize::<<QHB as ConsensusProtocol>::Message>(&bdat.data)
            {
              match badger.handle_message(&orid, msg)
              {
                Ok(_) =>
                {
                  //send is handled separately. trigger propose? or leave it for stream
                  debug!("BadGER: decoded gossip message");

                  return (ValidationResult::Discard,false);
                }
                Err(e) =>
                {
                  info!("Error handling badger message {:?}", e);
                  telemetry!(CONSENSUS_DEBUG; "afg.err_handling_msg"; "err" => ?format!("{}", e));
                  return (ValidationResult::Discard,false);
                }
              }
            }
            else
            {
              return (ValidationResult::Discard,false);
            }
          }
          BadgerState::KeyGen(_) | BadgerState::AwaitingValidators =>
          {
            return (ValidationResult::Discard,true);
          }
          _ =>
          {
            warn!("Discarding badger message");
            return (ValidationResult::Punish(-1),false);
          }
        }
      },
      GossipMessage::JustificationData(just) =>
      {
        if !just.verify()
        {
          return (ValidationResult::Punish(-8),false);
        }
        if !self.is_authority()
         {
           // observers use Sync as verifiactions...
          return (ValidationResult::Discard,false);
         }
        let b_id=BlockId::Hash(just.hash);
        let hd=self.client.header(&b_id);
        let info = self.client.info().chain;
        let finalized_number = info.finalized_number;

        match hd 
        {
         Ok(opt) =>
         {
           if let Some(hdr)=opt
           {
           
           if *hdr.number()<=finalized_number
            {
              //we already have justification, hopefully
              info!("Discarding old justification");
              return    (ValidationResult::Discard,false);
            }
           }
         }
         Err(_) =>{}
        }
        info!("Got justification for {:?}",&just.hash);
        let hkey=just.hash.clone();
         match self.justification_collector.get_mut(&hkey)
         {
           Some(existing) => {
             info!("Some exists");
            Self::process_justification(just.clone(),existing);
            {
         
            match self.check_justification_completion(&hkey)
            {
             BatchProcResult::Completed(logs) =>
                 { 
                   self.justification_collector.remove(&hkey);
                   self.process_extracted(logs);
                 } ,
                 BatchProcResult::Nothing =>{},
                 BatchProcResult::EmitJustification(hash,alist,logs)=>
                 {
                  self.process_extracted(logs);
                   self.initiate_block_justification(hash, alist);
                 },
                 
                
            }
            }

           },
           None =>
           {
            info!("None exists");
             if self.delayed_justifications.iter().find( |x| (x.1.hash==just.hash && x.1.validator==just.validator)).is_some()
             {
                return  (ValidationResult::Discard,false);
             }
          self.delayed_justifications.push((0,just.clone()));
           }
         }
         //justification flood -automatic...
         (ValidationResult::Discard,false)
      }
    }
  }
  pub fn is_justification_expired(&self,hash:B::Hash) ->bool
  {
    let b_id=BlockId::Hash(hash);
    let hd=self.client.header(&b_id);
    let info = self.client.info().chain;
    let finalized_number = info.finalized_number;
    match hd 
    {
     Ok(opt) =>
     {
       if let Some(hdr)=opt
       {
       let five=NumberFor::<B>::one()+NumberFor::<B>::one()+NumberFor::<B>::one()+NumberFor::<B>::one()+NumberFor::<B>::one();
       if *hdr.number()+five<finalized_number
        {
         true
        }
        else
        {
          false
        }
       }
       else
       {
         true
       }
     }
     // we should not keep justifications for blocks we don't know
     Err(_) =>{ true}
    }

  }
  pub fn vote_change_encryption_schedule(&mut self, e: EncryptionSchedule) -> Result<(), &'static str>
  {
    match self.state
    {
      BadgerState::Badger(ref mut badger) =>
      {
        info!("BadGER: voting to change encrypt schedule ");

        match badger.vote_change_encryption_schedule(e)
        {
          Ok(_) =>
          {
            debug!("BadGER: voted");
            return Ok(());
          }
          Err(e) =>
          {
            info!("Error handling badger vote {:?}", e);
            return Err(e);
          }
        }
      }
      _ =>
      {
        warn!("Invalid state for voting");
        return Err("Invalid state".into());
      }
    }
  }
  pub fn vote_for_validators(&mut self, auths: Vec<AuthorityId>) -> Result<(), &'static str>
  {
    let mut map: BTreeMap<PeerIdW, PublicKey> = BTreeMap::new();
    for au in auths.into_iter()
    {
      if let Some(pid) = self.peers.inverse.get(&au)
      {
        map.insert(pid.clone().into(), au.into());
      }
      else
      {
        info!("No nodeId for {:?}", au);
        return Err("Missing NodeId".into());
      }
    }
    match self.state
    {
      BadgerState::Badger(ref mut badger) =>
      {
        info!("BadGER: voting to change authority set ");

        match badger.vote_for_validators(map)
        {
          Ok(_) =>
          {
            debug!("BadGER: voted");
            return Ok(());
          }
          Err(e) =>
          {
            info!("Error handling badger vote {:?}", e);
            return Err(e);
          }
        }
      }
      _ =>
      {
        warn!("Invalid state for voting");
        return Err("Invalid state".into());
      }
    }
  }

  pub fn process_message(
    &mut self, _who: &PeerId, mut data: &[u8],
  ) -> ( (ValidationResult<B>,bool),  Option<GossipMessage<B>>)
  //(SAction, Vec<(LocalTarget, GossipMessage<B>)>, Option<GossipMessage<B>>)
  {
    match GossipMessage::decode(&mut data)
    {
      Ok(message) =>
      {
        info!("GOt message from {:?} :{:?}", _who, &message);
        if !message.verify()
        {
          warn!("Invalid message signature in {:?}", &message);
          return (  (ValidationResult::Discard,false), None);
        }
        let a = self.process_decoded_message(&message);
        return (a, Some(message));
      }
      Err(e) =>
      {
        info!(target: "afg", "Error decoding message {:?}",e);
        telemetry!(CONSENSUS_DEBUG; "afg.err_decoding_msg"; "" => "");

        ((ValidationResult::Discard,false),  None)
      }
    }
  }


  pub fn process_and_replay(
    &mut self, who: &PeerId, data: &[u8],
  ) -> Vec<(ValidationResult<B>, Option<GossipMessage<B>>)>//, Vec<(LocalTarget, GossipMessage<B>)>)
  {
    let mut vals: Vec<(ValidationResult<B>, Option<GossipMessage<B>>)> = Vec::new();
    let (action,  msg) = self.process_message(who, data);
    match action
    {
      (act,true) =>
      {
        //message has not been processed
        match msg
        {
          Some(msg) => self.queued_messages.push((0, msg)),
          None =>
          {}
        }
        return vec![(act, None)] ; //no need to inform upstream, I think
      }
      (act,false) =>
      {
        //retry existing messages...
        vals.push((act, msg));
        let mut done = false;
        let mut iter = 0;
        while !done
        {
          let mut new_queue = Vec::new();
          done = true;
          let silly_compiler: Vec<_> = self.queued_messages.drain(..).collect();
          info!("Replaying {:?}", silly_compiler.len());
          for (_, msg) in silly_compiler.into_iter()
          {
            let s_act = self.process_decoded_message(&msg);
            match s_act
            {
              (_,true) =>
              {
                new_queue.push((0, msg));
              },
              (ValidationResult::Discard,false) =>
              {
                done = false;
              },
              (action,false) =>
              {
                vals.push((action, Some(msg)));
                done = false;
              }
            }
          }
          self.queued_messages = new_queue;
          iter = iter + 1;
          if iter > 100
          //just in case, prevent deadlock
          {
            break;
          }
        }
        return vals;
      }
    }
  }
}

impl<B: BlockT> BadgerNode<B, QHB>
{
  fn push_transaction(
    &mut self, tx: Vec<u8>,
  ) -> Result<CpStep<QHB>, badger::sender_queue::Error<badger::queueing_honey_badger::Error>>
  {
    info!("BaDGER pushing transaction {:?}", &tx);
    let ret = self.algo.push_transaction(tx, &mut self.main_rng);
    info!("BaDGER pushed: complete ");
    ret
  }
}

impl<B: BlockT, D: ConsensusProtocol<NodeId = NodeId>> BadgerNode<B, D> where D::Message: Serialize + DeserializeOwned {}
use std::thread;
pub type BadgerNodeStepResult<D> = CpStep<D>;
pub type TransactionSet = Vec<Vec<u8>>; //agnostic?

impl<B: BlockT> BadgerNode<B, QHB>
//where
//  D::Message: Serialize + DeserializeOwned,
{
  pub fn new(
    batch_size: usize, sks: Option<SecretKeyShare>, validator_set: AuthorityList, pkset: PublicKeySet,
    auth_id: AuthorityId, self_id: PeerId, keystore: KeyStorePtr, peers: &Peers,
  ) -> BadgerNode<B, QHB>
  {
    let mut rng = OsRng::new().unwrap();

    //let ap:app_crypto::hbbft_thresh::Public=hex!["946252149ad70604cf41e4b30db13861c919d7ed4e8f9bd049958895c6151fab8a9b0b027ad3372befe22c222e9b733f"].into();
    let secr: SecretKey = match keystore
      .read()
      .key_pair_by_type::<AuthorityPair>(&auth_id, app_crypto::key_types::HB_NODE)
    {
      Ok(key) => bincode::deserialize(&key.to_raw_vec()).expect("Stored key should be correct"),
      Err(_) => panic!("SHould really have key at this point"),
    };
    let mut vset: Vec<NodeId> = validator_set
      .iter()
      .cloned()
      .map(|x| {
        if x == auth_id
        {
          self_id.clone().into()
        }
        else
        {
          Into::<NodeId>::into(peers.inverse.get(&x).expect("All auths should be known").clone())
        }
      })
      .collect();
    vset.sort();

    let ni = NetworkInfo::<NodeId>::new(self_id.clone().into(), sks, (pkset).clone(), vset);

    let val_map: BTreeMap<NodeId, PublicKey> = validator_set
      .iter()
      .map(|auth| {
        if *auth==auth_id
        {
          
          (self_id.clone().into(), (*auth).clone().into())
        }
        else
        {
        let nid = peers.inverse.get(auth).expect("All authorities should have loaded!");
        (nid.clone().into(), (*auth).clone().into())
        }
      })
      .collect();

    let dhb = DynamicHoneyBadger::builder().build(ni, secr, Arc::new(val_map));
    let (qhb, qhb_step) = QueueingHoneyBadger::builder(dhb)
      .batch_size(batch_size)
      .build(&mut rng)
      .expect("instantiate QueueingHoneyBadger");

    let (sq, mut step) =
      SenderQueue::builder(qhb, peers.inner.keys().map(|x| (*x).clone().into())).build(self_id.clone().into());
    let output = step.extend_with(qhb_step, |fault| fault, BMessage::from);
    assert!(output.is_empty());
    let out_queue = step
      .messages
      .into_iter()
      .map(|msg| {
        let ser_msg = bincode::serialize(&msg.message).expect("serialize");
        SourcedMessage {
          sender_id: sq.our_id().clone(),
          target: msg.target.into(),
          message: ser_msg,
        }
      })
      .collect();
    let outputs = step.output.into_iter().collect();
    info!("BaDGER!! Initializing node");
    let node = BadgerNode {
      //id: self_id.clone(),
      node_id: self_id.clone().into(),
      algo: sq,
      main_rng: rng,
      // authorities: validator_set.clone(),
      //config: config.clone(),
      //in_queue: VecDeque::new(),
      out_queue: out_queue,
      outputs: outputs,
      _block: PhantomData,
    };
    node
  }
  fn process_step(
    &mut self, step: badger::sender_queue::Step<DynamicHoneyBadger<Vec<BadgerTransaction>, NodeId>>,
  ) -> Result<(), &'static str>
  {
    let out_msgs: Vec<_> = step
      .messages
      .into_iter()
      .map(|mmsg| {
        debug!("BaDGER!! Hundling  {:?} ", &mmsg.message);
        let ser_msg = bincode::serialize(&mmsg.message).expect("serialize");
        (mmsg.target, ser_msg)
      })
      .collect();
    self.outputs.extend(step.output.into_iter());
    debug!("BaDGER!! OK message, outputs: {} ", self.outputs.len());
    for (target, message) in out_msgs
    {
      self.out_queue.push_back(SourcedMessage {
        sender_id: self.node_id.clone(),
        target: target.into(),
        message,
      });
    }

    Ok(())
  }
  pub fn vote_change_encryption_schedule(&mut self, e: EncryptionSchedule) -> Result<(), &'static str>
  {
    match self.algo.vote_for(Change::EncryptionSchedule(e), &mut self.main_rng)
    {
      Ok(step) =>
      {
        return self.process_step(step);
      }
      Err(e) =>
      {
        info!("Error voting: {:?}", e);
        return Err("Error voting");
      }
    }
  }
  pub fn vote_for_validators(&mut self, new_vals: BTreeMap<PeerIdW, PublicKey>) -> Result<(), &'static str>
  {
    match self
      .algo
      .vote_for(Change::NodeChange(Arc::new(new_vals)), &mut self.main_rng)
    {
      Ok(step) =>
      {
        return self.process_step(step);
      }
      Err(e) =>
      {
        info!("Error voting: {:?}", e);
        return Err("Error voting");
      }
    }
  }
  pub fn handle_message(&mut self, who: &NodeId, msg: <QHB as ConsensusProtocol>::Message) -> Result<(), &'static str>
  {
    debug!("BaDGER!! Handling message from {:?} {:?}", who, &msg);
    match self.algo.handle_message(who, msg, &mut self.main_rng)
    {
      Ok(step) =>
      {
        return self.process_step(step);
      }
      Err(_) => return Err("Cannot handle message"),
    }
  }
}
pub struct BadgerGossipValidator<Block: BlockT,Cl,BPM,Aux>
where
Block::Hash:Ord,
Cl:NetClient<Block>,
BPM:BlockPusherMaker<Block>,
Aux:AuxStore
{
  // peers: RwLock<Arc<Peers>>,
  inner: RwLock<BadgerStateMachine<Block, QHB,Cl,BPM,Aux>>,
  pending_messages: RwLock<BTreeMap<PeerIdW, Vec<Vec<u8>>>>,
}
impl<Block: BlockT,Cl,BPM,Aux> BadgerGossipValidator<Block,Cl,BPM,Aux>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
BPM:BlockPusherMaker<Block>,
Aux:AuxStore+Send+Sync+'static
{
  pub fn on_block_imported(&self,num:NumberFor::<Block>)
  {
    self.inner.write().imported_block_number(num);
  }
  fn send_message(
    &self, who: &PeerId, vdata: Vec<u8>, context_val: &mut dyn ValidatorContext<Block>,
  )
  {
    context_val.send_single(who, vdata);
  }
  fn flush_message(
    &self, additional: &mut Vec<(LocalTarget<Block>, GossipMessage<Block>)>,  context_val:&mut dyn ValidatorContext<Block>,   )
  {
    info!("BaDGER!! Enter flush {:?}", thread::current().id());
    // let topic = badger_topic::<Block>();
    let spid;
    let mut drain: Vec<_> = Vec::new();
    {
      info!("Lock inner");
      let mut locked = self.inner.write();
      locked.flush_state();
      //add the buffer...
      let mut extr=locked.extract_state();
      drain.append(&mut extr);

      spid=locked.config.my_peer_id.clone();
      info!("UnLock inner");
      
    }
    drain.append(additional);
    if drain.len()==0
    {
      return;
    }
    {
      let mut ldict = self.pending_messages.write();
      let plist;
      {
      let inner = self.inner.read();
       plist = inner.peers.connected_peer_list();
      }
      for (k, v) in ldict.iter_mut()
      {
        if v.len() == 0
        {
          continue;
        }
        if plist.contains(&k.0)
        {
          for msg in v.drain(..)
          {
            info!("BaDGER!! RESending to {:?}", &k);
            self.send_message(&k.0, msg,  context_val);
          }
        }
      }
    }
    let mut self_directed=Vec::new();
    for (target, msg) in drain
    {
      if let &GossipMessage::Session(ref sdat) = &msg
      {
       if sdat.ses.peer_id.0==spid
       {
        context_val.keep(badger_session::<Block>(),msg.encode());
       }
      }
      //let vdata = GossipMessage::BadgerData(BadgeredMessage::new(pair,msg.message)).encode();
      let vdata = msg.encode();
      let spidw=PeerIdW {0: spid.clone() };
      match target
      {
        LocalTarget::Keep(cell) =>
        {
          context_val.keep(cell,vdata);
        },
        LocalTarget::Nodes(node_set) =>
        {
          if node_set.contains(&spidw)
          {
            self_directed.push(vdata.clone());
          }
          context_val.send_to_set(node_set.iter().map(|x| x.0.clone()).collect(), vdata.clone()); 
          let av_list;
          {
          let inner = self.inner.read();
          trace!("Nodes lock success");
          let peers = &inner.peers;
          av_list = peers.connected_peer_list();

          }
          for to_id in node_set.iter()
          {
            if !av_list.contains(&to_id.0)
            {
              let mut ldict = self.pending_messages.write();
              let stat = ldict.entry(to_id.clone()).or_insert(Vec::new());
              stat.push(vdata.clone());
            }
          }


        }
        LocalTarget::AllExcept(exclude) =>
        {
          debug!("BaDGER!! AllExcept  {}", exclude.len());
          if !exclude.contains(&spidw)
          {
            self_directed.push(vdata.clone());
          }
          let clist;
          let mut vallist: Vec<_>;
          context_val.broadcast_except(exclude.iter().map(|x| x.0.clone()).collect(), vdata.clone());

          {
            let locked = self.inner.write();
          info!("Allex lock success");
          
          let peers = &locked.peers;
          vallist = locked
            .persistent.authority_set
            .inner
            .read()
            .current_authorities
            .iter()
            .map(|x| {
              if *x == locked.config.my_auth_id
              {
                locked.config.my_peer_id.clone()
              }
              else
              {
                peers.inverse.get(x).expect("All auths should be known").clone()
              }
            }).filter(|n| !exclude.contains(&PeerIdW { 0: (*n).clone() }) )
            .collect();
            clist=peers.connected_peer_list();
            for pid in clist.iter()
            {
            let tmp = pid.clone();
            vallist.retain(|x| *x != tmp);
             }
             if vallist.len() > 0
             {
               let mut ldict = self.pending_messages.write();
               for val in vallist.into_iter()
               {
                 let stat = ldict.entry(val.clone().into()).or_insert(Vec::new());
                 stat.push(vdata.clone());
               }
   
               // context.register_gossip_message(topic,vdata.clone());
             }
          }
          info!("Excluded {:?}", &exclude);

        }
      }
    }
    {
      //loop self-directed messages... additional mesages will go ou on next flush
      for data in self_directed.into_iter()
      {
        let actions = self.inner.write().process_and_replay(&spid, &data);//TODO: Locks! cannot use handler inside...
        for (action, msg) in actions.into_iter()
        {
          match action
          {
            ValidationResult::Maintain(cell) =>
            {
              if let Some(mmsg) =msg{
              context_val.keep(cell,mmsg.encode());
              }
            }
           _ =>{}
          }
        }
      }


    }
    info!("BaDGER!! Exit flush {:?}", thread::current().id());
  }
  /// Create a new gossip-validator.
  pub fn new(keystore: KeyStorePtr, self_peer: PeerId, batch_size: u64, persist: BadgerPersistentData, client:Arc<Cl>,flizer:Box<dyn FnMut( &Block::Hash,Option<Justification>)->bool+Send+Sync>,
            bpusher:BPM,astore:Aux) -> Self
  {
    Self {
      inner: RwLock::new(BadgerStateMachine::<Block, QHB,Cl,BPM,Aux>::new(
        keystore, self_peer, batch_size, persist,client,flizer,bpusher,astore
      )),
      pending_messages: RwLock::new(BTreeMap::new()),
    }
  }
  /// collect outputs from
  pub fn pop_output(&self) -> Option<<QHB as ConsensusProtocol>::Output>
  {
    let mut locked = self.inner.write();
    match &mut locked.state
    {
      BadgerState::Badger(ref mut node) =>
      {
        info!("OUTPUTS: {:?}", node.outputs.len());
        node.outputs.pop_front()
      }
      _ => None,
    }
  }

  pub fn is_validator(&self) -> bool
  {
    let rd = self.inner.read();
    rd.is_authority()
  }

  pub fn push_transaction(&self, tx: Vec<u8>, net:  &mut dyn ValidatorContext<Block> ) -> Result<(), Error>
  {

    {
     match self.inner.write().push_transaction(tx)
     {
       Ok(_) =>{},
       Err(e) =>{info!("Error pushing transaction {:?}",e); },
     }
    }
    //send messages out
    {
      self.flush_message(&mut Vec::new(),net);
    }

    Ok(())
  }
  pub fn process_batch(&self,batch:<QHB as ConsensusProtocol>::Output,net:  &mut dyn ValidatorContext<Block>)
  {
    {
     self.inner.write().consume_batch(batch);
    }
    {
      self.flush_message(&mut Vec::new(), net);
    }
  }
  

/*  pub fn do_emit_justification(&self,hash:&Block::Hash, net:  &mut dyn ValidatorContext<Block>,auth_list:AuthorityList)
  {
    if !self.is_validator()
    {
      return;
    }
    {
      let mut inner=self.inner.write();
      inner.initiate_block_justification(hash.clone(),auth_list);
     }

    {
      self.flush_message(&mut Vec::new(), net);
     }

  }*/
  pub fn do_vote_validators(&self, auths: Vec<AuthorityId>, net:  &mut dyn ValidatorContext<Block>) -> Result<(), Error>
  {
    if !self.is_validator()
    {
      info!("Non-validator cannot vote");
      return Err(Error::Badger("Non-validator".to_string()));
    }

    {
      let mut locked = self.inner.write();
      match locked.vote_for_validators(auths)
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          return Err(Error::Badger(e.to_string()));
        }
      }
    }
    info!("DO_VOTE");
    //send messages out
    {
      self.flush_message(&mut Vec::new(), net);
    }

    Ok(())
  }
  pub fn do_vote_change_enc_schedule(&self, e: EncryptionSchedule, net:  &mut dyn ValidatorContext<Block>) -> Result<(), Error>
  {
    if !self.is_validator()
    {
      info!("Non-validator cannot vote");
      return Err(Error::Badger("Non-validator".to_string()));
    }
    {
      let mut locked = self.inner.write();
      match locked.vote_change_encryption_schedule(e)
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          return Err(Error::Badger(e.to_string()));
        }
      }

    }
    //send messages out
    {
      self.flush_message(&mut Vec::new(), net);
    }

    Ok(())
  }
 // pub fn update_session()
}
use gossip::PeerConsensusState;

impl<Block: BlockT,Cl,BPM,Aux> sc_network_ranting::Validator<Block> for BadgerGossipValidator<Block,Cl,BPM,Aux>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
Aux:AuxStore+Send+Sync+'static,
BPM:BlockPusherMaker<Block>
{

  fn new_peer(&self, context: &mut dyn ValidatorContext<Block>, who: &PeerId, _roles: Roles)
  {
    info!("New Peer called {:?}", who);
    {
      let mut inner = self.inner.write();
      inner.peers.update_peer_state(who, PeerConsensusState::Connected);
    };
    let scell=badger_session::<Block>();
    let packet_data = match context.get_kept(&scell)
    {
      Some (p) =>{p},
      None =>
      {
       let packet= {
          let mut inner = self.inner.write();
          inner.peers.new_peer(who.clone());
          inner.load_origin();
    
          let ses_mes = SessionData {
            ///TODO: mitigate sending of invalid peerid
            ses_id: inner.persistent.authority_set.inner.read().set_id,
            session_key: inner.cached_origin.as_ref().unwrap().public(),
            peer_id: inner.config.my_peer_id.clone().into(),
          };
          let sgn = inner.cached_origin.as_ref().unwrap().sign(&ses_mes.encode());
          SessionMessage { ses: ses_mes, sgn: sgn }
        };
        let packet_data = GossipMessage::<Block>::Session(packet).encode();
        context.keep(scell, packet_data.clone());
        packet_data
      }
    };
    context.send_single(who, packet_data);

    //if let Some(packet) = packet {
    
    //self.inner.write().process_and_replay(who, &packet_data);
   
    //}
  }

  fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<Block>, who: &PeerId)
  {
    self.inner.write().peers.peer_disconnected(who);
  }

  fn validate(
    &self, context: &mut dyn ValidatorContext<Block>, who: &PeerId, data: &[u8],
  ) -> sc_network_ranting::ValidationResult<Block>
  {
    info!("Enter validate {:?}", who);
    let actions = self.inner.write().process_and_replay(who, data);//TODO: Locks! cannot use handler inside...
    let mut ret = sc_network_ranting::ValidationResult::Discard;
    
    for (action, msg) in actions.into_iter()
    {
      match action
      {
        ValidationResult::Maintain(cell) =>
        {
          if let Some(mmsg) =msg{
          context.keep(cell,mmsg.encode());
          ret = sc_network_ranting::ValidationResult::Maintain(cell);
          }
        }
       _ =>{}
      }
    }
  
    {
      self.flush_message(&mut Vec::new(),context);
    }
    info!("Flushed");

    return ret;
  }



  fn message_expired<'b>(&'b self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'b>
  {
    //let inner = self.inner.read();
    Box::new(move |cell, mut data| {
      //only one topic, i guess we may add epochs eventually

      match GossipMessage::<Block>::decode(&mut data)
      {
        Err(_) => true,
        Ok(GossipMessage::Session(ses_data)) =>
        {
          if cell!=badger_session::<Block>()
          {
            return true;
          }
          ses_data.ses.ses_id < self.inner.read().persistent.authority_set.inner.read().set_id
        }
        Ok(GossipMessage::JustificationData(just)) =>
        {
          if badger_justification::<Block>(just.hash.clone())!=cell
          {
            return true;
          }
          self.inner.read().is_justification_expired(just.hash)
        }
        Ok(_) => true,
      }
    })
  }
}



pub trait Network<Block: BlockT>: RantingNetwork<Block> + Clone + Send + 'static {
	/// Notifies the sync service to try and sync the given block from the given
	/// peers.
	///
	/// If the given vector of peers is empty then the underlying implementation
	/// should make a best effort to fetch the block from any peers it is
	/// connected to (NOTE: this assumption will change in the future #3629).
	fn set_sync_fork_request(&self, peers: Vec<network::PeerId>, hash: Block::Hash, number: NumberFor<Block>);
}

impl<B, S, H> Network<B> for Arc<NetworkService<B, S, H>> where
	B: BlockT,
	S: network::specialization::NetworkSpecialization<B>,
	H: network::ExHashT,
{
	fn set_sync_fork_request(&self, peers: Vec<network::PeerId>, hash: B::Hash, number: NumberFor<B>) {
		NetworkService::set_sync_fork_request(self, peers, hash, number)
	}
}


/// A stream used by NetworkBridge in its implementation of Network.
pub struct NetworkStream
{
  inner: Option<mpsc::UnboundedReceiver<RawMessage>>,
  outer: oneshot::Receiver<mpsc::UnboundedReceiver<RawMessage>>,
}

impl Stream for NetworkStream
{
  type Item = RawMessage;

  fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>>
  {
    if let Some(ref mut inner) = self.inner
    {
      match inner.try_next()
      {
        Ok(Some(opt)) => return Poll::Ready(Some(opt)),
        Ok(None) => return Poll::Pending,
        Err(_) => return Poll::Pending,
      };
    }
    match self.outer.try_recv()
    {
      Ok(Some(mut inner)) =>
      {
        let poll_result = match inner.try_next()
        {
          Ok(Some(opt)) => Poll::Ready(Some(opt)),
          Ok(None) => Poll::Pending,
          Err(_) => Poll::Pending,
        };
        self.inner = Some(inner);
        poll_result
      }
      Ok(None) => Poll::Pending,
      Err(_) => Poll::Pending,
    }
  }
}

/// Bridge between the underlying network service, gossiping consensus messages and Badger
pub struct NetworkBridge<B: BlockT,Cl,BPM,Aux>
where
Cl:NetClient<B>,
B::Hash:Ord,
Aux:AuxStore,
BPM:BlockPusherMaker<B>,
{
  engine: sc_network_ranting::RantingEngine<B>,
  node: Arc<BadgerGossipValidator<B,Cl,BPM,Aux>>,
}
impl<B: BlockT,Cl,BPM,Aux>  BatchProcessor<B> for NetworkBridge<B, Cl,BPM,Aux>
where
Cl:NetClient<B>+'static,
B::Hash:Ord,
Aux:AuxStore+Send+Sync+'static,
BPM:BlockPusherMaker<B>,
{
  fn process_batch(&self,batch:<QHB as ConsensusProtocol>::Output)
  {
     self.engine.with_lock(|en|  self.node.process_batch(batch,en)) ;
  }
}


impl<B: BlockT, Cl,BPM,Aux> NetworkBridge<B,Cl,BPM,Aux>
where
Cl:NetClient<B>+'static,
B::Hash:Ord,
Aux:AuxStore+Send+Sync+'static,
BPM:BlockPusherMaker<B>+'static,
{
  pub fn on_block_imported(&self,num:NumberFor::<B>)
  {
  self.node.on_block_imported(num);
  }
  /// Create a new NetworkBridge to the given NetworkService. Returns the service
  /// handle and a future that must be polled to completion to finish startup.
  /// If a voter set state is given it registers previous round votes with the
  /// gossip service.
  pub fn new<N:RantingNetwork<B>+Send+Sync+Clone+'static>(
    service: N, config: crate::Config, keystore: KeyStorePtr, persist: BadgerPersistentData,client:Arc<Cl>,flizer:Box<dyn FnMut( &B::Hash,Option<Justification>)->bool+Send+Sync>,
    bpusher:BPM,astore:Aux,
    executor: &impl futures03::task::Spawn,
  ) -> (Self, impl futures03::future::Future<Output = ()> + Send + Unpin)
  {
    let validator = BadgerGossipValidator::new(keystore, service.local_id().clone(), config.batch_size.into(), persist,client,flizer,bpusher,astore);
    let validator_arc = Arc::new(validator);
    let engine=RantingEngine::new(service, executor,HBBFT_ENGINE_ID, validator_arc.clone() );
   
    let bridge = NetworkBridge {
      engine: engine,
      node: validator_arc,
    };

    let startup_work = futures03::future::lazy(move |_| {
      // lazily spawn these jobs onto their own tasks. the lazy future has access
      // to tokio globals, which aren't available outside.
      //	let mut executor = tokio_executor::DefaultExecutor::current();
      //	executor.spawn(Box::new(reporting_job.select(on_exit.clone()).then(|_| Ok(()))))
      //		.expect("failed to spawn grandpa reporting job task");
      ()
    });

    (bridge, startup_work)
  }
  pub fn is_validator(&self) -> bool
  {
    self.node.is_validator()
  }

}

impl<B: BlockT, Cl,BPM,Aux> Clone for NetworkBridge<B, Cl,BPM,Aux>
where
Cl:NetClient<B>,
B::Hash:Ord,
Aux:AuxStore,
BPM:BlockPusherMaker<B>,
{
  fn clone(&self) -> Self
  {
    NetworkBridge {
      engine: self.engine.clone(),
      node: Arc::clone(&self.node),
    }
  }
}

pub trait BadgerHandler<B:BlockT>
{
  fn vote_for_validators<Be: AuxStore>(&self, auths: Vec<AuthorityId>, backend: &Be) -> Result<(), Error>;
  fn vote_change_encryption_schedule(&self, e: EncryptionSchedule) -> Result<(), Error>;
  fn notify_node_set(&self, vc: Vec<NodeId>); // probably not too important, though ve might want to make sure nodea are connected
 
  fn update_validators<Be: AuxStore>(&self, new_validators: BTreeMap<NodeId, AuthorityId>, backend: &Be);
  fn emit_justification(&self,hash:&B::Hash,auth_list:AuthorityList);
  fn get_current_authorities(&self) -> AuthorityList;
  fn assign_finalizer(&self,fnl:Box<dyn FnMut( &B::Hash,Option<Justification>)->bool+Send+Sync>);
  
}

pub struct BadgerStream<Block:BlockT,Cl,BPM,Aux>
where
Cl:NetClient<Block>,

Block::Hash:Ord,
Aux:AuxStore+Send+Sync+'static,
BPM:BlockPusherMaker<Block>,
{
  pub wrap:Arc< NetworkBridge<Block,Cl,BPM,Aux>>,
}


impl<Block: BlockT,Cl,BPM,Aux> Stream for BadgerStream<Block,Cl,BPM,Aux>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
Aux:AuxStore+Send+Sync+'static,
BPM:BlockPusherMaker<Block>,
{
  type Item = <QHB as ConsensusProtocol>::Output;

  fn poll_next(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>>
  {
    match self.wrap.node.pop_output()
    {
      Some(data) => Poll::Ready(Some(data)),
      None => Poll::Pending,
    }
  }
}


pub trait SendOut
{
  fn send_out(&self, input: Vec<Vec<u8>>) -> Result<(), Error>;
}


impl<Block: BlockT, Cl,BPM,Aux> SendOut for NetworkBridge<Block, Cl,BPM,Aux>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
Aux:AuxStore+Send+Sync+'static,
BPM:BlockPusherMaker<Block>,

{
  fn send_out(&self, input: Vec<Vec<u8>>) -> Result<(), Error>
  {
    for tx in input.into_iter().enumerate()
    {
      
      match  self.engine.with_lock(|en| self.node.push_transaction(tx.1,en) )
      {
        Ok(_) =>
        {}
        Err(e) => return Err(e),
      }
    }
    Ok(())
  }
}

use runtime_primitives::ConsensusEngineId;



impl<Block: BlockT, Cl,BPM,Aux> Sink<Vec<Vec<u8>>> for NetworkBridge<Block, Cl,BPM,Aux>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
Aux:AuxStore+Send+Sync+'static,
BPM:BlockPusherMaker<Block>,
{
  type Error = Error;

  fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>>
  {
    Poll::Ready(Ok(()))
  }

  fn start_send(self: Pin<&mut Self>, input: Vec<Vec<u8>>) -> Result<(), Self::Error>
  {
    for tx in input.into_iter().enumerate()
    {
      //let locked = &self.node;
      match  self.engine.with_lock(|en| { self.node.push_transaction(tx.1,en)}  )
      {
        Ok(_) =>
        {}
        Err(e) => return Err(e),
      }
    }
    Ok(())
  }

  fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>>
  {
    Poll::Ready(Ok(()))
  }

  fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>>
  {
    Poll::Ready(Ok(()))
  }
}
