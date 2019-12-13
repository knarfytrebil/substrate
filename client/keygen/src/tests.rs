use client::{
	error::Result,
	runtime_api::{ApiExt, Core, RuntimeVersion},
	LongestChain,
};
use codec::Decode;
use consensus_common::import_queue::{
	BoxBlockImport, BoxFinalityProofImport, BoxJustificationImport,
};
use consensus_common::{
	BlockImportParams, BlockOrigin, ForkChoiceStrategy, ImportResult, ImportedAux,
};
use futures::prelude::*;
use futures03::{StreamExt, TryStreamExt};

use keyring::Ed25519Keyring;
use network::config::{BoxFinalityProofRequestBuilder, ProtocolConfig, Roles};
use network::test::PassThroughVerifier;
use network::test::{Block, DummySpecialization, Hash, Peer, PeersClient, TestNetFactory};
use parking_lot::Mutex;
use primitives::{crypto::Public, ExecutionContext, NativeOrEncoded};
use sr_primitives::generic::BlockId;
use sr_primitives::traits::{ApiRef, Header as HeaderT, ProvideRuntimeApi};
use std::collections::{HashMap, HashSet};
use std::result;
use test_client::{self, runtime::BlockNumber};
use tokio::runtime::current_thread;

use super::*;

pub struct TestNet {
	peers: Vec<Peer<(), DummySpecialization>>,
}

impl TestNet {
	fn new(n_peers: usize) -> Self {
		let mut net = Self {
			peers: Vec::with_capacity(n_peers),
		};
		let mut config = Self::default_config();
		config.roles = Roles::AUTHORITY;
		for _ in 0..n_peers {
			net.add_full_peer(&config);
		}
		net
	}
}

impl TestNetFactory for TestNet {
	type Specialization = DummySpecialization;
	type Verifier = PassThroughVerifier;
	type PeerData = ();

	/// Create new test network with peers and given config.
	fn from_config(_config: &ProtocolConfig) -> Self {
		TestNet { peers: Vec::new() }
	}

	fn make_verifier(
		&self,
		_client: PeersClient,
		_config: &ProtocolConfig,
		_peer_data: &Self::PeerData,
	) -> Self::Verifier {
		PassThroughVerifier(false)
	}

	fn peer(&mut self, i: usize) -> &mut Peer<(), Self::Specialization> {
		&mut self.peers[i]
	}

	fn peers(&self) -> &Vec<Peer<(), Self::Specialization>> {
		&self.peers
	}

	fn mut_peers<F: FnOnce(&mut Vec<Peer<(), Self::Specialization>>)>(&mut self, closure: F) {
		closure(&mut self.peers);
	}
}

fn create_keystore(authority: Ed25519Keyring) -> (KeyStorePtr, tempfile::TempDir) {
	let keystore_path = tempfile::tempdir().expect("Creates keystore path");
	let keystore = keystore::Store::open(keystore_path.path(), None).expect("Creates keystore");
	(keystore, keystore_path)
}

#[test]
fn test_key_gen() {
	use super::*;

	let peers = &[
		Ed25519Keyring::Alice,
		Ed25519Keyring::Bob,
		Ed25519Keyring::Charlie,
		Ed25519Keyring::Dave,
		// Ed25519Keyring::Eve,
		// Ed25519Keyring::Ferdie,
	];

	let peers_len = peers.len();
	let blocks = 50;

	let mut runtime = current_thread::Runtime::new().unwrap();

	let net = TestNet::new(peers_len);
	let net = Arc::new(Mutex::new(net));

	let mut notifications = Vec::new();

	let all_peers = peers.iter().cloned();

	for (peer_id, local_key) in all_peers.enumerate() {
		let net = net.lock();
		let client = net.peers[peer_id].client().clone();
		let network = net.peers[peer_id].network_service().clone();
		let local_peer_id = network.local_peer_id();

		let (keystore, keystore_path) = create_keystore(local_key);

		// if peer_id != 0 {
		notifications.push(
			client
				.import_notification_stream()
				.map(move |n| Ok::<_, ()>(n))
				.compat()
				.take_while(|n| Ok(n.header.number() < &blocks))
				.for_each(move |_| Ok(())),
		);
		// }

		let full_client = client.as_full().unwrap();
		let node = run_key_gen(
			local_peer_id,
			(1, peers_len as u16),
			1,
			keystore,
			full_client,
			network,
		)
		.unwrap()
		.compat(); // compat future03 -> 01
		runtime.spawn(node);
	}

	let sync = futures::future::poll_fn(|| {
		// make sure all peers are connected first
		net.lock().poll();
		if net.lock().peer(0).num_peers() < peers_len - 1 {
			Ok(Async::NotReady)
		} else {
			Ok(Async::Ready(()))
		}
	});

	runtime.block_on(sync.map_err(|_: ()| ())).unwrap();

	{
		// at this time, the blocks are not synced
		// so only peer 0 has 2 blocks
		// but peer 0 doesn't get import_notifications since "block origin" is File
		let mut net = net.lock();
		net.peer(0).push_blocks(blocks as usize, false);
		assert_eq!(net.peer(0).client().info().chain.best_number, blocks);
		assert_eq!(net.peer(1).client().info().chain.best_number, 0);
	}

	let wait_for = futures::future::join_all(notifications)
		.map(|_| ())
		.map_err(|_| ());

	let poll_forever = futures::future::poll_fn(|| {
		net.lock().poll();
		Ok(Async::NotReady)
	});
	let _ = runtime
		.block_on(wait_for.select(poll_forever).map_err(|_| ()))
		.unwrap();
}
