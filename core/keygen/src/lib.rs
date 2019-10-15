#![feature(async_closure)]

use std::{
	collections::{BTreeMap, VecDeque},
	fmt::Debug,
	marker::Unpin,
	pin::Pin,
	sync::Arc,
	time::{Duration, Instant},
};
use client::backend::OffchainStorage;
use primitives::offchain::StorageKind;

use mpe_primitives::ConsensusLog;
use sr_primitives::traits::Header;
use sr_primitives::generic::OpaqueDigestItemId;
use primitives::traits::BareCryptoStore;

use codec::{Decode, Encode};
use curv::cryptographic_primitives::proofs::sigma_dlog::DLogProof;
use curv::cryptographic_primitives::secret_sharing::feldman_vss::VerifiableSS;
use curv::{FE, GE};

use futures::prelude::{Future as Future01, Sink as Sink01, Stream as Stream01};
use futures03::channel::oneshot::{self, Canceled};
use futures03::compat::{Compat, Compat01As03};
use futures03::future::{FutureExt, TryFutureExt};
use futures03::prelude::{Future, Sink, Stream, TryStream};
use futures03::sink::SinkExt;
use futures03::stream::{FilterMap, StreamExt, TryStreamExt};
use futures03::task::{Context, Poll};
use keystore::KeyStorePtr;
use log::{debug, error, info};
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::{
	KeyGenBroadcastMessage1 as KeyGenCommit, KeyGenDecommitMessage1 as KeyGenDecommit, Keys,
	Parameters, PartyPrivate, SharedKeys, SignKeys,
};
use parking_lot::RwLock;
use rand::prelude::Rng;
use tokio_executor::DefaultExecutor;
use primitives::crypto::key_types::ECDSA_SHARED;
use mpe_primitives::ConsensusLog::RequestForKeygen;
use mpe_primitives::MP_ECDSA_ENGINE_ID;
use tokio02::timer::Interval;

use client::blockchain::HeaderBackend;
use client::{
	backend::Backend, error::Error as ClientError, error::Result as ClientResult, BlockchainEvents,
	CallExecutor, Client,
};
use consensus_common::SelectChain;
//use inherents::InherentDataProviders;
use network::{self, PeerId};
use primitives::{Blake2Hasher, H256};
use sr_primitives::generic::BlockId;
use sr_primitives::traits::{Block as BlockT, DigestFor, NumberFor, ProvideRuntimeApi};
//use crate::communication::MWSStream;
//use crate::communication::MWSSink;


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

const REBUILD_COOLDOWN: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum Error {
	Network(String),
	Periodic,
	Client(ClientError),
	Rebuild,
}

#[derive(Clone)]
pub struct NodeConfig {
	pub threshold: Count,
	pub players: Count,
	pub keystore: Option<KeyStorePtr>,
}

