use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::convert::TryInto;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use badger::dynamic_honey_badger::DynamicHoneyBadger;
use badger::queueing_honey_badger::QueueingHoneyBadger;
use badger::sender_queue::{Message as BMessage, SenderQueue};
use badger::sync_key_gen::{to_pub_keys, PartOutcome, SyncKeyGen};
use badger::{ConsensusProtocol, CpStep, NetworkInfo, Target};
use futures03::channel::{mpsc, oneshot};
use futures03::prelude::*;
use futures03::{task::Context, task::Poll};
use hex_fmt::HexFmt;
use log::{debug, info, trace};
use parity_codec::{Decode, Encode};
use parking_lot::RwLock;
use rand::{rngs::OsRng, Rng};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use badger_primitives::AuthorityId;
pub use badger_primitives::HBBFT_ENGINE_ID;
use badger_primitives::{PublicKeyWrap, SignatureWrap};
use gossip::{Action, BadgeredMessage, GossipMessage, GreetingMessage, Peers};
use network::config::Roles;
use network::consensus_gossip::MessageIntent;
use network::consensus_gossip::ValidatorContext;
use network::PeerId;
use network::{consensus_gossip as network_gossip, NetworkService};
use network_gossip::ConsensusMessage;
use runtime_primitives::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};

pub mod gossip;

use crate::Error;

mod peerid;
pub use peerid::PeerIdW;

// #[cfg(test)]
// mod tests;

//use badger::{SourcedMessage as BSM,  TargetedMessage};

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
  /// The message must be sent to all remote nodes.
  // All,
  /// The message must be sent to the node with the given ID.
  Nodes(BTreeSet<PeerIdW>),
  /// The message must be sent to all remote nodes except the passed nodes.
  /// Useful for sending messages to observer nodes that aren't
  /// present in a node's `all_ids()` list.
  AllExcept(BTreeSet<PeerIdW>),
}

impl From<Target<PeerIdW>> for LocalTarget
{
  fn from(t: Target<PeerIdW>) -> Self
  {
    match t
    {
      // Target::All => LocalTarget::All,
      Target::Nodes(n) => LocalTarget::Nodes(n),
      Target::AllExcept(set) => LocalTarget::AllExcept(set), //Target::AllExcept(set)=>LocalTarget::AllExcept(set.iter().map(|n| *n).collect())
    }
  }
}

