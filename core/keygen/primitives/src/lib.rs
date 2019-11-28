#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use serde::Serialize;

use codec::{ Decode, Encode,  }; //Codec Input Error as CodecError,
use client::decl_runtime_apis;
use rstd::vec::Vec;
use sr_primitives::ConsensusEngineId;

pub const MP_ECDSA_ENGINE_ID: ConsensusEngineId = *b"MPEC";

pub const MAIN_DB_PREFIX: &[u8] = b"srml/keygen/request_";



pub type AuthorityWeight = u64;

pub type AuthorityIndex = u64;
pub type RequestId=u64;

pub fn get_data_prefix(request_id:RequestId) ->Vec<u8>
{
		let mut key:Vec<u8>=MAIN_DB_PREFIX.to_vec();
		key.append(&mut b"_data".to_vec());
        key.append(&mut request_id.encode());
		key
}

pub fn get_key_prefix(request_id:RequestId) ->Vec<u8>
{
		let mut key:Vec<u8>=MAIN_DB_PREFIX.to_vec();
		key.append(&mut b"_key".to_vec());
        key.append(&mut request_id.encode());
		key
}

pub fn get_complete_list_prefix() ->Vec<u8>
{
		let mut key:Vec<u8>=MAIN_DB_PREFIX.to_vec();
		key.append(&mut b"_crequests".to_vec());
       key
}

pub use sr_primitives::{ RuntimeDebug};

#[cfg_attr(feature = "std", derive(Serialize))]
#[derive(Decode, Encode, PartialEq, Eq, Clone,RuntimeDebug)]
pub enum ConsensusLog {

	///  Request for a new key to be generated, with provided requestid
	#[codec(index = "1")]
	RequestForKeygen( (u64,Vec<u8>)),

}

impl ConsensusLog  {
	/// Try to cast the log entry as a contained signal.
	pub fn try_into_vec(self) -> Option< (u64,Vec<u8>) > {
		match self {
			ConsensusLog::RequestForKeygen( ( id,change) ) => Some((id,change)),
			//_ => None,
		}
	}

	
}

pub const PENDING_CHANGE_CALL: &str = "hbbft_pending_change";
pub const AUTHORITIES_CALL: &str = "hbbft_authorities";
pub const GET_THRESHOLD_SIGNATURE_CALL: &str = "get_threshold_signature";
pub const REQ_THRESHOLD_SIGNATURE_CALL: &str = "req_threshold_signature";

decl_runtime_apis! { // TODO implement srml module
	#[api_version(2)]
	pub trait MpecApi {
		fn req_key_gen();
		fn req_threshold_signature(req_id: Vec<u8>, data: Vec<u8>);
		fn get_threshold_signature(req_id: Vec<u8>) -> Vec<u8>;
	}
}
