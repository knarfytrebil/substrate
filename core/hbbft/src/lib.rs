//use std::collections::BTreeMap;
//use std::fs::File;
use std::marker::{Send, Sync};
//use std::path::PathBuf;
use std::pin::Pin;
//use std::str::FromStr;
use std::{fmt::Debug, hash::Hash, marker::PhantomData, sync::Arc, time::Duration};
use badger::crypto::{
	 SecretKeyShare, //Signature,PublicKeyShare,PublicKey,SecretKeyPublicKeySet
  };

//use app_crypto::hbbft_thresh::Pair as HBPair;
//use threshold_crypto::serde_impl::SerdeSecret;
use bincode;
use futures03::future::Future;
use futures03::prelude::*;
use futures03::task::Poll;
use futures_timer::Interval;
use hex;
use log::{debug, error, info, trace, warn};
use parity_codec::{Decode, Encode};
use parking_lot::Mutex;

use badger_primitives::HBBFT_ENGINE_ID;
use badger_primitives::BadgerPreRuntime;

use badger_primitives::ConsensusLog;
use badger::dynamic_honey_badger::Change;
use badger::dynamic_honey_badger::ChangeState;

use consensus_common::evaluation;
use inherents::InherentIdentifier;
use keystore::KeyStorePtr;
use runtime_primitives::traits::Hash as THash;
use runtime_primitives::traits::{
	BlakeTwo256, Block as BlockT, Header, NumberFor, ProvideRuntimeApi,
};

use runtime_primitives::{
	generic::{self, BlockId,OpaqueDigestItemId},
	ApplyError, Justification,
};

pub mod communication;
use crate::communication::Network;
use badger::ConsensusProtocol;
use badger_primitives::{  HBBFT_AUTHORITIES_KEY};//AuthorityId,AuthorityPair
use std::iter;

use client::backend::Backend;
use client::blockchain::HeaderBackend;
use client::error::Error as ClientError;
use client::runtime_api::ConstructRuntimeApi;
use client::{
	backend::AuxStore, block_builder::api::BlockBuilder as BlockBuilderApi,
	blockchain::ProvideCache, error,
	blockchain, CallExecutor, Client, well_known_cache_keys

};
use communication::NetworkBridge;
use communication::TransactionSet;
use communication::QHB;
use consensus_common::block_import::BlockImport;
use consensus_common::import_queue::{
	BasicQueue, BoxBlockImport, BoxFinalityProofImport, BoxJustificationImport, CacheKeyId,
	Verifier,
};
use std::collections::HashMap;
use consensus_common::ImportResult;
pub use consensus_common::SyncOracle;
use consensus_common::BlockCheckParams;
use consensus_common::{self, BlockImportParams, BlockOrigin, ForkChoiceStrategy, SelectChain, Error as ConsensusError,};
use inherents::{InherentData, InherentDataProviders};
//use network::PeerId;
use runtime_primitives::traits::DigestFor;
use runtime_primitives::generic::DigestItem;
use substrate_primitives::{Blake2Hasher, ExecutionContext, H256,storage::StorageKey,};
use substrate_telemetry::{telemetry, CONSENSUS_INFO, CONSENSUS_WARN};
use transaction_pool::txpool::{self, Pool as TransactionPool};

use crate::communication::SendOut;

pub const INHERENT_IDENTIFIER: InherentIdentifier = *b"snakeshr";

pub type BadgerImportQueue<B> = BasicQueue<B>;
pub mod aux_store;
pub mod rpc;
pub struct BadgerWorker<C, I, SO, Inbound, B: BlockT, N: Network<B>, A>
where
	A: txpool::ChainApi,
{
	pub client: Arc<C>,
	pub block_import: Arc<Mutex<I>>,
	pub network: NetworkBridge<B, N>,

	pub transaction_pool: Arc<TransactionPool<A>>,
	pub sync_oracle: SO,
	pub blocks_in: Inbound,
}

pub struct BadgerVerifier<C, Pub, Sig>
where
	Sig: Send + Sync,
	Pub: Send + Sync,
{
	_client: Arc<C>,
	_pub: PhantomData<Pub>,
	_sig: PhantomData<Sig>,
	_inherent_data_providers: inherents::InherentDataProviders,
}

impl<C, Pub, Sig> BadgerVerifier<C, Pub, Sig>
where
	Sig: Send + Sync,
	Pub: Send + Sync,
{
	fn _check_inherents<B: BlockT>(
		&self,
		_block: B,
		_block_id: BlockId<B>,
		_inherent_data: InherentData,
		_timestamp_now: u64,
	) -> Result<(), String>
	where
		C: ProvideRuntimeApi,
		C::Api: BlockBuilderApi<B>,
	{
		//empty for now
		Ok(())
	}
}


