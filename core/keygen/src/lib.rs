<<<<<<< HEAD
//#![feature(async_closure)]
=======
>>>>>>> ada6eea5c70dc89ce34688fcb25b40a720010a39

use client::backend::OffchainStorage;
use primitives::offchain::StorageKind;
use std::{
  collections::{BTreeMap, VecDeque},
  fmt::Debug,
  marker::Unpin,
  pin::Pin,
  sync::Arc,
  time::{Duration, Instant},
};

use mpe_primitives::ConsensusLog;
use primitives::traits::BareCryptoStore;

use codec::{Decode, Encode};
use curv::cryptographic_primitives::proofs::sigma_dlog::DLogProof;
use curv::cryptographic_primitives::secret_sharing::feldman_vss::VerifiableSS;
use curv::{FE, GE};

use futures03::channel::oneshot::{self, Canceled};
use futures03::compat::{Compat, Compat01As03};
use futures03::future::{FutureExt, TryFutureExt};
use futures03::prelude::{Future, Sink, Stream, TryStream};
use futures03::stream::{FilterMap, StreamExt, TryStreamExt};
use futures03::task::{Context, Poll};
use keystore::KeyStorePtr;
use log::{debug, error, info};
use mpe_primitives::ConsensusLog::RequestForKeygen;
use mpe_primitives::MP_ECDSA_ENGINE_ID;
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::{
  KeyGenBroadcastMessage1 as KeyGenCommit, KeyGenDecommitMessage1 as KeyGenDecommit, Keys,
  Parameters, PartyPrivate, SharedKeys, SignKeys,
};
use parking_lot::RwLock;
use rand::prelude::Rng;
use tokio02::timer::Interval;
use tokio_executor::DefaultExecutor;

use client::{
  backend::Backend, error::Error as ClientError, error::Result as ClientResult, BlockchainEvents,
  CallExecutor, Client,
};
use consensus_common::SelectChain;
use inherents::InherentDataProviders;
use network::{self, PeerId};
use primitives::crypto::key_types::ECDSA_SHARED;
use primitives::{Blake2Hasher, H256};
use sr_primitives::generic::{BlockId, OpaqueDigestItemId};
use sr_primitives::traits::{Block as BlockT, DigestFor, Header, NumberFor, ProvideRuntimeApi};

mod communication;
mod periodic_stream;
mod shared_state;
mod signer;

use communication::{
  gossip::{GossipMessage, MessageWithSender},
  message::{ConfirmPeersMessage, KeyGenMessage, PeerIndex, SignMessage},
  Network, NetworkBridge,
};
use periodic_stream::PeriodicStream;
use shared_state::{load_persistent, set_signers, SharedState};
use signer::Signer;

type Count = u16;

#[derive(Debug)]
pub enum Error
{
  Network(String),
  Periodic,
  Client(ClientError),
  Rebuild,
}

#[derive(Clone)]
pub struct NodeConfig
{
  pub duration: u64,
  pub threshold: Count,
  pub players: Count,
  pub keystore: Option<KeyStorePtr>,
}

impl NodeConfig
{
  pub fn get_params(&self) -> Parameters
  {
    Parameters {
      threshold: self.threshold,
      share_count: self.players,
    }
  }
}
//pub trait ErrorFuture=futures03::Future<Output = Result<(),Error>>;
//pub trait EmptyFuture=futures03::Future<Output = Result<(),()>>;
struct EmptyWrapper;
impl futures03::Future for EmptyWrapper
{
  type Output = Result<(), Error>;
  fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output>
  {
    Poll::Ready(Ok(()))
  }
}

#[derive(Debug)]
pub struct KeyGenState
{
  pub complete: bool,
  pub local_key: Option<Keys>,
  pub commits: BTreeMap<PeerIndex, KeyGenCommit>,
  pub decommits: BTreeMap<PeerIndex, KeyGenDecommit>,
  pub vsss: BTreeMap<PeerIndex, VerifiableSS>,
  pub secret_shares: BTreeMap<PeerIndex, FE>,
  pub proofs: BTreeMap<PeerIndex, DLogProof>,
  pub shared_keys: Option<SharedKeys>,
}

