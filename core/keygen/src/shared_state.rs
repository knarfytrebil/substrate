use std::{
	//collections::VecDeque,
	fmt::Debug,
	//marker::PhantomData,
	//sync::Arc,
	//time::{Duration, Instant},
};

//use client::blockchain::HeaderBackend;
use client::{
	backend::{AuxStore, },// Backend},
	error::Error as ClientError,
	error::Result as ClientResult,
//	BlockchainEvents, CallExecutor, Client,
};
use codec::{Decode, Encode};
//use consensus_common::SelectChain;
//use futures::{future::Loop as FutureLoop, prelude::*, stream::Fuse, sync::mpsc};
//use inherents::InherentDataProviders;
use log::{debug, error, info, warn};
use network::{self, PeerId};
//use parking_lot::RwLock;
use sr_primitives::generic::BlockId;
use sr_primitives::traits::{Block as BlockT, DigestFor, NumberFor, ProvideRuntimeApi};

const SIGNER_SET_KEY: &[u8] = b"multi_ecdsa_signers";

#[derive(Debug, Encode, Decode)]
pub struct SharedState {
	pub signer_set: Vec<Vec<u8>>,
}

pub(crate) fn load_decode<B: AuxStore, T: Decode>(
	backend: &B,
	key: &[u8],
) -> ClientResult<Option<T>> {
	match backend.get_aux(key)? {
		None => Ok(None),
		Some(t) => T::decode(&mut &t[..])
			.map_err(|e| ClientError::Backend(format!("Multi ecdsa DB is corrupted: {}", e.what())))
			.map(Some),
	}
}

pub(crate) fn load_persistent<B>(backend: &B) -> ClientResult<SharedState>
where
	B: AuxStore,
{
	let res: Option<Vec<Vec<u8>>> = load_decode(backend, SIGNER_SET_KEY)?;
	match res {
		Some(signer_set) => Ok(SharedState { signer_set }),
		None => Ok(SharedState {
			signer_set: Vec::new(),
		}),
	}
}

pub(crate) fn set_signers<B: AuxStore>(backend: &B, signers: Vec<Vec<u8>>) -> ClientResult<()> {
	backend.insert_aux(
		&[(SIGNER_SET_KEY, Encode::encode(&signers).as_slice())],
		&[],
	)
}