impl Into<Target<PeerIdW>> for LocalTarget
{
  fn into(self) -> Target<PeerIdW>
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
struct SourcedMessage<D: ConsensusProtocol>
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
pub type QHB = SenderQueue<QueueingHoneyBadger<BadgerTransaction, PeerIdW, Vec<BadgerTransaction>>>;

pub struct BadgerNode<B: BlockT, D>
where
  D: ConsensusProtocol<NodeId = PeerIdW>, //specialize to avoid some of the confusion
  D::Message: Serialize + DeserializeOwned,
{
  id: PeerId,
  algo: D,
  main_rng: OsRng,

  peers: Peers,
  authorities: Vec<AuthorityId>,
  config: crate::Config,
  //next_rebroadcast: Instant,
  /// Incoming messages from other nodes that this node has not yet handled.
  // in_queue: VecDeque<SourcedMessage<D>>,
  /// Outgoing messages to other nodes.
  out_queue: VecDeque<SourcedMessage<D>>,
  /// The values this node has output so far, with timestamps.
  outputs: VecDeque<D::Output>,
  _block: PhantomData<B>,
}

impl<B: BlockT> BadgerNode<B, QHB>
{
  fn push_transaction(
    &mut self,
    tx: Vec<u8>,
  ) -> Result<CpStep<QHB>, badger::sender_queue::Error<badger::queueing_honey_badger::Error>>
  {
    info!("BaDGER pushing transaction {:?}", &tx);
    let ret = self.algo.push_transaction(tx, &mut self.main_rng);
    info!("BaDGER pushed: complete ");
    ret
  }
}

impl<B: BlockT, D: ConsensusProtocol<NodeId = PeerIdW>> BadgerNode<B, D>
where
  D::Message: Serialize + DeserializeOwned,
{
  fn register_peer_public_key(&mut self, who: &PeerId, auth: AuthorityId)
  {
    self.peers.update_id(who, auth)
  }

  fn is_authority(&self, who: &PeerId) -> bool
  {
    trace!("BaDGER!! IsAuth {:?}", who);
    let auth = self.peers.peer(who);
    match auth
    {
      Some(info) =>
      {
        if let Some(iid) = &info.id
        {
          trace!("BaDGER!! SomeInfo {:?} {}", &iid, self.authorities.len());

          self.authorities.contains(&iid)
        }
        else
        {
          trace!("BaDGER!! ZeroInfo {:?}", &info.id);
          false
        }
      }
      None =>
      {
        info!("BaDGER!! NoInfo {:?}", who);
        false
      }
    }
  }
}

pub type BadgerNodeStepResult<D> = CpStep<D>;
pub type TransactionSet = Vec<Vec<u8>>; //agnostic?
impl<B: BlockT, D: ConsensusProtocol<NodeId = PeerIdW>> BadgerNode<B, D>
where
  D::Message: Serialize + DeserializeOwned,
{
  pub fn new(config: crate::Config, self_id: PeerId) -> BadgerNode<B, QHB>
  {
    let mut rng = OsRng::new().unwrap();
    let secr = match config.secret_key_share.clone()
    {
      Some(wrap) => Some(wrap.0.clone()),
      None => None,
    };
    let ni = NetworkInfo::<D::NodeId>::new(
      PeerIdW { 0: self_id.clone() },
      secr,
      config.public_key_set.0.clone(),
      //config.node_id.1.clone(),
      config.initial_validators.clone().keys(),
      //config.node_indices.clone(),
    );
    //let ni=NetworkInfo::<D::NodeId>::new(PeerIdW{ 0: self_id.clone() },secr,config.public_key_set.0.clone(),config.node_id.1.clone(),config.initial_validators.clone());
    // for (k, v) in ni.public_key_share_map().clone().into_iter()
    // {
    //    info!("JSON+ {:?} {:?} ", &k, &v);
    // }

    let peer_ids: Vec<_> = ni
      .all_ids()
      .filter(|&them| *them != PeerIdW { 0: self_id.clone() })
      .cloned()
      .collect();
    let dhb = DynamicHoneyBadger::builder().build(
      ni,
      config.node_id.1.clone(),
      Arc::new(config.initial_validators.clone()),
    );
    let (qhb, qhb_step) = QueueingHoneyBadger::builder(dhb)
      .batch_size(config.batch_size.try_into().unwrap())
      .build(&mut rng)
      .expect("instantiate QueueingHoneyBadger");

    let (sq, mut step) =
      SenderQueue::builder(qhb, peer_ids.into_iter()).build(PeerIdW { 0: self_id.clone() });
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
    let mut node = BadgerNode {
      id: self_id,
      algo: sq,
      main_rng: rng,
      peers: Peers::new(),
      authorities: config
        .initial_validators
        .clone()
        .iter()
        .map(|(_, val)| PublicKeyWrap { 0: *val })
        .collect(),
      config: config.clone(),
      //in_queue: VecDeque::new(),
      out_queue: out_queue,
      outputs: outputs,
      _block: PhantomData,
    };
    for (k, v) in config.initial_validators.clone()
    {
      info!("BaDGER!! Registering {:?} {:?}", &k.0, &v);
      node.register_peer_public_key(&k.0, PublicKeyWrap { 0: v })
    }
    node
  }

  pub fn handle_message(&mut self, who: &PeerIdW, msg: D::Message) -> Result<(), &'static str>
  {
    debug!("BaDGER!! Handling message from {} {:?}", who.0, &msg);
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
        debug!(
          "BaDGER!! OK message from {}, {} ",
          who.0,
          self.outputs.len()
        );
        for (target, message) in out_msgs
        {
          self.out_queue.push_back(SourcedMessage {
            sender_id: PeerIdW { 0: self.id.clone() },
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
  inner: RwLock<BadgerNode<Block, QHB>>,
  pending_messages: RwLock<BTreeMap<PeerIdW, Vec<Vec<u8>>>>,
}
impl<Block: BlockT> BadgerGossipValidator<Block>
{
  fn send_message_either<N: Network<Block>>(
    &self,
    who: PeerId,
    vdata: Vec<u8>,
    context_net: Option<&N>,
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
      context.send_message(&who, vdata);
    }
  }

  fn flush_message_either<N: Network<Block>>(
    &self,
    context_net: Option<&N>,
    context_val: &mut Option<&mut dyn ValidatorContext<Block>>,
  )
  {
    // let topic = badger_topic::<Block>();
    let sid: PeerId;
    let drain: Vec<_>;
    {
      let mut locked = self.inner.write();
      sid = locked.id.clone();
      debug!("BaDGER!! Flushing {} messages_net", &locked.out_queue.len());
      drain = locked.out_queue.drain(..).collect();
    }

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
            self.send_message_either(k.0.clone(), msg, context_net, context_val);
          }
        }
      }
    }
    for msg in drain
    {
      let uuid = OsRng::new().unwrap().gen::<u64>();
      debug!("Sending_ with uid: {} {}", &uuid, HexFmt(&msg.message));
      let vdata = GossipMessage::BadgerData(BadgeredMessage {
        uid: uuid,
        originator: PeerIdW { 0: sid.clone() },
        data: msg.message,
      })
      .encode();

      match &msg.target
      {
        LocalTarget::Nodes(node_set) =>
        {
          let inner = self.inner.write();
          let av_list = inner.peers.connected_peer_list();
          for to_id in node_set.iter()
          {
            debug!("BaDGER!! Id_net {}", &to_id.0);

            if av_list.contains(&to_id.0)
            {
              self.send_message_either(to_id.0.clone(), vdata.clone(), context_net, context_val);
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
          let mut vallist: Vec<_> = locked
            .config
            .initial_validators
            .keys()
            .filter(|n| !exclude.contains(&n))
            .collect();
          for pid in locked
            .peers
            .connected_peer_list()
            .iter()
            .filter(|n| !exclude.contains(&PeerIdW { 0: (*n).clone() }))
          {
            let tmp = PeerIdW { 0: pid.clone() };
            if tmp != msg.sender_id
            {
              self.send_message_either(pid.clone(), vdata.clone(), context_net, context_val);
            }
            vallist.retain(|&x| *x != tmp);
          }

          if vallist.len() > 0
          {
            let mut ldict = self.pending_messages.write();
            for val in vallist.into_iter()
            {
              let stat = ldict.entry(val.clone()).or_insert(Vec::new());
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
  pub fn new(config: crate::Config, self_id: PeerId) -> Self
  {
    Self {
      inner: RwLock::new(BadgerNode::<Block, QHB>::new(config, self_id)),
      pending_messages: RwLock::new(BTreeMap::new()),
    }
  }
  /// collect outputs from
  pub fn pop_output(&self) -> Option<<QHB as ConsensusProtocol>::Output>
  {
    let mut locked = self.inner.write();
    info!("OUTPUTS: {:?}", locked.outputs.len());
    locked.outputs.pop_front()
  }

  pub fn is_validator(&self) -> bool
  {
    let rd = self.inner.read();
    rd.is_authority(&rd.id)
  }

  pub fn push_transaction<N: Network<Block>>(&self, tx: Vec<u8>, net: &N) -> Result<(), Error>
  {
    let do_flush;
    {
      let mut locked = self.inner.write();
      do_flush = match locked.push_transaction(tx)
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

          locked.outputs.extend(step.output.into_iter());
          let cloneid = locked.id.clone();
          for (target, message) in out_msgs
          {
            locked.out_queue.push_back(SourcedMessage {
              sender_id: PeerIdW { 0: cloneid.clone() },
              target: target.into(),
              message,
            });
          }

          locked.out_queue.len() > 0
        }
        Err(e) => return Err(Error::Badger(e.to_string())),
      }
    }
    //send messages out
    if do_flush
    {
      self.flush_message_either(Some(net), &mut None);
    }

    Ok(())
  }

  pub fn do_validate(
    &self,
    who: &PeerId,
    mut data: &[u8],
  ) -> (Action<Block::Hash>, Option<GossipMessage>)
  {
    let mut peer_reply = None;

    let action = {
      match GossipMessage::decode(&mut data)
      {
        Ok(GossipMessage::Greeting(msg)) =>
        {
          if msg
            .my_id
            .0
            .verify(&msg.my_sig.0, msg.my_id.0.to_bytes().to_vec())
          {
            info!(
              "BadGER: got Greeting {:?} {:?} {:?}",
              &who, &msg.my_id, msg.my_pubshare
            );
            if let Some(_) = msg.my_pubshare
            {
              //self.inner.write().register_peer_public_key(who,share); TODO : maybe fix?
            }
            let mut inner = self.inner.write();
            inner
              .peers
              .update_peer_state(who, PeerConsensusState::GreetingReceived);
            Action::Keep
          }
          else
          {
            Action::Discard
          }
        }
        Ok(GossipMessage::RequestGreeting) =>
        {
          info!("BadGER: got RequestGreeting");
          let rd = self.inner.read();
          let msrep = GreetingMessage {
            my_pubshare: match rd
              .config
              .initial_validators
              .get(&PeerIdW { 0: rd.id.clone() })
            {
              Some(val) => Some(PublicKeyWrap { 0: val.clone() }),
              None => None,
            },
            my_id: PublicKeyWrap {
              0: rd.config.node_id.0.clone(),
            },
            my_sig: SignatureWrap {
              0: rd
                .config
                .node_id
                .1
                .sign(rd.config.node_id.0.clone().to_bytes().to_vec()),
            },
          };
          peer_reply = Some(GossipMessage::Greeting(msrep));
          Action::ProcessAndDiscard
        }
        Ok(GossipMessage::BadgerData(badger_msg)) =>
        {
          info!(
            "BadGER: got gossip message uid: {} {}",
            &badger_msg.uid,
            HexFmt(&badger_msg.data)
          );
          let mut locked = self.inner.write();
          if locked.is_authority(&badger_msg.originator.0)
          {
            debug!("BadGER: am authority");
            if let Ok(msg) =
              bincode::deserialize::<<QHB as ConsensusProtocol>::Message>(&badger_msg.data)
            {
              match locked.handle_message(&badger_msg.originator, msg)
              {
                Ok(_) =>
                {
                  //send is handled separately. trigger propose? or leave it for stream
                  debug!("BadGER: decoded gossip message");
                  Action::ProcessAndDiscard
                }
                Err(e) =>
                {
                  telemetry!(CONSENSUS_DEBUG; "afg.err_handling_msg"; "err" => ?format!("{}", e));
                  Action::Discard
                }
              }
            }
            else
            {
              Action::Discard
            }
          }
          else
          {
            Action::Discard
          }
        }
        Ok(GossipMessage::KeygenData(keygen_msg)) => Action::Discard,

        Err(e) =>
        {
          info!(target: "afg", "Error decoding message {:?}",e);
          telemetry!(CONSENSUS_DEBUG; "afg.err_decoding_msg"; "" => "");

          Action::Discard
        }
      }
    };

    (action, peer_reply)
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
      inner
        .peers
        .update_peer_state(who, PeerConsensusState::Connected);
    };
    let packet = {
      let inner = self.inner.write();
      //inner.peers.new_peer(who.clone());
      GreetingMessage {
        my_pubshare: match inner
          .config
          .initial_validators
          .get(&PeerIdW::from(inner.id.clone()))
        {
          Some(val) => Some(PublicKeyWrap(val.clone())),
          None => None,
        },
        my_id: PublicKeyWrap(inner.config.node_id.0),
        my_sig: SignatureWrap(
          inner
            .config
            .node_id
            .1
            .sign(inner.config.node_id.0.clone().to_bytes().to_vec()),
        ),
      }
    };

    //if let Some(packet) = packet {
    let packet_data = GossipMessage::from(packet).encode();
    context.send_message(who, packet_data);
    //}
  }

  fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<Block>, who: &PeerId)
  {
    self.inner.write().peers.peer_disconnected(who);
  }

  fn validate(
    &self,
    context: &mut dyn ValidatorContext<Block>,
    who: &PeerId,
    data: &[u8],
  ) -> network_gossip::ValidationResult<Block::Hash>
  {
    let (action, peer_reply) = self.do_validate(who, data);
    let topic = badger_topic::<Block>();
    // not with lock held!
    if let Some(msg) = peer_reply
    {
      context.send_message(who, msg.encode());
    }
    context.send_topic(who, topic, false);

    {
      self.flush_message_either::<ShutUp>(None, &mut Some(context));
    }

    match action
    {
      Action::Keep =>
      {
        context.broadcast_message(topic, data.to_vec(), false);
        network_gossip::ValidationResult::ProcessAndKeep(topic)
      }
      Action::ProcessAndDiscard => network_gossip::ValidationResult::ProcessAndDiscard(topic),
      Action::Discard =>
      {
        //self.report(who.clone(), cb);
        network_gossip::ValidationResult::Discard
      }
      Action::Useless(_) => network_gossip::ValidationResult::Discard,
    }
  }

  fn message_allowed<'b>(
    &'b self,
  ) -> Box<dyn FnMut(&PeerId, MessageIntent, &Block::Hash, &[u8]) -> bool + 'b>
  {
    /*let (inner, do_rebroadcast) = {
      use parking_lot::RwLockWriteGuard;

      let mut inner = self.inner.write();
      let now = Instant::now();
      let do_rebroadcast = false;

      /*if now >= inner.next_rebroadcast {
        inner.next_rebroadcast = now + REBROADCAST_AFTER;
        true
      } else {
        false
      };*/

      // downgrade to read-lock.
      (RwLockWriteGuard::downgrade(inner), do_rebroadcast)
    };*/
    //info!("MessageAllowed 2");
    Box::new(move |_who, intent, topic, mut data| {
      if let MessageIntent::PeriodicRebroadcast = intent
      {
        return false; //rebroadcast not needed, I hope?
      }

      //let peer = match inner.peers.peer(who) {
      //	None => return false,
      //	Some(x) => x,
      //};

      if *topic != badger_topic::<Block>()
      //only one topic, i guess we may add epochs eventually
      {
        return false;
      }
      // if the topic is not something we're keeping at the moment,
      // do not send.

      match GossipMessage::decode(&mut data)
      {
        Err(_) => false,
        Ok(GossipMessage::BadgerData(_)) => return false,
        Ok(GossipMessage::KeygenData(_)) => return false,
        Ok(GossipMessage::Greeting(_)) => true,
        Ok(GossipMessage::RequestGreeting) => false,
      }
    })
  }

  fn message_expired<'b>(&'b self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'b>
  {
    //let inner = self.inner.read();
    Box::new(move |topic, mut data| {
      // if the topic is not one of the ones that we are keeping at the moment,
      // it is expired.
      if topic != badger_topic::<Block>()
      //only one topic, i guess we may add epochs eventually
      {
        return true;
      }

      match GossipMessage::decode(&mut data)
      {
        Err(_) => true,
        Ok(GossipMessage::Greeting(_)) => false,
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
    NetworkStream {
      outer: rx,
      inner: None,
    }
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
    NetworkStream {
      outer: rx,
      inner: None,
    }
  }

  fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<B>>)
  {
    self.with_gossip(move |gossip, context| {
      gossip.register_validator(context, HBBFT_ENGINE_ID, validator)
    })
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
    service: N,
    config: crate::Config,
  ) -> (
    Self,
    impl futures03::future::Future<Output = ()> + Send + Unpin,
  )
  {
    let validator = BadgerGossipValidator::new(config, service.local_id().clone());
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
    &self,
    is_voter: bool,
  ) -> (
    impl Stream<Item = <QHB as ConsensusProtocol>::Output>,
    impl SendOut,
  )
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
  pub fn new(
    network: N,
    _is_voter: bool,
    gossip_validator: Arc<BadgerGossipValidator<Block>>,
  ) -> Self
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
      ConsensusMessage {
        engine_id: self.engine_id,
        data: message,
      },
    );
  }

  fn send_topic(&mut self, who: &PeerId, topic: B::Hash, force: bool)
  {
    self
      .gossip
      .send_topic(self.protocol, who, topic, self.engine_id, force);
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