impl KeyGenState
{
  pub fn shared_public_key(&self) -> Option<GE>
  {
    self.shared_keys.clone().map(|sk| sk.y)
  }

  pub fn reset(&mut self)
  {
    *self = Self::default();
  }
}

impl Default for KeyGenState
{
  fn default() -> Self
  {
    Self {
      complete: false,
      local_key: None,
      commits: BTreeMap::new(),
      decommits: BTreeMap::new(),
      vsss: BTreeMap::new(),
      secret_shares: BTreeMap::new(),
      proofs: BTreeMap::new(),
      shared_keys: None,
    }
  }
}

pub struct SigGenState
{
  pub complete: bool,
  pub sign_key: Option<SignKeys>,
}

fn global_comm<Block, N>(
  bridge: &NetworkBridge<Block, N>, duration: u64,
) -> (
  impl Stream<Item = MessageWithSender>,
  impl Sink<MessageWithSender, Error = Error>,
)
where
  Block: BlockT<Hash = H256>,
  N: Network<Block> + Unpin,
  N::In: Send,
{
  let (global_in, global_out) = bridge.global();
  let global_in = PeriodicStream::<_, MessageWithSender>::new(global_in, duration);

  (global_in, global_out)
}

pub(crate) struct Environment<B, E, Block: BlockT, N: Network<Block>, RA, Storage>
{
  pub client: Arc<Client<B, E, Block, RA>>,
  pub config: NodeConfig,
  pub bridge: NetworkBridge<Block, N>,
  pub state: Arc<RwLock<KeyGenState>>,
  pub offchain: Arc<RwLock<Storage>>,
}

use mpe_primitives::MAIN_DB_PREFIX;
pub const STORAGE_PREFIX: &[u8] = b"storage";
use primitives::crypto::Public;
use mpe_primitives::{get_data_prefix,get_key_prefix,get_complete_list_prefix,RequestId};
impl<B, E, Block: BlockT, N: Network<Block>, RA,Storage> Environment<B, E, Block, N, RA,Storage>
where Storage:OffchainStorage
{
	pub fn set_request_data(&self,request_id:RequestId,request_data:&[u8])
	{
		let  key:Vec<u8>=get_data_prefix(request_id);
		self.local_storage_set(StorageKind::PERSISTENT, &key, request_data)
	}
    pub fn set_request_complete(&self,request_id:RequestId) ->Result<(),&'static str>
	{
		let  key:Vec<u8>=get_complete_list_prefix();
		let mut reqs=Vec::<RequestId>::new();
        if let Some(data)=self.local_storage_get(&key)
		{
			reqs=match Decode::decode(&mut data.as_ref())
			{
				Ok(res) => res,
				Err(_) => return Err("Invalid data stored")
			}
		}
		reqs.push(request_id);
		reqs.sort();
		reqs.dedup();
        self.local_storage_set(StorageKind::PERSISTENT,&key,&reqs.encode());
		Ok(())
	}
	pub fn set_request_public_key<PK>(&self,request_id:RequestId,public_key:&PK) ->Result<(),&'static str>
	where PK:  Public
	{
		let  key:Vec<u8>=get_key_prefix(request_id);
		if let Some(_)=self.local_storage_get(&key)
		{
			return Err("Can only set key once per request")
		}
		self.local_storage_set(StorageKind::PERSISTENT, &key, public_key.as_slice())  ;
		self.set_request_complete(request_id)?;

		Ok(())

	}
	pub fn set_request_failure(&self,request_id:RequestId) ->Result<(),&'static str>
	{
		let mut key:Vec<u8>=get_key_prefix(request_id);
		if let Some(_)=self.local_storage_get(&key)
		{
			return Err("Can only set key/fail once per request")
		}
		let fail=[255u8;1];

		self.local_storage_set(StorageKind::PERSISTENT, &key, &fail) ;
		self.set_request_complete(request_id)?;

		Ok(())

	}

	pub fn local_storage_set(&self, kind: StorageKind, key: &[u8], value: &[u8]) {
		match kind {
			StorageKind::PERSISTENT => self.offchain.write().set(STORAGE_PREFIX, key, value),
			StorageKind::LOCAL => {},
		}
	}
	pub fn local_storage_get(&self,  key: &[u8]) -> Option<Vec<u8>>
	{
			self.offchain.read().get(STORAGE_PREFIX, key)
	}
	pub fn local_storage_compare_and_set(
		&self,
		kind: StorageKind,
		key: &[u8],
		old_value: Option<&[u8]>,
		new_value: &[u8],
	) -> bool {
		match kind {
			StorageKind::PERSISTENT => {
				self.offchain.write().compare_and_set(STORAGE_PREFIX, key, old_value, new_value)
			},
			StorageKind::LOCAL => false ,
		}
	}

}


