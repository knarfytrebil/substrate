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
use badger_primitives::{AuthorityId, SignedAccountBinding,BadgerPreRuntime};
use badger_primitives::{HBBFT_AUTHORITIES_KEY, HBBFT_AUTHORITIES_MAP_KEY};
use codec::{self as codec,  Encode,Decode }; //Decode,Error,Codec,
use rstd::collections::btree_map::BTreeMap;
use rstd::prelude::*;
use session::OnSessionEnding;
use sr_primitives::{
  generic::{DigestItem, },
  //traits::Zero,
 // Perbill,
};//OpaqueDigestItemId
use srml_support::{
  decl_event, decl_module, decl_storage, dispatch::Result, storage,  storage::StorageValue,
};
//storage::StorageMap,

//#[cfg(feature = "std")]
//use serde::{Deserialize, Serialize};

use app_crypto::RuntimeAppPublic;
//use sr_primitives::ConsensusEngineId;

//pub const HBBFT_ENGINE_ID: ConsensusEngineId = *b"BDGR";

use badger_primitives::{ConsensusLog, HBBFT_ENGINE_ID};
//pub use fg_primitives::{AuthorityId, ConsensusLog};
use system::ensure_signed; //DigestOf

//#[derive(Decode, Encode, PartialEq, Eq, Clone,Hash)]
//pub type AuthorityId = ([u8; 32],[u8; 16]);

/// An consensus log item for BADGER.
mod mock;

pub trait Trait: system::Trait + session::Trait
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

const HB_DEDUP_KEY_PREFIX: &[u8] = b":hbbft:votes";