pub struct BadgerBlockImport<B, E, Block: BlockT<Hash=H256>, RA, SC> {
	pub inner: Arc<Client<B, E, Block, RA>>,
	pub select_chain: SC,
	pub authority_set: aux_store::BadgerSharedAuthoritySet,
//	send_voter_commands: mpsc::UnboundedSender<VoterCommand<Block::Hash, NumberFor<Block>>>,
//	consensus_changes: SharedConsensusChanges<Block::Hash, NumberFor<Block>>,
}

impl<B, E, Block: BlockT<Hash=H256>, RA, SC: Clone> Clone for
BadgerBlockImport<B, E, Block, RA, SC>
{
	fn clone(&self) -> Self {
		BadgerBlockImport {
			inner: self.inner.clone(),
			select_chain: self.select_chain.clone(),
			authority_set: self.authority_set.clone(),
		}
	}
}



impl<B, E, Block: BlockT<Hash=H256>, RA, SC>
BadgerBlockImport<B, E, Block, RA, SC>
{
	pub(crate) fn new(
		inner: Arc<Client<B, E, Block, RA>>,
		select_chain: SC,
		authority_set:  aux_store::BadgerSharedAuthoritySet,
		//send_voter_commands: mpsc::UnboundedSender<VoterCommand<Block::Hash, NumberFor<Block>>>,
		//consensus_changes: SharedConsensusChanges<Block::Hash, NumberFor<Block>>,
	) -> BadgerBlockImport<B, E, Block, RA, SC> {
		BadgerBlockImport {
			inner,
			select_chain,
			authority_set,
			//send_voter_commands,
			//consensus_changes,
		}
	}
}



impl<B, E, Block: BlockT<Hash=H256>, RA, SC> BlockImport<Block>
	for BadgerBlockImport<B, E, Block, RA, SC> where
		//NumberFor<Block>: grandpa::BlockNumberOps,
		B: Backend<Block, Blake2Hasher> + 'static,
		E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
		DigestFor<Block>: Encode,
		RA: Send + Sync,
{
	type Error = ConsensusError;

	fn import_block(
		&mut self,
		mut block: BlockImportParams<Block>,
		new_cache: HashMap<well_known_cache_keys::Id, Vec<u8>>,
	) -> Result<ImportResult, Self::Error> {
		let hash = block.post_header().hash();
		let number = block.header.number().clone();

		// early exit if block already in chain, probably a duplicate sent from another node
		match self.inner.status(BlockId::Hash(hash)) {
			Ok(blockchain::BlockStatus::InChain) => 
			{
			info!("Block already in chain");
			return Ok(ImportResult::AlreadyInChain)
		    },
			Ok(blockchain::BlockStatus::Unknown) => {},
			Err(e) => return Err(ConsensusError::ClientImport(e.to_string()).into()),
		}

		//we only import our own blocks, though catch up may be necessary later on
		match block.origin
		{
 	    // Genesis block built into the client.
		BlockOrigin::Genesis => {
			info!("Importing Genesis"); },
	    // Block is part of the initial sync with the network.
	    BlockOrigin::NetworkInitialSync => {
			  //ignore?
			  return Ok(ImportResult::AlreadyInChain);
	          },
	// Block was broadcasted on the network.
	BlockOrigin::NetworkBroadcast => {
		//ignore?
		return Ok(ImportResult::AlreadyInChain);
		},
	// Block that was received from the network and validated in the consensus process.
	BlockOrigin::ConsensusBroadcast =>
	{
		return Ok(ImportResult::AlreadyInChain);
	},
	// Block that was collated by this node.
	BlockOrigin::Own => {},
	// Block was imported from a file.
	BlockOrigin::File => {
		//ignore?
		return Ok(ImportResult::AlreadyInChain);
		},
		}

		
		// we  want to finalize on `inner.import_block`, probably
		//let mut justification = block.justification.take();
		//let enacts_consensus_change = !new_cache.is_empty();
		let import_result = (&*self.inner).import_block(block, new_cache);
		import_result
	}

	fn check_block(
		&mut self,
		block: BlockCheckParams<Block>,
	) -> Result<ImportResult, Self::Error> {
		self.inner.check_block(block)
	}
}




impl<B: BlockT, C, Pub, Sig> Verifier<B> for BadgerVerifier<C, Pub, Sig>
where
	C: ProvideRuntimeApi + Send + Sync + client::backend::AuxStore + ProvideCache<B>,
	C::Api: BlockBuilderApi<B>,
	//DigestItemFor<B>: CompatibleDigestItem<P>,
	Pub: Send + Sync + Hash + Eq + Clone + Decode + Encode + Debug,
	Sig: Encode + Decode + Send + Sync,
{
	fn verify(
		&mut self,
		origin: BlockOrigin,
		header: B::Header,
		justification: Option<Justification>,
		body: Option<Vec<B::Extrinsic>>,
	) -> Result<(BlockImportParams<B>, Option<Vec<(CacheKeyId, Vec<u8>)>>), String> {
		// dummy for the moment
		//let hash = header.hash();
		//let parent_hash = *header.parent_hash();
		let import_block = BlockImportParams {
			origin,
			header: header,
			post_digests: vec![],
			body,
			finalized: true,
			justification,
			allow_missing_state: true,
			auxiliary: Vec::new(),
			fork_choice: ForkChoiceStrategy::LongestChain,
		};
		info!("VERIFIER BADGER");
		Ok((import_block, None))
	}
}