struct KeyGenWork<B, E, Block: BlockT, N: Network<Block>, RA, Storage>
{
  key_gen: Pin<Box<dyn Future<Output = Result<(), Error>> + Send + Unpin>>,
  env: Arc<Environment<B, E, Block, N, RA, Storage>>,
}

impl<B, E, Block, N, RA, Storage> KeyGenWork<B, E, Block, N, RA, Storage>
where
  B: Backend<Block, Blake2Hasher> + 'static,
  E: CallExecutor<Block, Blake2Hasher> + Send + Sync + 'static,
  Block: BlockT<Hash = H256> + Unpin,
  Block::Hash: Ord,
  N: Network<Block> + Sync + Unpin,
  N::In: Send + 'static + Unpin,
  RA: Send + Sync + 'static,
  Storage: OffchainStorage + 'static,
{
  fn new(
    client: Arc<Client<B, E, Block, RA>>, config: NodeConfig, bridge: NetworkBridge<Block, N>,
    db: Storage,
  ) -> Self
  {
    let state = KeyGenState::default();

    let env = Arc::new(Environment {
      client,
      config,
      bridge,
      state: Arc::new(RwLock::new(state)),
      offchain: Arc::new(RwLock::new(db)),
    });

    let mut work = Self {
      key_gen: Box::pin(futures03::future::pending()),
      env,
    };
    work.rebuild(true); // init should be okay
    work
  }

  fn rebuild(&mut self, last_message_ok: bool)
  {
    let (incoming, outgoing) = global_comm(&self.env.bridge, self.env.config.duration);
    let signer = Signer::new(self.env.clone(), incoming, outgoing, last_message_ok);

    self.key_gen = Box::pin(signer);
  }
}

impl<B, E, Block, N, RA, Storage> futures03::Future for KeyGenWork<B, E, Block, N, RA, Storage>
where
  B: Backend<Block, Blake2Hasher> + 'static,
  E: CallExecutor<Block, Blake2Hasher> + 'static + Send + Sync,
  Block: BlockT<Hash = H256> + Unpin,
  Block::Hash: Ord,
  N: Network<Block> + Send + Sync + Unpin + 'static,
  N::In: Send + 'static,
  RA: Send + Sync + 'static,
  Storage: OffchainStorage + 'static,
{
  type Output = Result<(), Error>;

  fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output>
  {
    println!("POLLLLLLLLLLLLLLLLLLLL");
    match self.key_gen.poll_unpin(cx)
    {
      Poll::Pending =>
      {
        let (is_complete, is_canceled, commits_len) = {
          let state = self.env.state.read();
          let validator = self.env.bridge.validator.inner.read();

          info!(
						"INDEX {:?} state: commits {:?} decommits {:?} vss {:?} ss {:?}  proof {:?} has key {:?} complete {:?} peers hash {:?}",
						validator.get_local_index(),
						state.commits.len(),
						state.decommits.len(),
						state.vsss.len(),
						state.secret_shares.len(),
						state.proofs.len(),
						state.local_key.is_some(),
						state.complete,
						validator.get_peers_hash()
					);
          (
            validator.is_local_complete(),
            validator.is_local_canceled(),
            state.commits.len(),
          )
        };

        return Poll::Pending;
      }
      Poll::Ready(Ok(())) =>
      {
        return Poll::Ready(Ok(()));
      }
      Poll::Ready(Err(e)) =>
      {
        match e
        {
          Error::Rebuild =>
          {
            println!("inner keygen rebuilt");
            self.rebuild(false);
            futures::task::current().notify();
            return Poll::Pending;
          }
          _ =>
          {}
        }
        return Poll::Ready(Err(e));
      }
    }
  }
}

