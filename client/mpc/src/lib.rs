use std::{
	collections::BTreeMap, fmt::Debug, hash::Hash, marker::PhantomData, pin::Pin, sync::Arc, thread, time::Duration,
};

use curv::{
	cryptographic_primitives::{proofs::sigma_dlog::DLogProof, secret_sharing::feldman_vss::VerifiableSS},
	elliptic::curves::traits::ECPoint,
	FE, GE,
};
use futures::{
	channel::mpsc,
	future::{ready, select, FutureExt, TryFutureExt},
	prelude::{Future, Sink, Stream},
	stream::StreamExt,
	task::{Context, Poll, Spawn},
};
use log::{ error, info};//debug
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::{
	KeyGenBroadcastMessage1 as KeyGenCommit, KeyGenDecommitMessage1 as KeyGenDecommit, Keys, Parameters, //PartyPrivate, SignKeys
	SharedKeys, 
};
use parking_lot::RwLock;

use sc_client::Client;
use sc_client_api::{backend::Backend, BlockchainEvents, CallExecutor, ExecutionStrategy};
//use sc_keystore::KeyStorePtr;
use sc_network::{NetworkService, NetworkStateInfo, PeerId};
use sc_network_gossip::{ Network as GossipNetwork, TopicNotification}; //GossipEngine
use sp_blockchain::{Error as ClientError,  Result as ClientResult}; //HeaderBackend
use sp_core::{
//	ecdsa::Pair,StorageKind
	offchain::{OffchainStorage, },
	Blake2Hasher, H256,
};
use sp_offchain::STORAGE_PREFIX;
use sp_runtime::generic::OpaqueDigestItemId;
use sp_runtime::traits::{Block as BlockT, Header};

use sp_mpc::{get_storage_key, ConsensusLog, MpcRequest, OffchainStorageType,  MPC_ENGINE_ID}; //RequestId

mod communication;
mod periodic_stream;
mod signer;

use communication::{
	gossip::{GossipMessage, MessageWithSender},
	message::{ConfirmPeersMessage, KeyGenMessage, PeerIndex, SigGenMessage},
	NetworkBridge,
};
use periodic_stream::PeriodicStream;
use signer::Signer;

pub trait Network<B: BlockT>: GossipNetwork<B> + Clone + Send + 'static {
	fn local_peer_id(&self) -> PeerId;
}

impl<B, S, H> Network<B> for Arc<NetworkService<B, S, H>>
where
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

#[derive(Debug)]
pub struct SigGenState {}

pub(crate) struct Environment<B, E, Block: BlockT, RA, Storage> {
	pub client: Arc<Client<B, E, Block, RA>>,
	pub config: NodeConfig,
	pub bridge: NetworkBridge<Block>,
	pub state: Arc<RwLock<KeyGenState>>,
	pub offchain: Arc<RwLock<Storage>>,
}

struct KeyGenWork<B, E, Block: BlockT, RA, Storage> {
	key_gen: Pin<Box<dyn Future<Output = Result<(), Error>> + Send + Unpin>>,
	env: Arc<Environment<B, E, Block, RA, Storage>>,
	mpc_arg_rx: mpsc::UnboundedReceiver<MpcRequest>,
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
		offchain: Storage,
		mpc_arg_rx: mpsc::UnboundedReceiver<MpcRequest>,
	) -> Self {
		let state = KeyGenState::default();

		let env = Arc::new(Environment {
			client,
			config,
			bridge,
			state: Arc::new(RwLock::new(state)),
			offchain: Arc::new(RwLock::new(offchain)),
		});

		let mut work = Self {
			key_gen: Box::pin(futures::future::pending()),
			env,
			mpc_arg_rx,
		};
		work.rebuild(true);
		work
	}

	fn rebuild(&mut self, last_message_ok: bool) {
		// last_message_ok = true when the first build, = false else
		let (incoming, outgoing) = global_comm(&self.env.bridge, self.env.config.duration);
		let signer = Signer::new(self.env.clone(), incoming, outgoing, last_message_ok);

		self.key_gen = Box::pin(signer);
	}

	fn handle_command(&mut self, command: MpcRequest) {
		match command {
			MpcRequest::KeyGen(id) => {
				self.env.bridge.start_key_gen(id);
			}
			_ => {}
		}
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
		println!("POLLING IN KEYGEN WORK");

		match self.mpc_arg_rx.poll_next_unpin(cx) {
			Poll::Pending => {}
			Poll::Ready(None) => {
				// impossible
				return Poll::Ready(Ok(()));
			}
			Poll::Ready(Some(command)) => {
				self.handle_command(command);
				futures01::task::current().notify();
			}
		}

		match self.key_gen.poll_unpin(cx) {
			Poll::Pending => {
				{
					let state = self.env.state.read();
					let validator = self.env.bridge.validator.inner.read();

					if state.complete {
						let mut offchain_storage = self.env.offchain.write();

						let lk = state.local_key.clone().unwrap();
						let seed = bincode::serialize(&lk).unwrap();
						offchain_storage.set(STORAGE_PREFIX, b"local_key", &seed);
					}

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
				};

				return Poll::Pending;
			}
			Poll::Ready(Ok(())) => {
				return Poll::Ready(Ok(()));
			}
			Poll::Ready(Err(e)) => {
				match e {
					Error::Rebuild => {
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

pub fn run_mpc_task<B, E, Block, N, RA, Ex>(
	client: Arc<Client<B, E, Block, RA>>,
	backend: Arc<B>,
	network: N,
	executor: Ex,
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
	let config = NodeConfig {
		duration: 1,
		threshold: 1,
		players: 3,
	};

	let local_peer_id = network.local_peer_id();
	let bridge = NetworkBridge::new(network, config.clone(), local_peer_id, &executor);
	let offchain_storage = backend.offchain_storage().expect("need offchain storage");

	let (tx, rx) = mpsc::unbounded();

	let streamer = client.clone().import_notification_stream().for_each(move |n| {
		let logs = n.header.digest().logs().iter();
		if n.header.number() == &5.into() {
			// temp workaround since cannot use polkadot js now
			let _ = tx.unbounded_send(MpcRequest::KeyGen(1));
		}

		let arg = logs
			.filter_map(|l| l.try_to::<ConsensusLog>(OpaqueDigestItemId::Consensus(&MPC_ENGINE_ID)))
			.find_map(|l| match l {
				ConsensusLog::RequestForSig(req_id, pk_id, data) => Some(MpcRequest::SigGen(req_id, pk_id, data)),
				ConsensusLog::RequestForKey(id) => Some(MpcRequest::KeyGen(id)),
			});

		if let Some(arg) = arg {
			match arg {
				MpcRequest::SigGen(req_id, pk_id, mut data) => {
					let req = MpcRequest::SigGen(req_id, pk_id, data.clone());
					let _ = tx.unbounded_send(req);
					if let Some(mut offchain_storage) = backend.offchain_storage() {
						let key = get_storage_key(req_id, OffchainStorageType::Signature);
						info!("key {:?} data {:?}", key, data);
						let mut t = vec![1u8];
						t.append(&mut data);
						offchain_storage.set(STORAGE_PREFIX, &key, &t);
					}
				}
				kg @ MpcRequest::KeyGen(_) => {
					let _ = tx.unbounded_send(kg);
				}
			}
		}

		ready(())
	});

	let keygen_work =
		KeyGenWork::new(client, config, bridge, offchain_storage, rx).map_err(|e| error!("Error {:?}", e));

	let worker = select(streamer, keygen_work).then(|_| ready(Ok(())));

	Ok(worker.compat())
}