/// Register the aura inherent data provider, if not registered already.
fn register_badger_inherent_data_provider(
	inherent_data_providers: &InherentDataProviders,
	_slot_duration: u64,
) -> Result<(), consensus_common::Error> {
	if !inherent_data_providers.has_provider(&srml_timestamp::INHERENT_IDENTIFIER) {
		inherent_data_providers
			.register_provider(srml_timestamp::InherentDataProvider)
			.map_err(Into::into)
			.map_err(consensus_common::Error::InherentData)
	} else {
		Ok(())
	}
}

/// Start an import queue for the Badger consensus algorithm.
pub fn badger_import_queue<B, C, Pub, Sig>(
	block_import: BoxBlockImport<B>,
	justification_import: Option<BoxJustificationImport<B>>,
	finality_proof_import: Option<BoxFinalityProofImport<B>>,
	client: Arc<C>,
	inherent_data_providers: InherentDataProviders,
) -> Result<BadgerImportQueue<B>, consensus_common::Error>
where
	B: BlockT,
	C: 'static + ProvideRuntimeApi + ProvideCache<B> + Send + Sync + AuxStore,
	C::Api: BlockBuilderApi<B>,
	//DigestItemFor<B>: CompatibleDigestItem<P>,
	Pub: Clone + Eq + Send + Sync + Hash + Debug + Encode + Decode + 'static,
	Sig: Encode + Decode + Send + Sync + 'static,
{
	register_badger_inherent_data_provider(&inherent_data_providers, 1)?;
	//initialize_authorities_cache(&*client)?;

	let verifier = BadgerVerifier::<C, Pub, Sig> {
		_client: client.clone(),
		_inherent_data_providers:inherent_data_providers,
		_pub: PhantomData,
		_sig: PhantomData,
	};

	Ok(BasicQueue::new(
		verifier,
		block_import,
		justification_import,
		finality_proof_import,
	))
}

use client::{
	//blockchain::Backend as BlockchainBackend,
	error::{ Result as ClientResult},
	light::fetcher::{ StorageProof},//FetchChecker, RemoteReadRequest,
};

use badger_primitives::AuthorityList;
/// Badger authority set getter? .
pub trait AuthoritySetGetter<Block: BlockT>: Send + Sync {
	/// Read HBBFT_AUTHORITIES_KEY from storage at given block.
	fn authorities(&self, block: &BlockId<Block>) -> ClientResult<AuthorityList>;
	/// Prove storage read of HBBFT_AUTHORITIES_KEY at given block.
	fn prove_authorities(&self, block: &BlockId<Block>) -> ClientResult<StorageProof>;
}

/// Client-based implementation of AuthoritySetForFinalityProver.
impl<B, E, Block: BlockT<Hash=H256>, RA> AuthoritySetGetter<Block> for Client<B, E, Block, RA>
	where
		B: Backend<Block, Blake2Hasher> + Send + Sync + 'static,
		E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
		RA: Send + Sync,
{
	fn authorities(&self, block: &BlockId<Block>) -> ClientResult<AuthorityList> {
		let storage_key = StorageKey(HBBFT_AUTHORITIES_KEY.to_vec());
		self.storage(block, &storage_key)?
			.and_then(|encoded| AuthorityList::decode(&mut encoded.0.as_slice()).ok())
			.map(|versioned| versioned.into())
			.ok_or(ClientError::InvalidAuthoritiesSet)
	}

	fn prove_authorities(&self, block: &BlockId<Block>) -> ClientResult<StorageProof> {
		self.read_proof(block, iter::once(HBBFT_AUTHORITIES_KEY))
	}
}


/// Configuration for the Badger service.
#[derive(Clone)]
pub struct Config {
	/// Some local identifier of the node.
	pub name: Option<String>,
	pub batch_size: u32,
//	pub initial_validators: BTreeMap<PeerIdW, PublicKey>, replaced by session aspects
//	pub node_indices: BTreeMap<PeerIdW, usize>, unnecessary
}

fn _secret_share_from_string(st: &str) -> Result<SecretKeyShare, Error> {
	let data = hex::decode(st)?;
	match bincode::deserialize(&data) {
		Ok(val) => Ok( val ),
		Err(_) => return Err(Error::Badger("secret key share binary invalid".to_string())),
	}
}

impl Config {
	fn _name(&self) -> &str {
		self.name
			.as_ref()
			.map(|s| s.as_str())
			.unwrap_or("<unknown>")
	}
	
}

