use badger::dynamic_honey_badger::DynamicHoneyBadger;
use badger::queueing_honey_badger::QueueingHoneyBadger;
use badger::sender_queue::{Message as BMessage, SenderQueue};
use badger::sync_key_gen::{Ack, AckFault, AckOutcome, Part, PartOutcome, PubKeyMap, SyncKeyGen};
use keystore::KeyStorePtr;
//use runtime_primitives::app_crypto::RuntimeAppPublic;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
//use std::convert::TryInto;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

//use badger::dynamic_honey_badger::KeyGenMessage::Ack;
use crate::aux_store::BadgerPersistentData;
use badger::crypto::{PublicKey, PublicKeySet,  SecretKey, SecretKeyShare, };//PublicKeyShare, Signature
use badger::{ConsensusProtocol, CpStep, NetworkInfo, Target};
use futures03::channel::{mpsc, oneshot};
use futures03::prelude::*;
use futures03::{task::Context, task::Poll};
//use hex_fmt::HexFmt;
use log::{debug, info, trace, warn};
use parity_codec::{Decode, Encode};
use parking_lot::RwLock;
use rand::{rngs::OsRng, Rng};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
//
//};//
pub use badger_primitives::HBBFT_ENGINE_ID;
use badger_primitives::{AuthorityId, AuthorityList, AuthorityPair};
use gossip::{ BadgeredMessage, GossipMessage, Peers, SAction, SessionData, SessionMessage};
use network::config::Roles;
use network::consensus_gossip::MessageIntent;
use network::consensus_gossip::ValidatorContext;
use network::PeerId;
use network::{consensus_gossip as network_gossip, NetworkService};
use network_gossip::ConsensusMessage;
//use runtime_primitives::app_crypto::hbbft_thresh::Public as bPublic;
use runtime_primitives::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use substrate_primitives::crypto::Pair;
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};

pub mod gossip;

use crate::Error;

mod peerid;
pub use peerid::PeerIdW;

//use badger_primitives::NodeId;

//use badger::{SourcedMessage as BSM,  TargetedMessage};
pub type NodeId = PeerIdW; //session index? -> might be difficult. but it looks like nodeId for observer does not matter?  but it does for restarts

pub trait Network<Block: BlockT>: Clone + Send + 'static
{
  type In: Stream<Item = network_gossip::TopicNotification>;

  fn messages_for(&self, topic: Block::Hash) -> Self::In;

  fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<Block>>);

  fn gossip_message(&self, topic: Block::Hash, data: Vec<u8>, force: bool);

  fn register_gossip_message(&self, topic: Block::Hash, data: Vec<u8>);

  fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>);

  fn report(&self, who: network::PeerId, cost_benefit: i32);

  fn announce(&self, block: Block::Hash, associated_data: Vec<u8>);

  fn local_id(&self) -> PeerId;
}

pub fn badger_topic<B: BlockT>() -> B::Hash
{
  <<B::Header as HeaderT>::Hashing as HashT>::hash(format!("badger-mushroom").as_bytes())
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum LocalTarget
{
  /// The message must be sent to the node with the given ID.
  Nodes(BTreeSet<NodeId>),
  /// The message must be sent to all remote nodes except the passed nodes.
  /// Useful for sending messages to observer nodes that aren't
  /// present in a node's `all_ids()` list.
  AllExcept(BTreeSet<NodeId>),
}

impl From<Target<NodeId>> for LocalTarget
{
  fn from(t: Target<NodeId>) -> Self
  {
    match t
    {
      // Target::All => LocalTarget::All,
      Target::Nodes(n) => LocalTarget::Nodes(n),
      Target::AllExcept(set) => LocalTarget::AllExcept(set), //Target::AllExcept(set)=>LocalTarget::AllExcept(set.iter().map(|n| *n).collect())
    }
  }
}

impl Into<Target<NodeId>> for LocalTarget
{
  fn into(self) -> Target<NodeId>
  {
    match self
    {
      // LocalTarget::All => Target::All,
      LocalTarget::Nodes(n) => Target::Nodes(n),
      LocalTarget::AllExcept(set) => Target::AllExcept(set),
    }
  }
}

#[derive(Eq, PartialEq, Debug, Encode, Decode)]
pub struct SourcedMessage<D: ConsensusProtocol>
where
  D::NodeId: Encode + Decode + Clone,
{
  sender_id: D::NodeId,
  target: LocalTarget,
  message: Vec<u8>,
}

impl rand::distributions::Distribution<PeerIdW> for rand::distributions::Standard
{
  fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> PeerIdW
  {
    PeerIdW(PeerId::random())
  }
}

impl rand::distributions::Distribution<PeerIdW> for PeerIdW
{
  fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> PeerIdW
  {
    PeerIdW(PeerId::random())
  }
}

pub type BadgerTransaction = Vec<u8>;
pub type QHB = SenderQueue<QueueingHoneyBadger<BadgerTransaction, NodeId, Vec<BadgerTransaction>>>;

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
  pub out_queue: VecDeque<SourcedMessage<D>>,
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
  pub remaining_parts: BTreeSet<NodeId>,
  pub is_done: bool,
  //pub ack: Option<(NodeId,AckOutcome)>,
  //pub step:KeyGenStep,
  //pub keyset:PublicKeySet,
  //pub secret_share: Option<SecretKeyShare>,
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
  /// Running genesis keygen
  KeyGen(KeyGenState),
  /// Running completed Badger node
  Badger(BadgerNode<B, D>),

  /// need a Join Plan to become observer
  AwaitingJoinPlan,
}

