use std::{
	collections::BTreeMap, fmt::Debug, hash::Hash, marker::PhantomData, pin::Pin, sync::Arc,
	thread, time::Duration,
};

use curv::cryptographic_primitives::proofs::sigma_dlog::DLogProof;
use curv::cryptographic_primitives::secret_sharing::feldman_vss::VerifiableSS;
use curv::{FE, GE};
use futures::{
	future::{select, FutureExt, TryFutureExt},
	prelude::{Future, Sink, Stream},
	stream::StreamExt,
	task::{Context, Poll, Spawn},
};
use log::{debug, error, info};
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::{
	KeyGenBroadcastMessage1 as KeyGenCommit, KeyGenDecommitMessage1 as KeyGenDecommit, Keys,
	Parameters, PartyPrivate, SharedKeys, SignKeys,
};
use parking_lot::RwLock;

use sc_client::Client;
use sc_client_api::{backend::Backend, BlockchainEvents, CallExecutor, ExecutionStrategy};
use sc_keystore::KeyStorePtr;
use sc_network::{PeerId, NetworkService, NetworkStateInfo};
use sc_network_gossip::{GossipEngine, Network as GossipNetwork, TopicNotification};
use sp_blockchain::{Error as ClientError, HeaderBackend, Result as ClientResult};
use sp_core::{
	offchain::{OffchainStorage, StorageKind},
	Blake2Hasher, H256,
};
use sp_offchain::STORAGE_PREFIX;
use sp_runtime::generic::OpaqueDigestItemId;
use sp_runtime::traits::{Block as BlockT, Header};

use sp_mpc::{ConsensusLog, RequestId, MPC_ENGINE_ID};

mod communication;
mod periodic_stream;
mod signer;

use communication::{
	gossip::{GossipMessage, MessageWithSender},
	message::{ConfirmPeersMessage, KeyGenMessage, PeerIndex, SignMessage},
	NetworkBridge,
};
use periodic_stream::PeriodicStream;
use signer::Signer;

pub trait Network<B: BlockT>: GossipNetwork<B> + Clone + Send + 'static {
	fn local_peer_id(&self) -> PeerId;
}

impl<B, S, H> Network<B> for Arc<NetworkService<B, S, H>> where
	B: BlockT,
	S: sc_network::specialization::NetworkSpecialization<B>,
	H: sc_network::ExHashT,
{
	fn local_peer_id(&self) -> PeerId {
		NetworkService::local_peer_id(self)
	}
}

#[derive(Debug)]
pub enum Error {
	Network(String),
	Periodic,
	Client(ClientError),
	Rebuild,
}