/// Errors that can occur while voting in BADGER.
#[derive(Debug)]
pub enum Error {
	/// An error within badger.
	Badger(String),
	/// A network error.
	Network(String),
	/// A blockchain error.
	Blockchain(String),
	/// Could not complete a round on disk.
	Client(ClientError),
	/// An invariant has been violated (e.g. not finalizing pending change blocks in-order)
	Safety(String),
	/// A timer failed to fire.
	Timer(tokio_timer::Error),

}

impl From<hex::FromHexError> for Error {
	fn from(e: hex::FromHexError) -> Self {
		Error::Badger(e.to_string())
	}
}
impl From<ClientError> for Error {
	fn from(e: ClientError) -> Self {
		Error::Client(e)
	}
}

/// Something which can determine if a block is known.
pub trait BlockStatus<Block: BlockT> {
	/// Return `Ok(Some(number))` or `Ok(None)` depending on whether the block
	/// is definitely known and has been imported.
	/// If an unexpected error occurs, return that.
	fn block_number(&self, hash: Block::Hash) -> Result<Option<NumberFor<Block>>, Error>;
}
pub trait BlockNumberOps:
	std::fmt::Debug
	+ std::cmp::Ord
	+ std::ops::Add<Output = Self>
	+ std::ops::Sub<Output = Self>
	+ num::One
	+ num::Zero
	+ num::AsPrimitive<usize>
{
}

impl<T> BlockNumberOps for T
where
	T: std::fmt::Debug,
	T: std::cmp::Ord,
	T: std::ops::Add<Output = Self>,
	T: std::ops::Sub<Output = Self>,
	T: num::One,
	T: num::Zero,
	T: num::AsPrimitive<usize>,
{
}

impl<B, E, Block: BlockT<Hash = H256>, RA> BlockStatus<Block> for Arc<Client<B, E, Block, RA>>
where
	B: Backend<Block, Blake2Hasher>,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync,
	RA: Send + Sync,
	NumberFor<Block>: BlockNumberOps,
{
	fn block_number(&self, hash: Block::Hash) -> Result<Option<NumberFor<Block>>, Error> {
		self.block_number_from_id(&BlockId::Hash(hash))
			.map_err(|e| Error::Blockchain(format!("{:?}", e)))
	}
}
use communication::BadgerHandler;

fn global_communication<Block: BlockT<Hash = H256>, B, E, N, RA>(
	_client: &Arc<Client<B, E, Block, RA>>,
	network: NetworkBridge<Block, N>,
) -> (
	impl Stream<Item = <QHB as ConsensusProtocol>::Output>,
	impl SendOut,
	impl BadgerHandler
)
where
	B: Backend<Block, Blake2Hasher>,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync,
	N: Network<Block>,
	RA: Send + Sync,
	NumberFor<Block>: BlockNumberOps,
{
	let is_voter = network.is_validator();

	// verification stream
	let (global_in, global_out,handler) = network.global_communication(
		//	voters.clone(),
		is_voter,
	);

	// block commit and catch up messages until relevant blocks are imported.
	/*let global_in = UntilGlobalMessageBlocksImported::new(
			client.import_notification_stream(),
			client.clone(),
			global_in,
	);*/ //later

	(global_in, global_out,handler)
}

/// Parameters used to run Honeyed Badger.
pub struct BadgerStartParams<Block: BlockT<Hash = H256>, N: Network<Block>, X> {
	/// Configuration for the Badger service.
	pub config: Config,
	/// A link to the block import worker.
	//pub link: LinkHalf<B, E, Block, RA, SC>,
	/// The Network instance.
	pub network: N,
	/// The inherent data providers.
	pub inherent_data_providers: InherentDataProviders,
	/// Handle to a future that will resolve on exit.
	pub on_exit: X,
	/// If supplied, can be used to hook on telemetry connection established events.
	//	pub telemetry_on_connect: Option<TelemetryOnConnect>,
	ph: PhantomData<Block>,
}

pub struct TxStream<A, B, E, RA, Block: BlockT<Hash = H256>>
where
	A: txpool::ChainApi,
	B: client::backend::Backend<Block, Blake2Hasher> + Send + Sync + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + Clone + 'static,
	RA: ConstructRuntimeApi<Block, Client<B, E, Block, RA>>,
{
	pub transaction_pool: Arc<TransactionPool<A>>,
	pub client: Arc<Client<B, E, Block, RA>>,
}

