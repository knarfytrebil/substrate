#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Decode, Encode};
use sp_core::crypto::KeyTypeId;
use sp_runtime::{ConsensusEngineId, RuntimeDebug};
use sp_std::vec::Vec;

pub const MPC_ENGINE_ID: ConsensusEngineId = *b"MPCE";

pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"mpc_");

pub mod crypto {
	use super::KEY_TYPE;
	use sp_runtime::app_crypto::{app_crypto, sr25519};
	app_crypto!(sr25519, KEY_TYPE);
}

#[derive(Decode, Encode, RuntimeDebug)]
pub enum ConsensusLog {
	#[codec(index = "1")]
	RequestForSig(u64, u64, Vec<u8>),
	#[codec(index = "2")]
	RequestForKey(u64),
}

pub type RequestId = u64;

pub type GeneratedKeyId = u64;

pub type AuthorityId = crypto::Public;

#[derive(Clone, Decode, Encode, RuntimeDebug)]
pub enum MpcRequest {
	KeyGen(RequestId),
	SigGen(RequestId, GeneratedKeyId, Vec<u8>), // id, generated key id, data
}

pub enum OffchainStorageType {
	LocalSecretKey,
	SharedPublicKey,
	Signature,
}

/*
Keygen:
	run key gen, save local key & shared key

Siggen:
	get local key, run sig gen, save sig
*/

pub fn get_storage_key(id: u64, ost: OffchainStorageType) -> Vec<u8> {
	let mut k = Vec::new();

	match ost {
		OffchainStorageType::LocalSecretKey => {
			k.extend(b"mpc/sk/");
		}
		OffchainStorageType::SharedPublicKey => {
			k.extend(b"mpc/pk/");
		}
		OffchainStorageType::Signature => {
			k.extend(b"mpc/sig/");
		}
	}
	k.extend(&id.to_le_bytes());
	k
}

// sp_api::decl_runtime_apis! {
// 	pub trait MpcApi {
// 		fn test();
// 	}
// }