decl_storage! {
  trait Store for Module<T: Trait> as BadgerFinality {

	   /// votes of a given Authority (in account form) 
		///
		/// The first key is always `HB_DEDUP_KEY_PREFIX` to have all the data in the same branch of
		/// the trie. Having all data in the same branch should prevent slowing down other queries.
		Votes: double_map hasher(twox_64_concat) Vec<u8>, blake2_256(T::AccountId) =>  Vec<T::AccountId>;

    /// The current authority set.
    //Authorities get(authorities): Vec<AuthorityId>;
	 /// The number of changes (both in terms of keys and underlying economic responsibilities)
    /// in the "set" of Badger validators from genesis.
    CurrentSetId get(current_set_id) build(|_| 0): u64;
  }
  add_extra_genesis {
    config(authorities): Vec<AuthorityId>;
    build(|config| Module::<T>::initialize_authorities(&config.authorities,&BTreeMap::new()))
  }
}
use runtime_io::misc::print_utf8;
decl_module! {
  pub struct Module<T: Trait> for enum Call where origin: T::Origin {
    fn deposit_event() = default;

    fn on_finalize(_block_number: T::BlockNumber) {


	}
	
	///Remove account binding for submitting account
	fn remove_account_binding(origin) -> Result
	{
		let who=ensure_signed(origin)?;
		let mut auth_map:BTreeMap<T::AccountId,AuthorityId>=storage::unhashed::get_or_default::<BTreeMap<T::AccountId,AuthorityId>>(HBBFT_AUTHORITIES_MAP_KEY).into();
		match auth_map.remove(&who)	
		{
			Some(_) => {
				storage::unhashed::put(
					HBBFT_AUTHORITIES_MAP_KEY,
					&auth_map,
				  );				
				Ok(()) 
				 },
				 None => {Err("Account not bound".into())}

		}
	}
    fn submit_account_binding(origin,binding: SignedAccountBinding<T::AccountId>) -> Result
    {
     let who=ensure_signed(origin)?;
     let signature_valid = binding.data.using_encoded(|encoded_data| {
    binding.data.self_pub_key.verify(&encoded_data, &binding.sig)
     });
     if !signature_valid {return Err("Invalid signature on binder".into());}
     if who != binding.data.bound_account
     {
    //print_utf8(b"ERROR: Invalid account trying to use binder");
    return Err("Invalid account trying to use binder".into());
     }
     if Self::check_either_present(&who,&binding.data.self_pub_key)
     {
     // print_utf8(b"ERROR: Account or node already bound");
      return Err("Account or node already bound".into());
     }
	 let mut auth_map:BTreeMap<T::AccountId,AuthorityId>=storage::unhashed::take_or_default::<BTreeMap<T::AccountId,AuthorityId>>(HBBFT_AUTHORITIES_MAP_KEY).into();
	 #[cfg(feature = "std")]
	 print!("EXECUTED vote to bind {:?} ",&binding.data.bound_account,);
	 auth_map.insert(binding.data.bound_account,binding.data.self_pub_key);
     storage::unhashed::put(
      HBBFT_AUTHORITIES_MAP_KEY,
      &auth_map,
    );
    print_utf8(b"Success binding");
     //binding.data.self_pub_key.ver
     Ok(())
	}
	

  //	Self::deposit_log(ConsensusLog::Resume(delay));
  fn send_log(_origin) ->Result
  {
    print_utf8(b"RUNEXTR");
  //let who =	ensure_signed(origin)?;
  //session::Module<Runtime>;
  //<session::Module<T>>::queued_keys();
  Self::deposit_log(ConsensusLog::VoteChangeSet(AuthorityId::default(),vec![ AuthorityId::default()]));
  Ok(())
  }
  

    /// vote to add authority
	pub fn vote_to_add(origin, new_auth_id: T::AccountId)->Result
	{
		let who=ensure_signed(origin)?;
		#[cfg(feature = "std")]
		print!("EXECUTE vote to add{:?}",new_auth_id);
		Self::update_vote_set(&who,vec![new_auth_id],|c_auths,new_ids|
			{
				if new_ids.len()>1
				{
					///panic!("Invalid ids provided");
					#[cfg(feature = "std")]
					print!("ERROR: Invalid ids provided");
					return Err("Invalid ids provided");
				}
				let new_auth=new_ids[0];
				if c_auths.iter().find(|x| *x==new_auth).is_some()
				{
					#[cfg(feature = "std")]
					print!("Account voted for already validator");
					return Err("Account voted for already validator".into()); 
				}
				c_auths.push(new_auth.clone());
				Ok(())
			}
		)	  
	}

	    /// vote to remove authority
		pub fn vote_to_remove(origin, new_auth_id: T::AccountId)->Result
		{
			let who=ensure_signed(origin)?;
			Self::update_vote_set(&who,vec![new_auth_id],|c_auths,new_ids|
				{
					if new_ids.len()>1
					{
						return Err("Invalid ids provided");
					}
					let new_auth=new_ids[0];
					let pos = match c_auths.iter().position(|x| *x == *new_auth)
					{
						Some(ps) =>ps,
						None => { return Err("Account voted against already not in validator set".into()) } 
					};
					if c_auths.len()<=4
					{
						return Err("Cannot reduce validator set further".into());
					}
					c_auths.remove(pos);	
					Ok(())
				}
			)	  
		}


	    /// vote to replace authority set
		pub fn vote_to_change(origin, new_auth_ids: Vec<T::AccountId>)->Result
		{
			let who=ensure_signed(origin)?;
			Self::update_vote_set(&who,new_auth_ids,|c_auths,new_ids|
				{
					if new_ids.len()<4
					{
						return Err("Not enough validators in new set".into()); 
					}
					c_auths.clear();
					for id in new_ids.into_iter()
					{
                     c_auths.push(id.clone());
					}
					Ok(())
				}
			)	  
		}

  //fn vote_for_auth
  //account_to_authority
}
}

impl<T: Trait> sr_primitives::BoundToRuntimeAppPublic for Module<T>
{
  type Public = AuthorityId;
}

impl<T: Trait> Module<T>
{

