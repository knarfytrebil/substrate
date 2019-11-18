#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

use sr_primitives::{ConsensusEngineId, };

pub mod app {
	use sr_primitives::app_crypto::{app_crypto, hbbft_thresh, key_types::HB_NODE};
	app_crypto!(hbbft_thresh, HB_NODE);
}

use rstd::vec::Vec;
#[cfg(feature = "std")]
pub type AuthorityPair = app::Pair;

pub type AuthorityId = app::Public;

pub type AuthoritySignature = app::Signature;

pub const HBBFT_ENGINE_ID: ConsensusEngineId = *b"BDGR";

pub const HBBFT_AUTHORITIES_KEY: &'static [u8] = b":honey_badger_authorities";

pub type AuthorityList = Vec<AuthorityId>;

pub type SetId = u64;


use client::decl_runtime_apis;



decl_runtime_apis! {
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