impl<A, B, E, RA, Block: BlockT<Hash = H256>> Stream for TxStream<A, B, E, RA, Block>
where
	A: txpool::ChainApi,
	<A as txpool::ChainApi>::Block: BlockT<Hash = H256>,
	B: client::backend::Backend<Block, Blake2Hasher> + Send + Sync + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + Clone + 'static,
	RA: ConstructRuntimeApi<Block, Client<B, E, Block, RA>>,
{
	type Item = TransactionSet;
	fn poll_next(
		self: Pin<&mut Self>,
		_cx: &mut futures03::task::Context,
	) -> Poll<Option<Self::Item>> {
		trace!("BADgER! Polled stream!");
		let pending_iterator = self.transaction_pool.ready();
		let batch: Vec<_> = pending_iterator.map(|a| a.data.encode()).collect();
		let pending_iterator = self.transaction_pool.ready();
		let tags: Vec<_> = pending_iterator
			.flat_map(|a| a.provides.clone().into_iter())
			.collect();
		let pending_iterator = self.transaction_pool.ready();
		let hashes: Vec<_> = pending_iterator.map(|a| a.hash.clone()).collect();
		if batch.len() == 0 {
			return Poll::Pending;
		}
		info!("BADgER! Ready stream!");
		let best_block_hash = self.client.info().chain.best_hash;
		futures03::executor::block_on(self.transaction_pool
			.prune_tags(&generic::BlockId::hash(best_block_hash), tags, hashes)).unwrap();
		Poll::Ready(Some(batch))
	}
}

pub struct BadgerProposerWorker<S, Block: BlockT<Hash = H256>, I, B, E, RA, SC>
where
	S: Stream<Item = <QHB as ConsensusProtocol>::Output>,
	//TF: Sink<TransactionSet>+Unpin,
	//A: txpool::ChainApi,
	B: client::backend::Backend<Block, Blake2Hasher> + Send + Sync + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + Clone + 'static,
	RA: ConstructRuntimeApi<Block, Client<B, E, Block, RA>>,
	//N : Network<Block> + Send + Sync + 'static,
	NumberFor<Block>: BlockNumberOps,
	SC: SelectChain<Block> + Clone,
	I: BlockImport<Block>,
{
	pub block_out: S,
	//pub transaction_in: TF,
	//pub transaction_out: TxStream<A>,//Arc<TransactionPool<A>>,
	//pub network: N,
	pub client: Arc<Client<B, E, Block, RA>>,
	pub block_import: Arc<Mutex<I>>,
	pub inherent_data_providers: InherentDataProviders,
	pub select_chain: SC,
	phb: PhantomData<Block>,
}

use crate::aux_store::GenesisAuthoritySetProvider;


pub fn block_importer<B, E, Block: BlockT<Hash=H256>, RA, SC>(
	client: Arc<Client<B, E, Block, RA>>,
	genesis_authorities_provider: &dyn GenesisAuthoritySetProvider<Block>,
	select_chain: SC,
) -> Result<BadgerBlockImport<B, E, Block, RA, SC>, ClientError>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
	RA: Send + Sync,
	SC: SelectChain<Block>,
{
	let chain_info = client.info();
	let genesis_hash = chain_info.chain.genesis_hash;

	let persistent_data = aux_store::loads_auth_set(
		&*client,
		|| {
			let authorities = genesis_authorities_provider.get()?;
			telemetry!(CONSENSUS_INFO; "afg.loading_authorities";
				"authorities_len" => ?authorities.len()
			);
			Ok(authorities)
		}
	)?;

	
	Ok(		BadgerBlockImport::new(
			client.clone(),
			select_chain.clone(),
			persistent_data.clone(),
		),
		
	)
}


