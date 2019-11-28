#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

use sr_primitives::{ConsensusEngineId, };

pub mod app {
	use sr_primitives::app_crypto::{app_crypto, hbbft_thresh, key_types::HB_NODE};
	app_crypto!(hbbft_thresh, HB_NODE);

	#[cfg(feature = "std")]
	use threshold_crypto;
	#[cfg(feature = "std")]
	impl From<Public> for threshold_crypto::PublicKey
	{
		fn from(x:Public) -> threshold_crypto::PublicKey
		{
		 let s=std::convert::Into::<hbbft_thresh::Public>::into(x);
		 s.into()
		}
	}
}

use rstd::vec::Vec;
#[cfg(feature = "std")]
pub type AuthorityPair = app::Pair;

#[cfg(feature = "std")]
use serde::{Serialize};


pub type AuthorityId = app::Public;

pub type AuthoritySignature = app::Signature;

pub const HBBFT_ENGINE_ID: ConsensusEngineId = *b"BDGR";

pub const HBBFT_AUTHORITIES_KEY: &'static [u8] = b":honey_badger_authorities";

pub const HBBFT_AUTHORITIES_MAP_KEY: &'static [u8] = b":honey_badger_auth_map";



pub type AuthorityList = Vec<AuthorityId>;

pub type SetId = u32;
use codec::{self as codec, Decode, Encode, Error,Codec};





use client::decl_runtime_apis;

#[cfg_attr(feature = "std", derive(Serialize))]
#[derive(Decode, Encode, PartialEq, Eq, Clone, RuntimeDebug)]
pub enum ConsensusLog {
	/// Schedule an authority set add by voting... If others also vote
	#[codec(index = "1")]
	VoteToAdd(AuthorityId),
	/// Schedule an authority set removal by voting... If others also vote
	#[codec(index = "2")]
	VoteToRemove(AuthorityId),
    ///completed voting ... m+ay not need these if we use session?
	#[codec(index = "3")]
  VoteComplete(Vec<AuthorityId>),
  #[codec(index = "4")]
  NotifyChangedSet(Vec<AuthorityId>),
  
}

impl ConsensusLog {
	/// Try to cast the log entry as a contained signal.
	pub fn try_into_add(self) -> Option<AuthorityId> {
		match self {
			ConsensusLog::VoteToAdd(id) => Some(id),
			_ => None,
		}
	}

	/// Try to cast the log entry as a contained forced signal.
	pub fn try_into_remove(self) -> Option<AuthorityId> {
		match self {
			ConsensusLog::VoteToRemove(id) => Some(id),
			_ => None,
		}
	}

	/// Try to cast the log entry as a contained pause signal.
	pub fn try_into_complete(self) -> Option<Vec<AuthorityId>> {
		match self {
			ConsensusLog::VoteComplete(authids) => Some(authids),
			_ => None,
		}
	}
}

pub use sr_primitives::{ RuntimeDebug};

#[derive(Decode, Encode, PartialEq, Eq, Clone,RuntimeDebug)]
pub struct AccountBinding<AccountId>
where AccountId:Encode+Decode+core::fmt::Debug,
{
  pub self_pub_key:AuthorityId,
  pub bound_account:AccountId,
}

#[derive(Decode, Encode, PartialEq, Eq, Clone,RuntimeDebug)]
pub struct SignedAccountBinding<AccountId>
where AccountId:Encode+Decode+core::fmt::Debug
{
	pub data:AccountBinding<AccountId>,
	pub sig:AuthoritySignature,
}


decl_runtime_apis! {
	#[api_version(2)]
	pub trait BadgerApi {
		/// Get the current GRANDPA authorities and weights. This should not change except
		/// for when changes are scheduled and the corresponding delay has passed.
		///
		/// When called at block B, it will return the set of authorities that should be
		/// used to finalize descendants of this block (B+1, B+2, ...). The block B itself
		/// is finalized by the authorities from block B-1.
		fn badger_authorities() -> AuthorityList;
	}
}