pub struct BadgerStateMachine<B: BlockT, D>
where
  D: ConsensusProtocol<NodeId = NodeId>, //specialize to avoid some of the confusion
  D::Message: Serialize + DeserializeOwned,
{
  pub state: BadgerState<B, D>,
  pub peers: Peers,
  pub config: SharedConfig,
  pub queued_transactions: Vec<Vec<u8>>,
  pub persistent: BadgerPersistentData,
  pub keystore: KeyStorePtr,
  pub cached_origin: Option<AuthorityPair>,
  pub queued_messages: Vec<(u64, GossipMessage)>,
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

#[derive(Serialize, Deserialize, Debug)]
pub enum SyncKeyGenMessage
{
  Part(Part),
  Ack(Ack),
}



impl<B: BlockT> BadgerStateMachine<B, QHB>
{
  pub fn new(
    keystore: KeyStorePtr, self_peer: PeerId, batch_size: u64, persist: BadgerPersistentData,
  ) -> BadgerStateMachine<B, QHB>
  {
    let  ap: AuthorityPair;
    let  is_ob: bool;

    {
      let aset = persist.authority_set.inner.read();
      ap = keystore
        .read()
        .key_pair_by_type::<AuthorityPair>(&aset.self_id.clone().into(), app_crypto::key_types::HB_NODE)
        .expect("Needs private key to work (bsm:new)");
      is_ob = !aset.current_authorities.contains(&aset.self_id);
      info!("SELFID: {:?} {:?}",&aset.self_id,&ap.public());
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
    }
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
  ) -> Result<CpStep<QHB>, badger::sender_queue::Error<badger::queueing_honey_badger::Error>>
  {
    match &mut self.state
    {
      BadgerState::AwaitingValidators | BadgerState::AwaitingJoinPlan | BadgerState::KeyGen(_) =>
      {
        match self.queue_transaction(tx)
        {
          Ok(_) => Ok(CpStep::<QHB>::default()),
          Err(_) => Err(badger::sender_queue::Error::Apply(
            badger::queueing_honey_badger::Error::HandleMessage(badger::dynamic_honey_badger::Error::UnknownSender),
          )),
        }
      }
      BadgerState::Badger(ref mut node) => node.push_transaction(tx),
    }
  }

  pub fn proceed_to_badger(&mut self) -> Vec<(LocalTarget, GossipMessage)>
  {
    if self.cached_origin.is_none()
    {
      self.cached_origin = Some(
        self
          .keystore
          .read()
          .key_pair_by_type::<AuthorityPair>(&self.config.my_auth_id.clone().into(), app_crypto::key_types::HB_NODE)
          .expect("Needs private key to work"),
      );
    }

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
    Vec::new()
  }

  fn processed_part(&mut self, partid: &NodeId)
  {
    match &mut self.state
    {
      BadgerState::KeyGen(ref mut state) =>
      {
        state.remaining_parts.remove(partid);
      }
      _ =>
      {}
    }
  }
  pub fn proceed_to_keygen(&mut self) -> Vec<(LocalTarget, GossipMessage)>
  {
    if self.cached_origin.is_none()
    {
      self.cached_origin = Some(
        self
          .keystore
          .read()
          .key_pair_by_type::<AuthorityPair>(&self.config.my_auth_id.clone().into(), app_crypto::key_types::HB_NODE)
          .expect("Needs private key to work"),
      );
    }

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
            .delete_aux(app_crypto::key_types::HB_NODE, &aset.self_id.encode()).expect("Could not delete keystore");

          true
        }
      }
      Err(_) => true,
    };
    if should_badger
    {
      return self.proceed_to_badger();
    }
    info!("Keygen: should regen :{:?}",&should_regen);
    if should_regen
    {
      let mut rng = rand::rngs::OsRng::new().expect("Could not open OS random number generator.");
      let secr: SecretKey = bincode::deserialize(&self.cached_origin.as_ref().unwrap().to_raw_vec()).unwrap();
      info!("Our secret key : {:?} pub: {:?}",&secr,&secr.public_key());
      let thresh = badger::util::max_faulty(aset.current_authorities.len());
      let val_pub_keys: PubKeyMap<NodeId> = Arc::new(
        aset
          .current_authorities
          .iter()
          .map(|x| {
            (
              if *x==self.cached_origin.as_ref().unwrap().public()
              {
               (std::convert::Into::<PeerIdW>::into(self.config.my_peer_id.clone()),x.clone().into())
              }
              else
              {
              (std::convert::Into::<PeerIdW>::into(self
                .peers
                .inverse
                .get(x)
                .expect("All validators should be mapped at this point")
              .clone())
              ,
              x.clone().into())
            
                 })
          })
          .collect(),
      );
      info!("VAL_PUB {:?} {:?}",&val_pub_keys,&self.config.my_peer_id);
      let (skg, part) = SyncKeyGen::new(
        self.config.my_peer_id.clone().into(),
        secr,
        val_pub_keys.clone(),
        thresh,
        &mut rng,
      )
      .expect("Failed to create SyncKeyGen! ");
      let  state = KeyGenState {
        is_observer: self.config.is_observer,
        keygen: skg,
        part: None,
        threshold: thresh as usize,
        remaining_parts: val_pub_keys.keys().cloned().collect(),
        is_done: false,
      };
      self.state = BadgerState::KeyGen(state);

      return match part
      {
        Some(parted) =>
        {
          let akk;
          if let BadgerState::KeyGen(ref mut state) = &mut self.state
          {
            akk = state
              .keygen
              .handle_part(&self.config.my_peer_id.clone().into(), parted.clone(), &mut rng)
              .expect("Failed handling our own part");
              info!("Handled part ");
          }
          else
          {
            return Vec::new();
          }
          let akk2 = match akk
          {
            PartOutcome::Valid(Some(ack)) =>
            {
              let ack_res;
              self.processed_part(&PeerIdW {
                0: self.config.my_peer_id.clone(),
              });

              if let BadgerState::KeyGen(ref mut state) = &mut self.state
              {
                ack_res = state
                  .keygen
                  .handle_ack(&self.config.my_peer_id.clone().into(), ack.clone())
                  .expect("Failed handing our own ack");
              }
              else
              {
                return Vec::new();
              }

              match ack_res
              {
                AckOutcome::Valid =>
                {}
                AckOutcome::Invalid(fault) => panic!("Invalid own Ack: {:?}", fault),
              }

              GossipMessage::KeygenData(BadgeredMessage::new(
                self.cached_origin.as_ref().unwrap().clone(),
                &bincode::serialize(&SyncKeyGenMessage::Ack(ack)).expect("Serial error"),
              ))
            }
            PartOutcome::Invalid(fault) => panic!("Invalid Own Part: {:?}", fault),
            PartOutcome::Valid(None) =>
            {
              panic!("We are not an observer, so we should send Ack.");
            }
          };
          //broadcast local Part and Ack

          vec![
            (
              LocalTarget::AllExcept(BTreeSet::new()),
              GossipMessage::KeygenData(BadgeredMessage::new(
                self.cached_origin.as_ref().unwrap().clone(),
                &bincode::serialize(&SyncKeyGenMessage::Part(parted)).expect("Serial error"),
              )),
            ),
            (LocalTarget::AllExcept(BTreeSet::new()), akk2),
          ]
        }
        None => Vec::new(),
      };
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

  pub fn process_decoded_message(&mut self, message: &GossipMessage) -> (SAction, Vec<(LocalTarget, GossipMessage)>)
  {
    let cset_id;
    {
      cset_id = self.persistent.authority_set.inner.read().set_id;
    }
    match message
    {
      GossipMessage::Session(ses_msg) =>
      {
      
        info!("Session received from {:?} of {:?}", &ses_msg.ses.peer_id,&ses_msg.ses.session_key);
        if ses_msg.ses.ses_id == cset_id
        //??? needs session versions
        {
          let mut ret_msgs: Vec<(LocalTarget, GossipMessage)> = Vec::new();
          self
            .peers
            .update_id(&ses_msg.ses.peer_id.0, ses_msg.ses.session_key.clone());
          //debug!("Adding session key for {:?} :{:}")
          if let BadgerState::AwaitingValidators = self.state
          {
            let ln = self.persistent.authority_set.inner.read().current_authorities.len();
            let mut cur;
            {
            let iaset=self.persistent.authority_set.inner.read();
            cur=iaset.current_authorities.iter().filter(|&n| self.peers.inverse.contains_key(n)).count();
            if self.is_authority() && !self.peers.inverse.contains_key(&self.cached_origin.as_ref().unwrap().public())
            {
              cur=cur+1;
            }
            }
            info!("Currently {:?} validators of {:?}, {:?} inverses",cur,ln,&self.peers.inverse.len());
            if cur ==  ln
            {
              ret_msgs = self.proceed_to_keygen();
            }
          }
          //repropagate
          return (SAction::PropagateOnce, ret_msgs);
        }
        else if ses_msg.ses.ses_id < cset_id
        {
          //discard
          return (SAction::Discard, Vec::new());
        }
        else if ses_msg.ses.ses_id > cset_id + 1
        {
          //?? cache/propagate?
          return (SAction::Discard, Vec::new());
        }
        else
        {
          return (SAction::QueueRetry(ses_msg.ses.ses_id.into()), Vec::new());
        }
      }
      GossipMessage::KeygenData(wkgen) =>
      {
        info!("Got Keygen message!");
        /*let  kgen:SyncKeyGenMessage= match bincode::deserialize(&wkgen.data)
        {
          Ok(data) =>data,
          Err(_) =>
          {
            info!("Invalid keygen data");
            return (SAction::Discard,Vec::new());
          }
        };*/
        let num_auth = self.persistent.authority_set.inner.read().current_authorities.len();
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
            info!("Unknown originator for {:?}", &wkgen.originator);
            return (SAction::QueueRetry(0), Vec::new());
          }
        };
        info!("Originator: {:?}, Peer: {:?}",&wkgen.originator,&orid);
        if let BadgerState::KeyGen(ref mut step) = &mut self.state
        {
          let k_message: SyncKeyGenMessage = match bincode::deserialize(&wkgen.data)
          {
            Ok(data) => data,
            Err(_) =>
            {
              warn!("Keygen message should be correct");
              return (SAction::Discard, Vec::new());
            }
          };
          match k_message
          {
            SyncKeyGenMessage::Part(part) =>
            {
              info!("New PART");
              let mut rng = OsRng::new().unwrap();
              match step.keygen.handle_part(&orid, part.clone(), &mut rng)
              {
                Ok(outcome) => match outcome
                {
                  PartOutcome::Valid(Some(ack)) =>
                  {
                    // send ack out: TODO
                    self.processed_part(&orid);
                    (
                      SAction::PropagateOnce,
                      vec![(
                        LocalTarget::AllExcept(BTreeSet::new()),
                        GossipMessage::KeygenData(BadgeredMessage::new(
                          self.cached_origin.as_ref().unwrap().clone(),
                          &bincode::serialize(&SyncKeyGenMessage::Ack(ack.clone())).expect("Serialize Ack error"),
                        )),
                      )],
                    )
                  }
                  PartOutcome::Invalid(fault) =>
                  {
                    panic!("Invalid Part: {:?}", fault);
                  }
                  PartOutcome::Valid(None) =>
                  {
                    info!("We are an observer, or this part is duplicate {:?}", &part);
                    (SAction::PropagateOnce, Vec::new())
                  }
                },
                Err(e) =>
                {
                  info!("Error handling message {:?}", e);
                  return (SAction::Discard, Vec::new());
                }
              }
            }
            SyncKeyGenMessage::Ack(ack) =>
            {
              info!("New ACK");
              if step.remaining_parts.len() > 0
              {
                info!("Still need some parts? ");
              }
              match step.keygen.handle_ack(&orid, ack.clone()).expect("Failed handing ack")
              {
                AckOutcome::Valid =>
                {
                  if step.keygen.count_complete() == num_auth
                  {
                    info!("Initial keygen ready, generating... ");
                    info!("Pub keys: {:?}",step.keygen.public_keys());
                    let (kset, shr) = step.keygen.generate().expect("Initial key generation failed!");
                   
                    let aux=BadgerAuxCrypto{
                      secret_share: match shr
                      {
                          Some(ref sec) => {
                            info!("Gensec {:?}",sec.public_key_share());
                            Some(SerdeSecret(sec.clone()))},

                          None =>None
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
                     .insert_aux_by_type(app_crypto::key_types::HB_NODE, &self.persistent.authority_set.inner.read().self_id.encode(), &aux).expect("Couldn't save keys");
                    }
                  
                    (SAction::PropagateOnce, self.proceed_to_badger())
                  }
                  else
                  {
                    return (SAction::Discard, Vec::new());
                  }
                  //consumed message
                  //
                }
                AckOutcome::Invalid(fault) =>
                {
                  info!("Invalid  Ack: {:?}", fault);
                  if fault == AckFault::MissingPart
                  {
                    return (SAction::QueueRetry(1), Vec::new());
                  }
                  else
                  {
                    return (SAction::Discard, Vec::new());
                  }
                }
              }
            }
          }
        }
        else
        {
          info!("Keygen data received while out of keygen");
          // propagate once?
          return (SAction::PropagateOnce, Vec::new());
        }
      }
      GossipMessage::BadgerData(bdat) =>
      {
        let orid: PeerIdW = match self.peers.inverse.get(&bdat.originator)
        {
          Some(dat) => dat.clone().into(),
          None =>
          {
            info!("Unknown originator for {:?}", &bdat.originator);
            return (SAction::QueueRetry(0), Vec::new());
          }
        };
        if !self
          .persistent
          .authority_set
          .inner
          .read()
          .current_authorities
          .contains(&bdat.originator)
        {
          info!("Got Badger message from non-authority, discarding");
          return (SAction::Discard, Vec::new());
        }
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

                  return (SAction::Discard, Vec::new());
                }
                Err(e) =>
                {
                  info!("Error handling badger message {:?}", e);
                  telemetry!(CONSENSUS_DEBUG; "afg.err_handling_msg"; "err" => ?format!("{}", e));
                  return (SAction::Discard, Vec::new());
                }
              }
            }
            else
            {
              return (SAction::Discard, Vec::new());
            }
          }
          BadgerState::KeyGen(_) | BadgerState::AwaitingValidators =>
          {
            return (SAction::QueueRetry(2), Vec::new());
          }
          _ =>
          {
            warn!("Discarding badger message");
            return (SAction::Discard, Vec::new());
          }
        }
      }
    }
  }
  pub fn process_message(
    &mut self, _who: &PeerId, mut data: &[u8],
  ) -> (SAction, Vec<(LocalTarget, GossipMessage)>, Option<GossipMessage>)
  {
    match GossipMessage::decode(&mut data)
    {
      Ok(message) =>
      {
        if !message.verify()
        {
          warn!("Invalid message signature in {:?}", &message);
          return (SAction::Discard, Vec::new(), None);
        }
        let (a, b) = self.process_decoded_message(&message);
        return (a, b, Some(message));
      }
      Err(e) =>
      {
        info!(target: "afg", "Error decoding message {:?}",e);
        telemetry!(CONSENSUS_DEBUG; "afg.err_decoding_msg"; "" => "");

        (SAction::Discard, Vec::new(), None)
      }
    }
  }
  pub fn process_and_replay(
    &mut self, who: &PeerId,  data: &[u8],
  ) -> (Vec<(SAction, Option<GossipMessage>)>, Vec<(LocalTarget, GossipMessage)>)
  {
    let mut acc: Vec<(LocalTarget, GossipMessage)> = Vec::new();
    let mut vals: Vec<(SAction, Option<GossipMessage>)> = Vec::new();
    let (action, mut repl, msg) = self.process_message(who, data);
    acc.append(&mut repl);
    match action
    {
      SAction::QueueRetry(tp) =>
      {
        //message has not been processed
        match msg
        {
          Some(msg) => self.queued_messages.push((tp, msg)),
          None =>
          {}
        }
        return (vec![(SAction::Discard, None)], acc); //no need to inform upstream, I think
      }
      act =>
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
          info!("Replaying {:?}",silly_compiler.len()
        );
          for (_, msg) in silly_compiler.into_iter()
          //TODO: use the number eventually
          {
            
            let (s_act, mut repl) = self.process_decoded_message(&msg);
            acc.append(&mut repl);
            match s_act
            {
              SAction::QueueRetry(tp) =>
              {
                new_queue.push((tp, msg));
              }
              SAction::Discard =>
              {
                done = false;
              }
              action =>
              {
                vals.push((action.clone(), Some(msg)));
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
        return (vals, acc);
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

pub type BadgerNodeStepResult<D> = CpStep<D>;
pub type TransactionSet = Vec<Vec<u8>>; //agnostic?

impl<B: BlockT, D: ConsensusProtocol<NodeId = NodeId>> BadgerNode<B, D>
where
  D::Message: Serialize + DeserializeOwned,
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
      .map(|x| 
        {
        if x==auth_id {
          self_id.clone().into()
        }
        else
        {  Into::<NodeId>::into(peers.inverse.get(&x).expect("All auths should be known").clone()) } }

        
      )
      .collect();
     vset.sort();
  
    let ni = NetworkInfo::<D::NodeId>::new(self_id.clone().into(), sks, (pkset).clone(), vset);
    //let num_faulty = hbbft::util::max_faulty(indices.len());
    /*let peer_ids: Vec<_> = ni
    .all_ids()
    .filter(|&them| *them != PeerIdW { 0: self_id.clone() })
    .cloned()
    .collect();*/
    let val_map: BTreeMap<NodeId, PublicKey> = validator_set
      .iter()
      .map(|auth| {
        let nid = peers.inverse.get(auth).expect("All authorities should have loaded!");
        (nid.clone().into(), (*auth).clone().into())
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
    let  node = BadgerNode {
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

  pub fn handle_message(&mut self, who: &NodeId, msg: D::Message) -> Result<(), &'static str>
  {
    debug!("BaDGER!! Handling message from {:?} {:?}", who, &msg);
    match self.algo.handle_message(who, msg, &mut self.main_rng)
    {
      Ok(step) =>
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
        debug!("BaDGER!! OK message from {:?}, {} ", who, self.outputs.len());
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
      Err(_) => return Err("Cannot handle message"),
    }
  }
}
pub struct BadgerGossipValidator<Block: BlockT>
{
  // peers: RwLock<Arc<Peers>>,
  inner: RwLock<BadgerStateMachine<Block, QHB>>,
  pending_messages: RwLock<BTreeMap<PeerIdW, Vec<Vec<u8>>>>,
}
impl<Block: BlockT> BadgerGossipValidator<Block>
{
  fn send_message_either<N: Network<Block>>(
    &self, who: &PeerId, vdata: Vec<u8>, context_net: Option<&N>,
    context_val: &mut Option<&mut dyn ValidatorContext<Block>>,
  )
  {
    if let Some(context) = context_net
    {
      context.send_message(vec![who.clone()], vdata);
      return;
    }
    if let Some(context) = context_val
    {
      context.send_message(who, vdata);
    }
  }
  fn flush_message_either<N: Network<Block>>(
    &self, additional: &mut Vec<(LocalTarget, GossipMessage)>, context_net: Option<&N>,
    context_val: &mut Option<&mut dyn ValidatorContext<Block>>,
  )
  {
    // let topic = badger_topic::<Block>();
    let sid: PeerId;
    let pair: AuthorityPair;
    let mut drain: Vec<_> = Vec::new();
    {
      let mut locked = self.inner.write();
      sid = locked.config.my_peer_id.clone();
      pair = locked.cached_origin.clone().unwrap();
      if let BadgerState::Badger(ref mut state) = &mut locked.state
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
    }
    drain.append(additional);
    {
      let mut ldict = self.pending_messages.write();
      let inner = self.inner.read();
      let plist = inner.peers.connected_peer_list();
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
            debug!("BaDGER!! RESending to {:?}", &k);
            self.send_message_either(&k.0, msg, context_net, context_val);
          }
        }
      }
    }
    for (target, msg) in drain
    {
      //let vdata = GossipMessage::BadgerData(BadgeredMessage::new(pair,msg.message)).encode();
      let vdata = msg.encode();
      match target
      {
        LocalTarget::Nodes(node_set) =>
        {
          let inner = self.inner.write();
          trace!("Nodes lock success");
          let peers = &inner.peers;
          let av_list = peers.connected_peer_list();
          for to_id in node_set.iter()
          {
            debug!("BaDGER!! Id_net {:?}", &to_id);

            if av_list.contains(&to_id.0)
            {
              self.send_message_either(&to_id.0, vdata.clone(), context_net, context_val);
            }
            else
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
          let locked = self.inner.write();
          trace!("Allex lock success");
          let peers = &locked.peers;
          let mut vallist: Vec<_> = locked.persistent.authority_set.inner
            .read()
            .current_authorities
            .iter()
            .map(|x| 
              {
                if *x==locked.config.my_auth_id {
                  &locked.config.my_peer_id
                }
                else
                { peers.inverse.get(x).expect("All auths should be known") } }
              )
            .collect();
          for pid in peers
            .connected_peer_list()
            .iter()
            .filter(|n| !exclude.contains(&PeerIdW { 0: (*n).clone() }))
          {
            let tmp = pid.clone();
            if tmp != sid
            {
              self.send_message_either(pid, vdata.clone(), context_net, context_val);
            }
            vallist.retain(|&x| *x != tmp);
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
      }
    }
    debug!("BaDGER!! Exit flush");
  }
  /// Create a new gossip-validator.
  pub fn new(keystore: KeyStorePtr, self_peer: PeerId, batch_size: u64, persist: BadgerPersistentData) -> Self
  {
    Self {
      inner: RwLock::new(BadgerStateMachine::<Block, QHB>::new(
        keystore, self_peer, batch_size, persist,
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

  pub fn push_transaction<N: Network<Block>>(&self, tx: Vec<u8>, net: &N) -> Result<(), Error>
  {
    let mut do_flush = false;
    {
      let mut locked = self.inner.write();
      if let BadgerState::Badger(ref mut node) = &mut locked.state
      {
        do_flush = match node.push_transaction(tx)
        {
          Ok(step) =>
          {
            info!("Push OK");
            let out_msgs: Vec<_> = step
              .messages
              .into_iter()
              .map(|mmsg| {
                info!("BaDGER!! Flushing {:?} ", &mmsg.message);
                let ser_msg = bincode::serialize(&mmsg.message).expect("serialize");
                (mmsg.target, ser_msg)
              })
              .collect();

            node.outputs.extend(step.output.into_iter());
            let cloneid = node.node_id.clone();
            for (target, message) in out_msgs
            {
              node.out_queue.push_back(SourcedMessage {
                sender_id: cloneid.clone(),
                target: target.into(),
                message,
              });
            }

            node.out_queue.len() > 0
          }
          Err(e) => return Err(Error::Badger(e.to_string())),
        }
      }
    }
    //send messages out
    if do_flush
    {
      self.flush_message_either(&mut Vec::new(), Some(net), &mut None);
    }

    Ok(())
  }
}
use gossip::PeerConsensusState;

impl<Block: BlockT> network_gossip::Validator<Block> for BadgerGossipValidator<Block>
{
  fn new_peer(&self, context: &mut dyn ValidatorContext<Block>, who: &PeerId, _roles: Roles)
  {
    info!("New Peer called {:?}", who);
    {
      let mut inner = self.inner.write();
      inner.peers.update_peer_state(who, PeerConsensusState::Connected);
    };
    let packet = {
      let mut inner = self.inner.write();
      inner.peers.new_peer(who.clone());
      if inner.cached_origin.is_none()
      {
        let pair: AuthorityPair;
        {
          pair = inner
            .keystore
            .read()
            .key_pair_by_type::<AuthorityPair>(&inner.config.my_auth_id.clone().into(), app_crypto::key_types::HB_NODE)
            .expect("Needs private key to work");
        }

        inner.cached_origin = Some(pair);
      }

      let ses_mes = SessionData {
        ses_id: inner.persistent.authority_set.inner.read().set_id,
        session_key: inner.cached_origin.as_ref().unwrap().public(),
        peer_id: inner.config.my_peer_id.clone().into(),
      };
      let sgn = inner.cached_origin.as_ref().unwrap().sign(&ses_mes.encode());
      SessionMessage { ses: ses_mes, sgn: sgn }
    };

    //if let Some(packet) = packet {
    let packet_data = GossipMessage::Session(packet).encode();
    context.send_message(who, packet_data);
    //}
  }

  fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<Block>, who: &PeerId)
  {
    self.inner.write().peers.peer_disconnected(who);
  }

  fn validate(
    &self, context: &mut dyn ValidatorContext<Block>, who: &PeerId, data: &[u8],
  ) -> network_gossip::ValidationResult<Block::Hash>
  {
    info!("Enter validate");
    let topic = badger_topic::<Block>();
    let (actions, mut accumulated_output) = self.inner.write().process_and_replay(who, data);

    let mut ret = network_gossip::ValidationResult::ProcessAndDiscard(topic);
    if accumulated_output.len() == 0
    //whatever? does this make any sense?
    {
      ret = network_gossip::ValidationResult::Discard;
    }
    let mut keep: bool = false; //a hack. TODO?

    for (action, msg) in actions.into_iter()
    {
      match action
      {
        SAction::Discard =>
        {}
        SAction::PropagateOnce =>
        {
          if let Some(msg) = msg
          {
            context.broadcast_message(topic, msg.encode().to_vec(), false);
          }
        }

        SAction::RepropagateKeep =>
        {
          /*if let Some(msg)=msg
          {
           let cmsg = ConsensusMessage {
            engine_id: HBBFT_ENGINE_ID,
            data: msg.encode().to_vec(),
             };
           context.register_gossip_message(topic, cmsg);
           }*/
          //ret=network_gossip::ValidationResult::ProcessAndKeep;
          keep = true;
        }
        SAction::QueueRetry(num) =>
        {
          //shouldn't reach here normally
          info!("Unexpected SAction::QueueRetry {:?}", num);
        }
      }
    }
    info!("Sending topic");
    context.send_topic(who, topic, false);

    {
      self.flush_message_either::<ShutUp>(&mut accumulated_output, None, &mut Some(context));
    }
    info!("Flushed");
    if keep
    {
      ret = network_gossip::ValidationResult::ProcessAndKeep(topic);
    }
    return ret;
  }

  fn message_allowed<'b>(&'b self) -> Box<dyn FnMut(&PeerId, MessageIntent, &Block::Hash, &[u8]) -> bool + 'b>
  {
    info!("MessageAllowed 2");
    Box::new(move |who, intent, topic, mut data| {
      // if the topic is not something we're keeping at the moment,
      // do not send.
      if *topic != badger_topic::<Block>()
      //only one topic, i guess we may add epochs eventually
      {
        return false;
      }

      if let MessageIntent::PeriodicRebroadcast = intent
      {
        return true; //might be useful
      }

      let _peer = match self.inner.read().peers.inner.get(who)
      {
        None => return false,
        Some(x) => x,
      };

      match GossipMessage::decode(&mut data)
      {
        Err(_) => false,
        Ok(GossipMessage::BadgerData(_)) => return true,
        Ok(GossipMessage::KeygenData(_)) => return true,
        Ok(GossipMessage::Session(_)) => true,
      }
    })
  }

  fn message_expired<'b>(&'b self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'b>
  {
    //let inner = self.inner.read();
    Box::new(move |topic, mut data| {
      //only one topic, i guess we may add epochs eventually
      if topic != badger_topic::<Block>()
      {
        return true;
      }

      match GossipMessage::decode(&mut data)
      {
        Err(_) => true,
        Ok(GossipMessage::Session(_)) => false,
        Ok(_) => true,
      }
    })
  }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
struct ShutUp;

impl<B> Network<B> for ShutUp
where
  B: BlockT,
{
  type In = NetworkStream;
  fn local_id(&self) -> PeerId
  {
    PeerId::random()
  }

  fn messages_for(&self, _topic: B::Hash) -> Self::In
  {
    let (_tx, rx) = oneshot::channel::<mpsc::UnboundedReceiver<_>>();
    NetworkStream { outer: rx, inner: None }
  }

  fn register_validator(&self, _validator: Arc<dyn network_gossip::Validator<B>>) {}

  fn gossip_message(&self, _topic: B::Hash, _data: Vec<u8>, _force: bool) {}

  fn register_gossip_message(&self, _topic: B::Hash, _data: Vec<u8>) {}

  fn send_message(&self, _who: Vec<network::PeerId>, _data: Vec<u8>) {}

  fn report(&self, _who: network::PeerId, _cost_benefit: i32) {}

  fn announce(&self, _block: B::Hash, _data: Vec<u8>) {}
}

impl<B, S, H> Network<B> for Arc<NetworkService<B, S, H>>
where
  B: BlockT,
  S: network::specialization::NetworkSpecialization<B>,
  H: network::ExHashT,
{
  type In = NetworkStream;
  fn local_id(&self) -> PeerId
  {
    self.local_peer_id()
  }

  fn messages_for(&self, topic: B::Hash) -> Self::In
  {
    let (tx, rx) = oneshot::channel::<mpsc::UnboundedReceiver<_>>();
    self.with_gossip(move |gossip, _| {
      let inner_rx = gossip.messages_for(HBBFT_ENGINE_ID, topic);
      let _ = tx.send(inner_rx);
    });
    NetworkStream { outer: rx, inner: None }
  }

  fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<B>>)
  {
    self.with_gossip(move |gossip, context| gossip.register_validator(context, HBBFT_ENGINE_ID, validator))
  }

  fn gossip_message(&self, topic: B::Hash, data: Vec<u8>, force: bool)
  {
    let msg = ConsensusMessage {
      engine_id: HBBFT_ENGINE_ID,
      data,
    };

    self.with_gossip(move |gossip, ctx| gossip.multicast(ctx, topic, msg, force))
  }

  fn register_gossip_message(&self, topic: B::Hash, data: Vec<u8>)
  {
    let msg = ConsensusMessage {
      engine_id: HBBFT_ENGINE_ID,
      data,
    };

    self.with_gossip(move |gossip, _| gossip.register_message(topic, msg))
  }

  fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>)
  {
    let msg = ConsensusMessage {
      engine_id: HBBFT_ENGINE_ID,
      data,
    };

    self.with_gossip(move |gossip, ctx| {
      for who in &who
      {
        gossip.send_message(ctx, who, msg.clone())
      }
    })
  }

  fn report(&self, who: network::PeerId, cost_benefit: i32)
  {
    self.report_peer(who, cost_benefit)
  }

  fn announce(&self, block: B::Hash, associated_data: Vec<u8>)
  {
    info!("Announcing block!");
    self.announce_block(block, associated_data)
  }
}

/// A stream used by NetworkBridge in its implementation of Network.
pub struct NetworkStream
{
  inner: Option<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>,
  outer: oneshot::Receiver<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>,
}

impl Stream for NetworkStream
{
  type Item = network_gossip::TopicNotification;

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

/// Bridge between the underlying network service, gossiping consensus messages and Grandpa
pub struct NetworkBridge<B: BlockT, N: Network<B>>
{
  service: N,
  node: Arc<BadgerGossipValidator<B>>,
}

impl<B: BlockT, N: Network<B>> NetworkBridge<B, N>
{
  /// Create a new NetworkBridge to the given NetworkService. Returns the service
  /// handle and a future that must be polled to completion to finish startup.
  /// If a voter set state is given it registers previous round votes with the
  /// gossip service.
  pub fn new(
    service: N, config: crate::Config, keystore: KeyStorePtr, persist: BadgerPersistentData,
  ) -> (Self, impl futures03::future::Future<Output = ()> + Send + Unpin)
  {
    let validator = BadgerGossipValidator::new(keystore, service.local_id().clone(), config.batch_size.into(), persist);
    let validator_arc = Arc::new(validator);
    service.register_validator(validator_arc.clone());

    let bridge = NetworkBridge {
      service,
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

  /// Set up the global communication streams. blocks out transaction in. Maybe reverse of grandpa...
  pub fn global_communication(
    &self, is_voter: bool,
  ) -> (impl Stream<Item = <QHB as ConsensusProtocol>::Output>, impl SendOut)
  {
    let incoming = incoming_global::<B, N>(self.node.clone());

    let outgoing = TransactionFeed::<B, N>::new(self.service.clone(), is_voter, self.node.clone());

    (incoming, outgoing)
  }
}

fn incoming_global<B: BlockT, N: Network<B>>(
  gossip_validator: Arc<BadgerGossipValidator<B>>,
) -> impl Stream<Item = <QHB as ConsensusProtocol>::Output>
{
  BadgerStream::new(gossip_validator.clone())
}

impl<B: BlockT, N: Network<B>> Clone for NetworkBridge<B, N>
{
  fn clone(&self) -> Self
  {
    NetworkBridge {
      service: self.service.clone(),
      node: Arc::clone(&self.node),
    }
  }
}

pub struct BadgerStream<Block: BlockT>
{
  validator: Arc<BadgerGossipValidator<Block>>,
}

impl<Block: BlockT> BadgerStream<Block>
{
  fn new(gossip_validator: Arc<BadgerGossipValidator<Block>>) -> Self
  {
    BadgerStream {
      validator: gossip_validator,
    }
  }
}

impl<Block: BlockT> Stream for BadgerStream<Block>
{
  type Item = <QHB as ConsensusProtocol>::Output;

  fn poll_next(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>>
  {
    match self.validator.pop_output()
    {
      Some(data) => Poll::Ready(Some(data)),
      None => Poll::Pending,
    }
  }
}

/// An output sink for commit messages.
struct TransactionFeed<Block: BlockT, N: Network<Block>>
{
  network: N,
  gossip_validator: Arc<BadgerGossipValidator<Block>>,
}

pub trait SendOut
{
  fn send_out(&mut self, input: Vec<Vec<u8>>) -> Result<(), Error>;
}

impl<Block: BlockT, N: Network<Block>> TransactionFeed<Block, N>
{
  pub fn new(network: N, _is_voter: bool, gossip_validator: Arc<BadgerGossipValidator<Block>>) -> Self
  {
    TransactionFeed {
      network,
      // is_voter,
      gossip_validator,
    }
  }
}
impl<Block: BlockT, N: Network<Block>> SendOut for TransactionFeed<Block, N>
{
  fn send_out(&mut self, input: Vec<Vec<u8>>) -> Result<(), Error>
  {
    for tx in input.into_iter().enumerate()
    {
      let locked = &self.gossip_validator;
      match locked.push_transaction(tx.1, &self.network)
      {
        Ok(_) =>
        {}
        Err(e) => return Err(e),
      }
    }
    Ok(())
  }
}

use network::consensus_gossip::ConsensusGossip;
use runtime_primitives::ConsensusEngineId;

struct NetworkSubtext<'g, 'p, B: BlockT>
{
  gossip: &'g mut ConsensusGossip<B>,
  protocol: &'p mut dyn network::Context<B>,
  engine_id: ConsensusEngineId,
}

impl<'g, 'p, B: BlockT> ValidatorContext<B> for NetworkSubtext<'g, 'p, B>
{
  fn broadcast_topic(&mut self, topic: B::Hash, force: bool)
  {
    self.gossip.broadcast_topic(self.protocol, topic, force);
  }

  fn broadcast_message(&mut self, topic: B::Hash, message: Vec<u8>, force: bool)
  {
    info!("mcast");
    self.gossip.multicast(
      self.protocol,
      topic,
      ConsensusMessage {
        data: message,
        engine_id: self.engine_id.clone(),
      },
      force,
    );
  }

  fn send_message(&mut self, who: &PeerId, message: Vec<u8>)
  {
    self.protocol.send_consensus(
      who.clone(),
      vec![ConsensusMessage {
        engine_id: self.engine_id,
        data: message,
      }],
    );
  }

  fn send_topic(&mut self, who: &PeerId, topic: B::Hash, force: bool)
  {
    self.gossip.send_topic(self.protocol, who, topic, self.engine_id, force);
  }
}

impl<Block: BlockT, N: Network<Block>> Sink<Vec<Vec<u8>>> for TransactionFeed<Block, N>
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
      let locked = &self.gossip_validator;
      match locked.push_transaction(tx.1, &self.network)
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