/// Run a HBBFT churn as a task. Provide configuration and a link to a
/// block import worker that has already been instantiated with `block_import`.
pub fn run_honey_badger<B, E, Block: BlockT<Hash = H256>, N, RA, SC, X, I, A>(
	client: Arc<Client<B, E, Block, RA>>,
	t_pool: Arc<TransactionPool<A>>,
	config: Config,
	network: N,
	on_exit: X,
	block_import: Arc<Mutex<I>>,
	inherent_data_providers: InherentDataProviders,
	selch: SC,
	keystore: KeyStorePtr,
	_node_key:Option<String>,
	_dev_seed:Option<String>
) -> ::client::error::Result<impl Future<Output = ()> + Send + Unpin>
where
	Block::Hash: Ord,
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + 'static + Clone,
	N: Network<Block> + Send + Sync + Unpin,
	N::In: Send,
	SC: SelectChain<Block> + 'static + Unpin,
	NumberFor<Block>: BlockNumberOps,
	DigestFor<Block>: Encode,
	RA: Send + Sync + 'static,
	X: futures03::future::Future<Output = ()> + Send + Unpin,
	A: txpool::ChainApi + 'static,
	<A as txpool::ChainApi>::Block: BlockT<Hash = H256>,
	I: BlockImport<Block> + Send + Sync + 'static,
	RA: ConstructRuntimeApi<Block, Client<B, E, Block, RA>>,
	<Client<B, E, Block, RA> as ProvideRuntimeApi>::Api: BlockBuilderApi<Block>,
{
	let genesis_authorities_provider= &*client.clone();
	let persistent_data = aux_store::load_persistent_badger(
		&*client,
		|| {
			let authorities = genesis_authorities_provider.get()?;
			telemetry!(CONSENSUS_INFO; "afg.loading_authorities";
				"authorities_len" => ?authorities.len()
			);
			Ok(authorities)
		},keystore.clone()
	)?;
	info!("Badger AUTH {:?}",&persistent_data.authority_set.inner);

	let auth_ref:Arc<parking_lot::RwLock<aux_store::AuthoritySet>>=persistent_data.authority_set.inner.clone();
	let (network_bridge, network_startup) = NetworkBridge::new(network, config.clone(),keystore.clone(),persistent_data);

	//let PersistentData { authority_set, set_state, consensus_changes } = persistent_data;

	//	register_finality_tracker_inherent_data_provider(client.clone(), &inherent_data_providers)?;
	let (blk_out, tx_in,handler ) = global_communication(&client, network_bridge);
	let txcopy = Arc::new(parking_lot::RwLock::new(tx_in));
	let tx_in_arc = txcopy.clone();

	let tx_out = TxStream {
		transaction_pool: t_pool.clone(),
		client: client.clone(),
	};
	let sender = tx_out.for_each(move |data: std::vec::Vec<std::vec::Vec<u8>>| {
		{
			let mut lock = tx_in_arc.write();
			match lock.send_out(data) {
				Ok(_) => {}
				Err(_) => {
					debug!("Well, this is weird");
				}
			}
		}
		future::ready(())
	});
	let cclient = client.clone();
	let cblock_import = block_import.clone();
	let ping_sel = selch.clone();
	let receiver = blk_out.for_each(move |batch| {
		info!("[[[[[[[");
		let mut inherent_digests = generic::Digest { logs: vec![] };
		//
		info!(
			"Processing batch with epoch {:?} of {:?} transactions into blocks",
			batch.epoch(),
			batch.len(),			
		);
		match batch.change()
		{
			ChangeState::None => {},
			ChangeState::InProgress(Change::NodeChange(pubkeymap)) => 
			{
			 //change in progress, broadcast messages have to be sent to new node as well
			 info!("CHANGE: in progress");
			 handler.notify_node_set(pubkeymap.iter().map(|(v,_)| v.clone().into() ).collect() )
			},
			ChangeState::InProgress(Change::EncryptionSchedule(_)) => {},//don't care?
			ChangeState::Complete(Change::NodeChange(pubkeymap)) => {
				info!("CHANGE: complete {:?}",&pubkeymap);
				let digest=BadgerPreRuntime::ValidatorsChanged(pubkeymap.iter().map(|(_,v)| v.clone().into() ).collect());
				let logs=vec![DigestItem::PreRuntime(HBBFT_ENGINE_ID, digest.encode())];
				inherent_digests.logs=logs;
				//update internals before block is created?
				handler.update_validators(pubkeymap.iter().
			    map(| (c,v)| (c.clone().into(),v.clone().into()) ).collect() ,&*cclient);
			},
			ChangeState::Complete(Change::EncryptionSchedule(_)) => {},//don't care?
			
		
		}
		let chain_head = match selch.best_chain() {
			Ok(x) => x,
			Err(e) => {
				warn!(target: "formation", "Unable to author block, no best block header: {:?}", e);
				return future::ready(());
			}
		};
		let parent_hash = chain_head.hash();
		let mut pnumber = *chain_head.number();
		let mut parent_id = BlockId::hash(parent_hash);
		let mut block_builder = match cclient.new_block_at(&parent_id, inherent_digests.clone()) {
			Ok(val) => val,
			Err(_) => {
				warn!("Error in new_block_at");
				return future::ready(());
			}
		};

		// proceed with transactions
		let mut is_first = true;
		//let mut skipped = 0;
		info!(
			"Attempting to push transactions from the batch. {}",
			batch.len()
		);

		for pending in batch.iter() {
			let data: Result<<Block as BlockT>::Extrinsic, _> =
				Decode::decode(&mut pending.as_slice());
			//   <Transaction<ExHash<A>,<Block as BlockT>::Extrinsic> as Decode>::decode(&mut pending.as_slice());
			let data = match data {
				Ok(val) => val,
				Err(_) => {
					info!("Data decoding error");
					continue;
				}
			};
			//a.data.encode()
			info!("[{:?}] Pushing to the block.", pending);
			info!("[{:?}] Data ", &data);
			match client::block_builder::BlockBuilder::push(&mut block_builder, data.clone()) {
				Ok(()) => {
					debug!("[{:?}] bytes Pushed to the block.", pending.len());
				}
				Err(error::Error::ApplyExtrinsicFailed(ApplyError::BadState)) => {
					if is_first {
						debug!(
							"[{:?}] Invalid transaction: FullBlock on empty block",
							pending.len()
						);
					//	unqueue_invalid.push(pending.hash.clone());
					} else {
						debug!("Block is full, proceed with proposing.");
						let block = match block_builder.bake() {
							Ok(val) => val,
							Err(e) => {
								warn!("Block baking error {:?}", e);
								return future::ready(());
							}
						};
						info!(
							"Prepared block for proposing at {} [hash: {:?}; parent_hash: {}; extrinsics: [{}]]",
							block.header().number(),
							<Block as BlockT>::Hash::from(block.header().hash()),
							block.header().parent_hash(),
							block
								.extrinsics()
								.iter()
								.map(|xt| format!("{}", BlakeTwo256::hash_of(xt)))
								.collect::<Vec<_>>()
								.join(", ")
						);
						telemetry!(CONSENSUS_INFO; "prepared_block_for_proposing";
						"number" => ?block.header().number(),
						"hash" => ?<Block as BlockT>::Hash::from(block.header().hash()),
						);
						if Decode::decode(&mut block.encode().as_slice()).as_ref() != Ok(&block) {
							error!("Failed to verify block encoding/decoding");
						}

						if let Err(err) =
							evaluation::evaluate_initial(&block, &parent_hash, pnumber)
						{
							error!("Failed to evaluate authored block: {:?}", err);
						}
						let (header, body) = block.deconstruct();

						let header_num = header.number().clone();
						let parent_hash; //= header.parent_hash().clone();

						// sign the pre-sealed hash of the block and then
						// add it to a digest item.
						let header_hash = header.hash();

						let import_block: BlockImportParams<Block> = BlockImportParams {
							origin: BlockOrigin::Own,
							header,
							justification: None,
							post_digests: vec![],
							body: Some(body),
							finalized: true,
							allow_missing_state:true,
							auxiliary: Vec::new(),
							fork_choice: ForkChoiceStrategy::LongestChain,
						};

						info!(
							"Pre-sealed block for proposal at {}. Hash now {:?}, previously {:?}.",
							header_num,
							import_block.post_header().hash(),
							header_hash
						);
						telemetry!(CONSENSUS_INFO; "badger.pre_sealed_block";
							"header_num" => ?header_num,
							"hash_now" => ?import_block.post_header().hash(),
							"hash_previously" => ?header_hash
						);
						parent_hash = import_block.post_header().hash();
						pnumber = *import_block.post_header().number();
						parent_id = BlockId::hash(parent_hash);
						// go on to next block
						{
							let eh = import_block.header.parent_hash().clone();
							if let Err(e) = cblock_import
								.lock()
								.import_block(import_block, Default::default())
							{
								warn!(target: "badger", "Error with block built on {:?}: {:?}",eh, e);
								telemetry!(CONSENSUS_WARN; "mushroom.err_with_block_built_on";
							"hash" => ?eh, "err" => ?e);
							}
							block_builder = cclient
								.new_block_at(&parent_id, inherent_digests.clone())
								.unwrap();
							is_first = true;
							continue;
						}
					}
				}
				Err(e) => {
					info!("[{:?}] Invalid transaction: {}", pending.len(), e);
					//unqueue_invalid.push(pending.hash.clone());
				}
			}

			is_first = false;
		}

		if !is_first {
			info!("BADger: importing block");
			{
				debug!("Block is done, proceed with proposing.");
				let block = match block_builder.bake() {
					Ok(val) => val,
					Err(e) => {
						warn!("Block baking error {:?}", e);
						return future::ready(());
					}
				};
				info!(
					"Prepared block for proposing at {} [hash: {:?}; parent_hash: {}; extrinsics: [{}]]",
					block.header().number(),
					<Block as BlockT>::Hash::from(block.header().hash()),
					block.header().parent_hash(),
					block
						.extrinsics()
						.iter()
						.map(|xt| format!("{}", BlakeTwo256::hash_of(xt)))
						.collect::<Vec<_>>()
						.join(", ")
				);
				telemetry!(CONSENSUS_INFO; "prepared_block_for_proposing";
				"number" => ?block.header().number(),
				"hash" => ?<Block as BlockT>::Hash::from(block.header().hash()),
				);
				if Decode::decode(&mut block.encode().as_slice()).as_ref() != Ok(&block) {
					error!("Failed to verify block encoding/decoding");
				}

				if let Err(err) = evaluation::evaluate_initial(&block, &parent_hash, pnumber) {
					error!("Failed to evaluate authored block: {:?}", err);
				}
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
					sid=auth_ref.read().self_id.clone();
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
						   match handler.vote_for_validators(changeset,&*cclient)
						   {
							   Ok(_) =>{},
							   Err(e) =>{
								   info!("VoteErr: {:?}",e);
							   }
						   }
                           info!("VOTEDED");
						 }
						},
						ConsensusLog::NotifyChangedSet(newset) =>
						{
							info!("Notified of new set {:?}",&newset);
						}

					}
				}

				// sign the pre-sealed hash of the block and then
				// add it to a digest item.
				let header_hash = header.hash();

				let import_block: BlockImportParams<Block> = BlockImportParams {
					origin: BlockOrigin::Own,
					header,
					justification: None,
					post_digests: vec![],
					body: Some(body),
					allow_missing_state:true,
					finalized: true,
					auxiliary: Vec::new(),
					fork_choice: ForkChoiceStrategy::LongestChain,
				};

				info!(
					"Pre-sealed block for proposal at {}. Hash now {:?}, previously {:?}.",
					header_num,
					import_block.post_header().hash(),
					header_hash
				);
				telemetry!(CONSENSUS_INFO; "badger.pre_sealed_block";
					"header_num" => ?header_num,
					"hash_now" => ?import_block.post_header().hash(),
					"hash_previously" => ?header_hash
				);
				parent_hash = import_block.post_header().hash();
				//pnumber = *import_block.post_header().number();
				//parent_id = BlockId::hash(parent_hash);
				// go on to next block
				{
					let eh = import_block.header.parent_hash().clone();
					if let Err(e) = cblock_import
						.lock()
						.import_block(import_block, Default::default())
					{
						warn!(target: "badger", "Error with block built on {:?}: {:?}",eh, e);
						telemetry!(CONSENSUS_WARN; "mushroom.err_with_block_built_on";
							"hash" => ?eh, "err" => ?e);
					}
				}
				
			}
		}
		info!("[[[[[[[--]]]]]]]");

		future::ready(())
	});

	//.map(|_| ()).map_err(|e|
	//    {
	//			warn!("BADGER failed: {:?}", e); 
	//			telemetry!(CONSENSUS_WARN; "afg.badger_failed"; "e" => ?e);
	//		}) ;

	