	pub fn update_vote_set(who:&T::AccountId,new_auth_ids: Vec<T::AccountId>,mut how: impl FnMut(&mut Vec<AuthorityId>, Vec<&AuthorityId>)->Result )->Result
	{
		if let Some(our_authid)= Self::is_account_authority(&who)
		{
		   let     auth_map:BTreeMap<T::AccountId,AuthorityId>=storage::unhashed::get_or_default::<BTreeMap<T::AccountId,AuthorityId>>(HBBFT_AUTHORITIES_MAP_KEY).into();
		   let mut new_auths:Vec<&AuthorityId>=Vec::new();
		   for id in new_auth_ids.iter()
		   {
			if let Some(new_auth)=auth_map.get(id)
			{
                new_auths.push(new_auth);
			}   
			else
			{
				#[cfg(feature = "std")]
				print!("One of the accounts does not have bound authority {:?}",id);
				#[cfg(feature = "std")]
			    print!("No authority! {:?}",&auth_map);
				return Err("One of the accounts does not have bound authority".into());
			}
		   }
			   let mut c_auths=Self::badger_authorities();
			   how(&mut c_auths,new_auths)?;
			   let mut cmap:Vec<T::AccountId>=Vec::new();
			   for auth in c_auths.iter()
			   {
				   let acc=match auth_map.iter().find( |(_,au)| *au==auth )
				   {
					   Some(dat) =>dat.0.clone(),
   
					   None => return Err("Internal error: not all validators mapped".into()),
				   };
					 cmap.push(acc);
			   }
			   <Votes<T>>::insert(HB_DEDUP_KEY_PREFIX, who,cmap);
			   Self::deposit_log(ConsensusLog::VoteChangeSet(our_authid.clone(),c_auths));
              Ok(())
		   }
		   else
		   {
			#[cfg(feature = "std")]
			print!("Account not validator");
			return Err("Account not validator".into());   
		   }
	}
   pub fn is_account_authority(acc: &T::AccountId) ->Option<AuthorityId>
   {
	let  auth_map:BTreeMap<T::AccountId,AuthorityId>=storage::unhashed::get_or_default::<BTreeMap<T::AccountId,AuthorityId>>(HBBFT_AUTHORITIES_MAP_KEY).into();
	if let Some(authid)=auth_map.get(acc)
	{
		let auth_list: Vec<AuthorityId> = storage::unhashed::get_or_default::<Vec<AuthorityId>>(HBBFT_AUTHORITIES_KEY).into();
		match auth_list.iter().find(|x| *x==authid)
		{
			Some(k) => Some(k.clone()),
			None=>None
		}
	}
	else
	{
	None
	}
   }	
  pub fn badger_authorities() -> Vec<AuthorityId>
  {
    storage::unhashed::get_or_default::<Vec<AuthorityId>>(HBBFT_AUTHORITIES_KEY).into()
  }
  pub fn account_to_authority(acc: &T::AccountId) -> Option<AuthorityId>
  {
    let auth_map: BTreeMap<T::AccountId, AuthorityId> =
      storage::unhashed::get_or_default::<BTreeMap<T::AccountId, AuthorityId>>(HBBFT_AUTHORITIES_MAP_KEY).into();
    match auth_map.get(acc)
    {
      Some(au) => Some(au.clone()),
      None => None,
    }
  }
  pub fn check_either_present(acc: &T::AccountId, auth: &AuthorityId) -> bool
  {
    let auth_map: BTreeMap<T::AccountId, AuthorityId> =
      storage::unhashed::get_or_default::<BTreeMap<T::AccountId, AuthorityId>>(HBBFT_AUTHORITIES_MAP_KEY).into();
    match auth_map.get(acc)
    {
      Some(_au) => return true,
      None =>
      {}
    };
    auth_map.iter().find(|(_, v)| **v == *auth).is_some()
  }

  /// Set the current set of authorities, along with their respective weights.
  fn set_badger_authorities(authorities: &Vec<AuthorityId>, auth_map: &BTreeMap<T::AccountId, AuthorityId>)
  {
    storage::unhashed::put(HBBFT_AUTHORITIES_KEY, authorities);
    storage::unhashed::put(HBBFT_AUTHORITIES_MAP_KEY, auth_map);
    //	pub const HBBFT_AUTHORITIES_MAP_KEY: &'static [u8] = b":honey_badger_auth_map";
  }

  fn initialize_authorities(authorities: &Vec<AuthorityId>, auth_map: &BTreeMap<T::AccountId, AuthorityId>)
  {
    if !authorities.is_empty()
    {
      assert!(
        Self::badger_authorities().is_empty(),
        "Authorities are already initialized!"
      );
      Self::set_badger_authorities(authorities, auth_map);
    }
  }



  /// vote to completely change authority set

  /// Deposit one of this module's logs.
  fn deposit_log(log: ConsensusLog)
  {
    let log: DigestItem<T::Hash> = DigestItem::Consensus(HBBFT_ENGINE_ID, log.encode());
    <system::Module<T>>::deposit_log(log.into());
  }
}

