// Copyright 2017-2019 Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

//! Badger Consensus module for runtime.
//!
//! This manages the Badger authority set ready for the native code, or does it? We'll see.
//! These authorities are only for GRANDPA finality, not for consensus overall.
//!
//! In the future, it will also handle misbehavior reports, and on-chain
//! finality notifications.
//!
//! For full integration with GRANDPA, the `GrandpaApi` should be implemented.
//! The necessary items are re-exported via the `fg_primitives` crate.

#![cfg_attr(not(feature = "std"), no_std)]

// re-export since this is necessary for `impl_apis` in runtime.
//pub use substrate_badger_primitives as fg_primitives;
use badger_primitives::AuthorityId;
use badger_primitives::HBBFT_AUTHORITIES_KEY;
use codec::{self as codec, Decode, Encode, Error,Codec};
use rstd::prelude::*;
use sr_primitives::{
	generic::{DigestItem, OpaqueDigestItemId},
	traits::Zero,
	Perbill,
};
use srml_support::{
	decl_event, decl_module, decl_storage, dispatch::Result, storage::StorageMap,
	storage::StorageValue, storage
};

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

use sr_primitives::ConsensusEngineId;

pub const HBBFT_ENGINE_ID: ConsensusEngineId = *b"BDGR";

//use fg_primitives::HBBFT_ENGINE_ID;
//pub use fg_primitives::{AuthorityId, ConsensusLog};
use system::{ensure_signed, DigestOf};

//#[derive(Decode, Encode, PartialEq, Eq, Clone,Hash)]
//pub type AuthorityId = ([u8; 32],[u8; 16]);

/// An consensus log item for BADGER.
#[cfg_attr(feature = "std", derive(Serialize, Debug))]
#[derive(Decode, Encode, PartialEq, Eq, Clone)]
pub enum ConsensusLog {
	/// Schedule an authority set add by voting... If others also vote
	#[codec(index = "1")]
	VoteToAdd(AuthorityId),
	/// Schedule an authority set removal by voting... If others also vote
	#[codec(index = "2")]
	VoteToRemove(AuthorityId),
    ///completed voting ... may not need these if we use session?
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

mod mock;

pub trait Trait: system::Trait {
	/// The event type of this module.
	type Event: From<Event> + Into<<Self as system::Trait>::Event>;
}

decl_event!(
	// #[derive(Debug)]
	pub enum Event {
		/// New authority set has been applied.
		NewAuthorities(Vec<AuthorityId>),
	}
);

decl_storage! {
	trait Store for Module<T: Trait> as BadgerFinality {
		/// The current authority set.
		Authorities get(authorities): Vec<AuthorityId>;

		/// The number of changes (both in terms of keys and underlying economic responsibilities)
		/// in the "set" of Grandpa validators from genesis.
		CurrentSetId get(current_set_id) build(|_| 0): u64;
	}
	add_extra_genesis {
		config(authorities): Vec<AuthorityId>;
		build(|config| Module::<T>::initialize_authorities(&config.authorities))
	}
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn deposit_event() = default;

		fn on_finalize(block_number: T::BlockNumber) {


		}

	//	Self::deposit_log(ConsensusLog::Resume(delay));
	fn send_log(origin) ->Result
	{
	let who =	ensure_signed(origin)?;
	Self::deposit_log(ConsensusLog::VoteToAdd((AuthorityId::default())));
	Ok(())
	}
	}
}

impl<T: Trait> sr_primitives::BoundToRuntimeAppPublic for Module<T> {
	type Public = AuthorityId;
}

impl<T: Trait> Module<T>
{

  pub fn badger_authorities() -> Vec<AuthorityId> {
		storage::unhashed::get_or_default::<Vec<AuthorityId>>(HBBFT_AUTHORITIES_KEY).into()
	}

	/// Set the current set of authorities, along with their respective weights.
	fn set_badger_authorities(authorities: &Vec<AuthorityId>) {
		storage::unhashed::put(
			HBBFT_AUTHORITIES_KEY,
			authorities,
		);
	}

  
  fn initialize_authorities(authorities: &Vec<AuthorityId>) {
		if !authorities.is_empty() {
			assert!(
				Self::badger_authorities().is_empty(),
				"Authorities are already initialized!"
			);
			Self::set_badger_authorities(authorities);
		}
  }
  

  /// vote to add authority
  pub fn vote_for(auth_id: AuthorityId)
  {
    Self::deposit_log(ConsensusLog::VoteToAdd(auth_id));
  }

  /// vote to remove authority
  fn vote_against(auth_id: AuthorityId)
  {
    Self::deposit_log(ConsensusLog::VoteToRemove(auth_id));
  }


  /// vote to completely change authority set
  fn vote_complete_change(auth_ids: Vec<AuthorityId>)
  {
    Self::deposit_log(ConsensusLog::VoteComplete(auth_ids));
  }
  
  /// Deposit one of this module's logs.
  fn deposit_log(log: ConsensusLog)
  {
    let log: DigestItem<T::Hash> = DigestItem::Consensus(HBBFT_ENGINE_ID, log.encode());
    <system::Module<T>>::deposit_log(log.into());
  }


}



impl<T: Trait> session::OneSessionHandler<T::AccountId> for Module<T>
	where T: session::Trait
{
	type Key = AuthorityId;

	fn on_genesis_session<'a, I: 'a>(validators: I)
		where I: Iterator<Item=(&'a T::AccountId, AuthorityId)>
	{
		let authorities = validators.map(|(_, k)| k).collect::<Vec<_>>();
		Self::initialize_authorities(&authorities);
	}

	fn on_new_session<'a, I: 'a>(changed: bool, validators: I, _queued_validators: I)
		where I: Iterator<Item=(&'a T::AccountId, AuthorityId)>
	{
		// Always issue a change if `session` says that the validators have changed.
		// Even if their session keys are the same as before, the underyling economic
		// identities have changed.
		let current_set_id = if changed {
      let next_authorities = validators.map(|(_, k)| k).collect::<Vec<_>>();
      Self::deposit_log(ConsensusLog::NotifyChangedSet(next_authorities));
			CurrentSetId::mutate(|s| { *s += 1; *s })
		} else {
			// nothing's changed, neither economic conditions nor session keys. update the pointer
			// of the current set.
			Self::current_set_id()
		};

		// if we didn't issue a change, we update the mapping to note that the current
		// set corresponds to the latest equivalent session (i.e. now).
		let session_index = <session::Module<T>>::current_index();
		//SetIdSession::insert(current_set_id, &session_index);
	}

	fn on_disabled(_i: usize) {
    //hbbft cannot be disabled
		//Self::deposit_log(ConsensusLog::OnDisabled(i as u64))
	}
}