//	use hex_literal::*;
//	use substrate_primitives::crypto::Pair;
//	let ap:app_crypto::hbbft_thresh::Public=hex!["946252149ad70604cf41e4b30db13861c919d7ed4e8f9bd049958895c6151fab8a9b0b027ad3372befe22c222e9b733f"].into();

//	let secr:SecretKey=bincode::deserialize(&keystore.read().key_pair_by_type::<AuthorityPair>(&ap.into(), app_crypto::key_types::HB_NODE).unwrap().to_raw_vec()).unwrap();
//	info!("Badger AUTH  private {:?}",&secr);
	
	let with_start = network_startup.then(move |()| futures03::future::join(sender, receiver));
	let ping_client = client.clone();
	// Delay::new(Duration::from_secs(1)).then(|_| {
	let ping = Interval::new(Duration::from_millis(11500)).for_each(move |_| {

		//put inherents here for now
		let chain_head = match ping_sel.best_chain() {
			Ok(x) => x,
			Err(e) => {
				warn!(target: "formation", "Unable to author block, no best block header: {:?}", e);
				return future::ready(());
			}
		};
		let  parent_hash = chain_head.hash();
		//let  pnumber = *chain_head.number();
		let  parent_id = BlockId::hash(parent_hash);

		let inherent_data = match inherent_data_providers.create_inherent_data() {
			Ok(id) => id,
			Err(_) => return future::ready(()), //future::err(err),
		};
		//empty for now?

		// We don't check the API versions any further here since the dispatch compatibility
		// check should be enough.
		// do this only once?
		let inh = ping_client.runtime_api().inherent_extrinsics_with_context(
			&parent_id,
			ExecutionContext::BlockConstruction,
			inherent_data,
		);
		if let Ok(res) = inh {
			let mut lock = txcopy.write();
			info!("This many INHERS {:?}", res.len());
			for extrinsic in res {
				info!("INHER {:?}", &extrinsic);
				match lock.send_out(vec![extrinsic.encode().into_iter().collect()]) {
					Ok(_) => {}
					Err(_) => {
						warn!("Error in ping sendout");
						return future::ready(());
					}
				}
			}
		} else {
			info!("Inherent panic {:?}", inh);
		}
		info!("Ping done");
		future::ready(())
	});

	//let jn = ping.merge(t_pool.clone().import_notification_stream());
	let pinged = futures03::future::select(with_start, ping);
	// Make sure that `telemetry_task` doesn't accidentally finish and kill grandpa.

	Ok(futures03::future::select(
		on_exit.then(|_| {
			info!("READY");
			future::ready(())
		}),
		pinged,
	)
	.then(|_| future::ready(())))
	/*let ping_lesser = Interval::new(Duration::from_millis(1000)).for_each(move |_| {
		info!("ping");
				let mut chain_head = match ping_sel.best_chain()
		{
			Ok(x) => x,
			Err(e) =>
			{
				warn!(target: "formation", "Unable to author block, no best block header: {:?}", e);
				return future::ready(());
			}
		};
	let mut parent_hash = chain_head.hash();
		let mut pnumber = *chain_head.number();
		let mut parent_id = BlockId::hash(parent_hash);

		let inherent_data = match inherent_data_providers.create_inherent_data()
		{
			Ok(id) => id,
			Err(err) => return future::ready(()), //future::err(err),
		};
	let inh = ping_client.runtime_api().inherent_extrinsics_with_context(
			&parent_id,
			ExecutionContext::BlockConstruction,
			inherent_data,
		);
			info!("ping end");
		future::ready(())
		});

	let ready_on_exit= on_exit.then(|_| {
				info!("READY");
				future::ready(())
			});
	Ok(  futures03::future::select( ping_lesser,ready_on_exit)  .then(|_| future::ready(())))*/
}