impl NodeConfig {
	pub fn get_params(&self) -> Parameters {
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
  type Output= Result<(),Error>;
  fn poll (self:Pin <&mut Self>,_cx: &mut Context<'_>) -> Poll<Self::Output>
  {
	  Poll::Ready(Ok(()))
  }
}



#[derive(Debug)]
pub struct KeyGenState {
	pub complete: bool,
	pub local_key: Option<Keys>,
	pub commits: BTreeMap<PeerIndex, KeyGenCommit>,
	pub decommits: BTreeMap<PeerIndex, KeyGenDecommit>,
	pub vsss: BTreeMap<PeerIndex, VerifiableSS>,
	pub secret_shares: BTreeMap<PeerIndex, FE>,
	pub proofs: BTreeMap<PeerIndex, DLogProof>,
	pub shared_keys: Option<SharedKeys>,
}

impl KeyGenState {
	pub fn shared_public_key(&self) -> Option<GE> {
		self.shared_keys.clone().map(|sk| sk.y)
	}

	pub fn reset(&mut self) {
		*self = Self::default();
	}
}

impl Default for KeyGenState {
	fn default() -> Self {
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

pub struct SigGenState {
	pub complete: bool,
	pub sign_key: Option<SignKeys>,
}

fn global_comm<Block, N>(
	bridge: &NetworkBridge<Block, N>,
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
	let global_in = PeriodicStream::<_, MessageWithSender>::new(global_in);

	(global_in, global_out)
}

pub(crate) struct Environment<B, E, Block: BlockT, N: Network<Block>, RA,Storage> {
	pub client: Arc<Client<B, E, Block, RA>>,
	pub config: NodeConfig,
	pub bridge: NetworkBridge<Block, N>,
	pub state: Arc<RwLock<KeyGenState>>,
	pub offchain:Arc<RwLock<Storage>>,
}

struct KeyGenWork<B, E, Block: BlockT, N: Network<Block>, RA,Storage> 
where Storage:OffchainStorage
{
	key_gen: Pin<Box<dyn Future<Output = Result<(), Error>> + Send + Unpin + 'static>>,
	env: Arc<Environment<B, E, Block, N, RA,Storage>>,
	//check_pending: Interval,
}





impl<B, E, Block, N, RA,Storage> KeyGenWork<B, E, Block, N, RA,Storage>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + 'static,
	Block: BlockT<Hash = H256>+Unpin,
	Block::Hash: Ord,
	N: Network<Block> + Sync +Unpin,
	N::In: Send + 'static+Unpin,
	RA: Send + Sync + 'static,
    Storage:OffchainStorage+'static,
{
	fn new(
		client: Arc<Client<B, E, Block, RA>>,
		config: NodeConfig,
		bridge: NetworkBridge<Block, N>,
		  db:Storage,
	) -> Self {
		let state = KeyGenState::default();

		let env = Arc::new(Environment {
			client,
			config,
			bridge,
			state: Arc::new(RwLock::new(state)),
			offchain:Arc::new(RwLock::new(db)),
		});

		let mut work = Self {
			key_gen: Box::pin(futures03::future::pending()),
			env,
		};
		work.rebuild(true); // init should be okay
		work
	}

	fn rebuild(&mut self, last_message_ok: bool) {
		let (incoming, outgoing) = global_comm(&self.env.bridge);
		let signer = Signer::new(self.env.clone(), incoming, outgoing, last_message_ok);

		self.key_gen = Box::pin(signer);
	}
}

impl<B, E, Block, N, RA,Storage> futures03::Future for KeyGenWork<B, E, Block, N, RA,Storage>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + 'static + Send + Sync,
	Block: BlockT<Hash = H256>+Unpin,
	Block::Hash: Ord,
	N: Network<Block> + Send + Sync + Unpin + 'static,
	N::In: Send + 'static,
	RA: Send + Sync + 'static,
	Storage: OffchainStorage+'static,
{

	type Output = Result<(), Error>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		let mut is_ready = false;
		// while let Poll::Ready(Some(_)) = self.check_pending.poll().map_err(|_| Error::Periodic)? {
		// 	is_ready = true;
		// }

		println!("POLLLLLLLLLLLLLLLLLLLL");
		match self.key_gen.poll_unpin(cx) {
			Poll::Pending => {
				let (is_complete, is_canceled, commits_len) = {
					let state = self.env.state.read();
					let validator = self.env.bridge.validator.inner.read();

					info!(
						"INDEX {:?} state: commits {:?} decommits {:?} vss {:?} ss {:?}  proof {:?} has key {:?} complete {:?} periodic ready {:?} peers hash {:?}",
						validator.get_local_index(),
						state.commits.len(),
						state.decommits.len(),
						state.vsss.len(),
						state.secret_shares.len(),
						state.proofs.len(),
						state.local_key.is_some(),
						state.complete,
						is_ready,
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
			Poll::Ready(Ok(())) => {
				return Poll::Ready(Ok(()));
			}
			Poll::Ready(Err(e)) => {
				match e {
					Error::Rebuild => {
						println!("inner keygen rebuilt");
						self.rebuild(false);
						futures::task::current().notify();
						return Poll::Pending;
					}
					_ => {}
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
	local_peer_id: PeerId,
	(threshold, players): (PeerIndex, PeerIndex),
	keystore: KeyStorePtr,
	client: Arc<Client<B, E, Block, RA>>,
	network: N,
	backend:Arc<B>,
) -> ClientResult<impl Future<Output = Result<(), ()>> + Send + 'static>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + 'static + Send + Sync,
	Block: BlockT<Hash = H256> +Unpin,
	Block::Hash: Ord,
	N: Network<Block> + Send + Sync + Unpin + 'static,
	N::In: Send + 'static,
	RA: Send + Sync + 'static,
{
	let keyclone=keystore.clone();
	let config = NodeConfig {
		threshold,
		players,
		keystore: Some(keystore),
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
					   keyclone.read().initiate_request(&id.to_be_bytes(),ECDSA_SHARED);
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

	let key_gen_work = KeyGenWork::new(client, config, bridge,backend.offchain_storage().expect("Need offchain for keygen work")).map_err(|e| error!("Error {:?}", e));
	Ok(futures03::future::select(key_gen_work,streamer).then(|_| futures03::future::ready( Ok(())) ))
}

#[cfg(test)]
mod tests;
