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

use codec::{self as codec, Decode, Encode, Error,Codec};
use rstd::prelude::*;
use sr_primitives::{
  generic::{DigestItem, OpaqueDigestItemId},
  traits::Zero,
  Perbill,
};
use srml_support::{
  decl_event, decl_module, decl_storage, dispatch::Result, storage::StorageMap,
  storage::StorageValue,
};

#[cfg(feature = "std")]
use serde::{Serialize,Deserialize};

use sr_primitives::ConsensusEngineId;

pub const HBBFT_ENGINE_ID: ConsensusEngineId = *b"BDGR";

//use fg_primitives::HBBFT_ENGINE_ID;
//pub use fg_primitives::{AuthorityId, ConsensusLog};
use system::{ensure_signed, DigestOf};

//#[derive(Decode, Encode, PartialEq, Eq, Clone,Hash)]
pub type AuthorityId = ([u8; 32],[u8; 16]);



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
    ///completed voting
	#[codec(index = "3")]
	VoteComplete(Vec<AuthorityId>),
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

pub trait Trait: system::Trait
{
  /// The event type of this module.
  type Event: From<Event> + Into<<Self as system::Trait>::Event>;
}



decl_event!(
// #[derive(Debug)]
  pub enum Event
  {
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

          //TODO: check if I need to add anything.

        }

    //	Self::deposit_log(ConsensusLog::Resume(delay));
	fn send_log(origin) ->Result
	{
	let who =	ensure_signed(origin)?;
    Self::deposit_log(ConsensusLog::VoteToAdd(([0;32],[0;16])));
	Ok(())
	}
		
  }
}

impl<T: Trait> Module<T>
{
  /// Get the current set of authorities, along with their respective weights.
  pub fn badger_authorities() -> Vec<AuthorityId>
  {
    Authorities::get()
  }

  /// vote to add authority
  pub fn vote_for(authId: AuthorityId)
  {
    Self::deposit_log(ConsensusLog::VoteToAdd(authId));
  }

  /// vote to remove authority
  fn vote_against(auth_id: AuthorityId)
  {
    Self::deposit_log(ConsensusLog::VoteToRemove(auth_id));
  }
  
  /// Deposit one of this module's logs.
  fn deposit_log(log: ConsensusLog)
  {
    let log: DigestItem<T::Hash> = DigestItem::Consensus(HBBFT_ENGINE_ID, log.encode());
    <system::Module<T>>::deposit_log(log.into());
  }

  fn initialize_authorities(authorities: &[AuthorityId])
  {
    if !authorities.is_empty()
    {
      assert!(
        Authorities::get().is_empty(),
        "Authorities are already initialized!"
      );
      Authorities::put(authorities);
    }
  }
}

impl<T: Trait> Module<T>
{
  /// Attempt to extract a GRANDPA log from a generic digest.
  pub fn badger_log(digest: &DigestOf<T>) -> Option<ConsensusLog>
  {
    let id = OpaqueDigestItemId::Consensus(&HBBFT_ENGINE_ID);
    digest.convert_first(|l| l.try_to::<ConsensusLog>(id))
  }

  /// Attempt to extract a pending set-change signal from a digest.
  pub fn pending_vote_to_add(digest: &DigestOf<T>) -> Option<AuthorityId>
  {
    Self::badger_log(digest).and_then(|signal| signal.try_into_add())
  }

  /// Attempt to extract a pending set-change signal from a digest.
  pub fn pending_vote_to_remove(digest: &DigestOf<T>) -> Option<AuthorityId>
  {
    Self::badger_log(digest).and_then(|signal| signal.try_into_remove())
  }

  /// Attempt to extract a pending set-change signal from a digest.
  pub fn vote_complete(digest: &DigestOf<T>) -> Option<Vec<AuthorityId>>
  {
    Self::badger_log(digest).and_then(|signal| signal.try_into_complete())
  }
}