// pub fn init_shared_state<B, E, Block, RA>(client: Arc<Client<B, E, Block, RA>>) -> SharedState
// where
// 	B: Backend<Block, Blake2Hasher> + 'static,
// 	E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
// 	Block: BlockT<Hash = H256>,
// 	Block::Hash: Ord,
// 	RA: Send + Sync + 'static,
// {
// 	let persistent_data: SharedState = load_persistent(&**client.backend()).unwrap();
// 	persistent_data
// }

pub fn run_key_gen<B, E, Block, N, RA>(
  local_peer_id: PeerId, (threshold, players): (PeerIndex, PeerIndex), duration: u64,
  keystore: KeyStorePtr, client: Arc<Client<B, E, Block, RA>>, network: N, backend: Arc<B>,
) -> ClientResult<impl Future<Output = Result<(), ()>> + Send + 'static>
where
  B: Backend<Block, Blake2Hasher> + 'static,
  E: CallExecutor<Block, Blake2Hasher> + 'static + Send + Sync,
  Block: BlockT<Hash = H256> + Unpin,
  Block::Hash: Ord,
  N: Network<Block> + Send + Sync + Unpin + 'static,
  N::In: Send + 'static,
  RA: Send + Sync + 'static,
{
  let keyclone = keystore.clone();
  let config = NodeConfig {
    threshold,
    players,
    keystore: Some(keystore),
    duration,
  };

  // let keystore = keystore.read();

  // let persistent_data: SharedState = load_persistent(&**client.backend()).unwrap();
  // println!("{:?}", persistent_data);
  // println!("Local peer ID {:?}", current_id.as_bytes());

  // let mut signers = persistent_data.signer_set;
  // let current_id = current_id.into_bytes();
  // if !signers.contains(&current_id) {
  // 	// if our id is not in it, add our self
  // 	signers.push(current_id);
  // 	set_signers(&**client.backend(), signers);
  // }
  let bridge = NetworkBridge::new(network, config.clone(), local_peer_id);
  let streamer=client.clone().import_notification_stream().for_each(move |n| {
		info!(target: "keygen", "HEADER {:?}, looking for consensus message", &n.header);
	        for log in n.header.digest().logs()
		{
		 info!(target: "keygen", "Checking log {:?}, looking for consensus message", log);
		 match log.try_as_raw(OpaqueDigestItemId::Consensus(&MP_ECDSA_ENGINE_ID))
		 {
			Some(data) => {
				info!("Got log id! {:?}",data);
		       let log_inner = log.try_to::<ConsensusLog>(OpaqueDigestItemId::Consensus(&MP_ECDSA_ENGINE_ID));
                info!("Got log inner! {:?}",log_inner);
				if let Some(log_in)=log_inner
				{
		       match log_in
			   {
				   RequestForKeygen((id,data)) =>
				   {
					  match keyclone.read().initiate_request(&id.to_be_bytes(),ECDSA_SHARED)
					  {
						  Ok(_) => {},
						  Err(e) =>debug!("Error in initiate request: {:?}",&e),
					  }

				   },
				   _ =>{}
			   }
				}
			 },
			None => {}
	     }
		//..ECDSA_SHARED KeyTypeId
	    }
		info!(target: "substrate-log", "Imported with log called  #{} ({})", n.header.number(), n.hash);
		futures03::future::ready( ())
//		Ok(())
	});

  let key_gen_work = KeyGenWork::new(
    client,
    config,
    bridge,
    backend
      .offchain_storage()
      .expect("Need offchain for keygen work"),
  )
  .map_err(|e| error!("Error {:?}", e));
  Ok(futures03::future::select(key_gen_work, streamer).then(|_| futures03::future::ready(Ok(()))))
}

#[cfg(test)]
mod tests;
