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
	RequestForSig(u64, Vec<u8>), // id, data
	#[codec(index = "2")]
	RequestForKey(u64),
}

pub type RequestId = u64;

pub type AuthorityId = crypto::Public;

// sp_api::decl_runtime_apis! {
// 	pub trait MpcApi {
// 		fn test();
// 	}
// }