impl<T: Trait> session::ShouldEndSession<T::BlockNumber> for Module<T>
{
  fn should_end_session(_now: T::BlockNumber) -> bool
  {
	  //just check if session shift pre_digest was provided
	  	let maybe_pre_digest:Vec<_> = <system::Module<T>>::digest()
			.logs
			.iter()
			.filter_map(|s| s.as_pre_runtime())
			.filter_map(|(id, mut data)| if id == HBBFT_ENGINE_ID {
				BadgerPreRuntime::decode(&mut data).ok()
			} else {
				None
			})
			.collect();
			for v in maybe_pre_digest.into_iter()
			{
				match v
				{
					BadgerPreRuntime::ValidatorsChanged(_) => {return true;}
				}
			}

	//Self::do_initialize(now);
	//storage::unhashed::take(HBBFT_AUTHORITIES_KEY, authorities);
    return false; //for now. TODO
                  //Self::should_epoch_change(now)
  }
}

type SessionIndex = u32;

impl<T: Trait> OnSessionEnding<T::AccountId> for Module<T>
{

	/// We ignore when session *should* start since when the pre_runtime is issued session *is* over in the consensus engine
  fn on_session_ending(_ending: SessionIndex, _start_session: SessionIndex) -> Option<Vec<T::AccountId>>
  {
	let auth_map: BTreeMap<T::AccountId, AuthorityId> =
	storage::unhashed::get_or_default::<BTreeMap<T::AccountId, AuthorityId>>(HBBFT_AUTHORITIES_MAP_KEY).into();

	let maybe_pre_digest:Vec<_> = <system::Module<T>>::digest()
	.logs
	.iter()
	.filter_map(|s| s.as_pre_runtime())
	.filter_map(|(id, mut data)| if id == HBBFT_ENGINE_ID {
		BadgerPreRuntime::decode(&mut data).ok()
	} else {
		None
	})
	.collect();
	for v in maybe_pre_digest.into_iter()
	{
		match v
		{
			BadgerPreRuntime::ValidatorsChanged(valids) => 
			     {
					 let mut ret:Vec<T::AccountId>=Vec::new();
				 for authid in valids.into_iter()
				 {
				   let mp=auth_map.iter().find(|(_,value)| **value==authid);
				   if let Some(accid)=mp
				   {
                  ret.push(accid.0.clone());
				   }
				   else
                  {
                    //this is ERROR XD
                    } 

				 }
				 return Some(ret);
				}
		}
	}

    None
  }
}

impl<T: Trait> session::OneSessionHandler<T::AccountId> for Module<T>
where
  T: session::Trait,
{
  type Key = AuthorityId;

  fn on_genesis_session<'a, I: 'a>(validators: I)
  where
    I: Iterator<Item = (&'a T::AccountId, AuthorityId)>,
  {
    let auth_map = validators.map(|(k, v)| (k.clone(), v)).collect::<BTreeMap<_, _>>();
    let authorities = auth_map.iter().map(|(_, k)| k.clone()).collect::<Vec<_>>();
    Self::initialize_authorities(&authorities, &auth_map);
  }

  fn on_new_session<'a, I: 'a>(changed: bool, validators: I, _queued_validators: I)
  where
    I: Iterator<Item = (&'a T::AccountId, AuthorityId)>,
  {
    // Always issue a change if `session` says that the validators have changed.
    // Even if their session keys are the same as before, the underyling economic
    // identities have changed.
    let _current_set_id = if changed
    {
	  let next_authorities = validators.map(|(_, k)| k).collect::<Vec<_>>();
	  storage::unhashed::put(HBBFT_AUTHORITIES_KEY, &next_authorities);//update
      Self::deposit_log(ConsensusLog::NotifyChangedSet(next_authorities));
      CurrentSetId::mutate(|s| {
        *s += 1;
        *s
      })
    }
    else
    {
      // nothing's changed, neither economic conditions nor session keys. update the pointer
      // of the current set.
      Self::current_set_id()
    };

    // if we didn't issue a change, we update the mapping to note that the current
    // set corresponds to the latest equivalent session (i.e. now).
    //let session_index = <session::Module<T>>::current_index();
    //SetIdSession::insert(current_set_id, &session_index);
  }

  fn on_disabled(_i: usize)
  {
    //hbbft cannot be disabled
    //Self::deposit_log(ConsensusLog::OnDisabled(i as u64))
  }
}