#[derive(Clone)]
pub struct NodeConfig {
	pub duration: u64,
	pub threshold: u16,
	pub players: u16,
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

pub(crate) struct Environment<B, E, Block: BlockT, RA, Storage> {
	pub client: Arc<Client<B, E, Block, RA>>,
	pub config: NodeConfig,
	pub bridge: NetworkBridge<Block>,
	pub state: Arc<RwLock<KeyGenState>>,
	pub offchain: Arc<RwLock<Storage>>,
}

#[derive(Debug)]
pub enum MpcArgument {
	KeyGen(RequestId),
	SigGen(RequestId, Vec<u8>),
}

struct KeyGenWork<B, E, Block: BlockT, RA, Storage> {
	key_gen: Pin<Box<dyn Future<Output = Result<(), Error>> + Send + Unpin>>,
	env: Arc<Environment<B, E, Block, RA, Storage>>,
}

impl<B, E, Block, RA, Storage> KeyGenWork<B, E, Block, RA, Storage>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + 'static,
	Block: BlockT<Hash = H256> + Unpin,
	Block::Hash: Ord,
	RA: Send + Sync + 'static,
	Storage: OffchainStorage + 'static,
{
	fn new(
		client: Arc<Client<B, E, Block, RA>>,
		config: NodeConfig,
		bridge: NetworkBridge<Block>,
		db: Storage,
	) -> Self {
		let state = KeyGenState::default();

		let env = Arc::new(Environment {
			client,
			config,
			bridge,
			state: Arc::new(RwLock::new(state)),
			offchain: Arc::new(RwLock::new(db)),
		});

		let mut work = Self {
			key_gen: Box::pin(futures::future::pending()),
			env,
		};
		work.rebuild(true); // init should be okay
		work
	}

	fn rebuild(&mut self, last_message_ok: bool) {
		let (incoming, outgoing) = global_comm(&self.env.bridge, self.env.config.duration);
		let signer = Signer::new(self.env.clone(), incoming, outgoing, last_message_ok);

		self.key_gen = Box::pin(signer);
	}
}

impl<B, E, Block, RA, Storage> Future for KeyGenWork<B, E, Block, RA, Storage>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + 'static + Send + Sync,
	Block: BlockT<Hash = H256> + Unpin,
	Block::Hash: Ord,
	RA: Send + Sync + 'static,
	Storage: OffchainStorage + 'static,
{
	type Output = Result<(), Error>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		println!("POLLLLLLLLLLLLLLLLLLLL");
		match self.key_gen.poll_unpin(cx) {
			Poll::Pending => {
				let (is_complete, is_canceled, commits_len) = {
					let state = self.env.state.read();
					let validator = self.env.bridge.validator.inner.read();

					println!(
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
			Poll::Ready(Ok(())) => {
				return Poll::Ready(Ok(()));
			}
			Poll::Ready(Err(e)) => {
				match e {
					Error::Rebuild => {
						println!("inner keygen rebuilt");
						self.rebuild(false);
						futures01::task::current().notify();
						return Poll::Pending;
					}
					_ => {}
				}
				return Poll::Ready(Err(e));
			}
		}
	}
}

fn global_comm<Block>(
	bridge: &NetworkBridge<Block>,
	duration: u64,
) -> (
	impl Stream<Item = MessageWithSender>,
	impl Sink<MessageWithSender, Error = Error>,
)
where
	Block: BlockT<Hash = H256>,
{
	let (global_in, global_out) = bridge.global();
	let global_in = PeriodicStream::<_, MessageWithSender>::new(global_in, duration);

	(global_in, global_out)
}

pub fn run_task<B, E, Block, N, RA, Ex>(
	client: Arc<Client<B, E, Block, RA>>,
	backend: Arc<B>,
	network: N,
	keystore: KeyStorePtr,
	executor: Ex
) -> ClientResult<impl futures01::Future<Item = (), Error = ()>>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + 'static + Send + Sync,
	Block: BlockT<Hash = H256> + Unpin,
	Block::Hash: Ord,
	N: Network<Block>,
	RA: Send + Sync + 'static,
	Ex: Spawn + 'static,
{
	let keyclone = keystore.clone();
	let config = NodeConfig {
		duration: 1,
		threshold: 1,
		players: 3,
		keystore: Some(keystore),
	};
	let backend_ = backend.clone();
	let streamer = client
		.clone()
		.import_notification_stream()
		.for_each(move |n| {
			let logs = n.header.digest().logs().iter();

			let arg = logs
				.filter_map(|l| {
					l.try_to::<ConsensusLog>(OpaqueDigestItemId::Consensus(&MPC_ENGINE_ID))
				})
				.find_map(|l| match l {
					ConsensusLog::RequestForSig(id, data) => Some(MpcArgument::SigGen(id, data)),
					ConsensusLog::RequestForKey(id) => Some(MpcArgument::KeyGen(id)),
				});

			if let Some(arg) = arg {
				match arg {
					MpcArgument::SigGen(id, mut data) => {
						if let Some(mut offchain_storage) = backend.offchain_storage() {
							let key = id.to_le_bytes();
							info!("key {:?} data {:?}", key, data);
							let mut t = vec![1u8];
							t.append(&mut data);
							offchain_storage.set(STORAGE_PREFIX, &key, &t);
						}
					}
					MpcArgument::KeyGen(id) => {}
				}
			}

			futures::future::ready(())
		});

	let local_peer_id = network.local_peer_id();
	let bridge = NetworkBridge::new(network, config.clone(), local_peer_id, &executor);

	let keygen_work = KeyGenWork::new(
		client,
		config,
		bridge,
		backend_
			.offchain_storage()
			.expect("Need offchain for keygen work"),
	)
	.map_err(|e| error!("Error {:?}", e));

	let worker = select(streamer, keygen_work).then(|_| futures::future::ready(Ok(())));

	Ok(worker.compat())
}
