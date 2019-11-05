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

#![cfg_attr(not(feature = "std"), no_std)]

// re-export since this is necessary for `impl_apis` in runtime.
//pub use substrate_badger_primitives as fg_primitives;
use app_crypto::RuntimeAppPublic;
use codec::{self as codec,  Decode, Encode}; //Codec,, Error
use primitives::offchain::StorageKind;
use rstd::prelude::*;
use sr_primitives::traits::Member;
use sr_primitives::traits::Printable;
use sr_primitives::{RuntimeDebug,
  generic::{DigestItem, OpaqueDigestItemId},
  	transaction_validity::{
		TransactionValidity, TransactionLongevity, ValidTransaction, InvalidTransaction,
	},
 // traits::Zero,
 // Perbill,
};
use srml_support::{
  decl_event, decl_module, decl_storage, dispatch::Result as dresult, ensure, print,
   storage::StorageValue, Parameter, //storage::StorageMap,
};
use substrate_mpecdsa_primitives::{
  get_complete_list_prefix, get_key_prefix, ConsensusLog, RequestId, //get_data_prefix, 
  MAIN_DB_PREFIX, MP_ECDSA_ENGINE_ID,
};

use system::ensure_none;
use system::offchain::SubmitUnsignedTransaction;

//#[cfg(feature = "std")]
//use serde::{Deserialize, Serialize};

//use sr_primitives::ConsensusEngineId;

//use fg_primitives::HBBFT_ENGINE_ID;
//pub use fg_primitives::{AuthorityId, ConsensusLog};
use system::{ensure_signed, DigestOf};

pub mod sr25519
{
  mod app_sr25519
  {
    use app_crypto::{app_crypto, key_types::IM_ONLINE, sr25519};
    app_crypto!(sr25519, IM_ONLINE);

    impl From<Signature> for sr_primitives::AnySignature
    {
      fn from(sig: Signature) -> Self
      {
        sr25519::Signature::from(sig).into()
      }
    }
  }

  /// An i'm online keypair using sr25519 as its crypto.
  #[cfg(feature = "std")]
  pub type AuthorityPair = app_sr25519::Pair;

  /// An i'm online signature using sr25519 as its crypto.
  pub type AuthoritySignature = app_sr25519::Signature;

  /// An i'm online identifier using sr25519 as its crypto.
  pub type AuthorityId = app_sr25519::Public;
}

//#[derive(Decode, Encode, PartialEq, Eq, Clone,Hash)]
pub type AuthorityId = ([u8; 32], [u8; 16]);

mod mock;

pub trait Trait: system::Trait
{
  /// The event type of this module.
  type Event: From<Event> + Into<<Self as system::Trait>::Event>;
  type AuthorityId: Member + Parameter + RuntimeAppPublic + Default + Ord;
  /// A dispatchable call type.
  type Call: From<Call<Self>>;
  type SubmitTransaction: SubmitUnsignedTransaction<Self, <Self as Trait>::Call>;
}

decl_event!(
  // #[derive(Debug)]
  pub enum Event
  {
    /// New authority set has been applied.
    NewResult(RequestId),
  }
);

decl_storage! {
  trait Store for Module<T: Trait> as Keygen {
    /// The current active requests
    pub Keys get(keys): Vec<T::AuthorityId>;
    pub Test get(test): u64;
   // pub ReqIds get(requests): Vec<u64>;
    pub RequestResults: map u64 => Option<Option<[u8;32]>>;


    //RequestResultVotes: maps authority/requestid to request vote
    pub RequestResultVotes: double_map T::AuthorityId, blake2_256(u64) => Option<Option<Vec<u8>>>;
     }
  add_extra_genesis {
  //  config(requests): Vec<u64>;
  //  build(|config| Module::<T>::initialize_authorities(&config.authorities))
  }
}
pub type AuthIndex = u32;

/// Error which may occur while executing the off-chain code.
#[cfg_attr(feature = "std", derive(Debug))]
pub enum OffchainErr
{
  DecodeWorkerStatus,
  FailedSigning,
  NetworkState,
  SubmitTransaction,
}

impl Printable for OffchainErr
{
  fn print(&self)
  {
    match self
    {
      OffchainErr::DecodeWorkerStatus => print("Offchain error: decoding WorkerStatus failed!"),
      OffchainErr::FailedSigning => print("Offchain error: signing failed!"),
      OffchainErr::NetworkState => print("Offchain error: fetching network state failed!"),
      OffchainErr::SubmitTransaction => print("Offchain error: submitting transaction failed!"),
    }
  }
}

#[derive(Encode, Decode, Clone, PartialEq, Eq,RuntimeDebug)]
pub struct KeygenResult
{
  pub req_id: u64,
  pub result_data: Option<Vec<u8>>,
  pub our_auth_index: AuthIndex,
}


decl_module! {
  pub struct Module<T: Trait> for enum Call where origin: T::Origin {
    fn deposit_event() = default;

    fn set_test(origin)
    {
       print("Offchain ONCHAIN");
      <Self as Store>::Test::put(10);
      
    }
    fn report_result(
      origin,
      result:KeygenResult,
      signature: <T::AuthorityId as RuntimeAppPublic>::Signature
    ) {
      ensure_none(origin)?;
      let keys = Keys::<T>::get();
      let public = keys.get(result.our_auth_index as usize);
      if public.is_none()
      {
        Err("Non existent public key.")?
      }

      let public_un = public.unwrap();

      let exists = <RequestResultVotes<T>>::exists(
        &public_un,
        &result.req_id,
      );
      ensure!(!exists,"This result was already set");


        let signature_valid = result.using_encoded(|encoded_result| {
          public_un.verify(&encoded_result, &signature)
        });
        ensure!(signature_valid, "Invalid result signature.");
        <RequestResultVotes<T>>::insert(&public_un,&result.req_id,result.result_data);
    }


    fn on_finalize(_block_number: T::BlockNumber) {

          //TODO: check if I need to add anything.

        }

    //	Self::deposit_log(ConsensusLog::Resume(delay));
  fn send_log(origin,req_id:u64) ->dresult
  {
  let _who =	ensure_signed(origin)?;
  let mut a:Vec<u8>=MAIN_DB_PREFIX.to_vec();
  let mut b=req_id.encode(); 
  a.append(&mut b);
    let kind = primitives::offchain::StorageKind::PERSISTENT;
  runtime_io::local_storage_set(kind,&a,b"new");
    if <Self as Store>::RequestResults::exists(req_id)
    {
      return Err("Duplicate request ID");
    }
    let a:Option<[u8;32]>=None;
    <Self as Store>::RequestResults::insert(req_id,&a);
    Self::deposit_log(ConsensusLog::RequestForKeygen((req_id, [2;32].to_vec())));
  Ok(())
  }



        // Runs after every block.
    fn offchain_worker(_now: T::BlockNumber) {
       print("Offchain OFFCHAIN");
          let call = Call::set_test();
      T::SubmitTransaction::submit_unsigned(call).map_err(|_| OffchainErr::SubmitTransaction);
      // Only send messages if we are a potential validator.
      if runtime_io::is_validator() {
//        let mut requests = <ReqIds>::get();
        Self::offchain();
      }
    }

}
}

impl<T: Trait> Module<T>
{
  /// Deposit one of this module's logs.
  fn deposit_log(log: ConsensusLog)
  {
    let log: DigestItem<T::Hash> = DigestItem::Consensus(MP_ECDSA_ENGINE_ID, log.encode());
    <system::Module<T>>::deposit_log(log.into());
  }
  pub fn do_post_result(id: u64, r: Vec<u8>) -> Result<(), OffchainErr>
  {
    let authorities = Keys::<T>::get();
    let mut local_keys = T::AuthorityId::all();
    local_keys.sort();
    let data = match r.len()
    {
      1 => None,
      _ => Some(r),
    };

    for (authority_index, key) in
      authorities
        .into_iter()
        .enumerate()
        .filter_map(|(index, authority)| {
          local_keys
            .binary_search(&authority)
            .ok()
            .map(|location| (index as u32, &local_keys[location]))
        })
    {
      let res = KeygenResult {
        req_id: id,
        result_data: data.clone(),
        our_auth_index: authority_index,
      };
      let signature = key.sign(&res.encode()).ok_or(OffchainErr::FailedSigning)?;
      let call = Call::report_result(res, signature);
      T::SubmitTransaction::submit_unsigned(call).map_err(|_| OffchainErr::SubmitTransaction)?;
    }
    Ok(())
  }

  pub fn offchain()
  {
    let cl_key = get_complete_list_prefix();
    let result = runtime_io::local_storage_get(StorageKind::PERSISTENT, &cl_key);
    if let Some(dat) = result
    {
      let requests: Vec<RequestId> = match Decode::decode(&mut dat.as_ref())
      {
        Ok(res) => res,
        Err(_) => return,
      };
      if requests.len() == 0
      {
        return;
      }

      let _:Vec<RequestId>=requests.iter().filter(|req_id| {
        let key: Vec<u8> = get_key_prefix(**req_id);
        let result = runtime_io::local_storage_get(StorageKind::PERSISTENT, &key);
        if let Some(r) = result
        {
          match Self::do_post_result(**req_id, r)
          {
            Ok(_) =>
            {}
            Err(err) => print(err),
          };
          return false;
        }
        return true;
      }).cloned().collect();
    }
  }
  /// Attempt to extract a Keygen log from a generic digest.
  pub fn keygen_log(digest: &DigestOf<T>) -> Option<ConsensusLog>
  {
    let id = OpaqueDigestItemId::Consensus(&MP_ECDSA_ENGINE_ID);
    digest.convert_first(|l| l.try_to::<ConsensusLog>(id))
  }
}


impl<T: Trait> srml_support::unsigned::ValidateUnsigned for Module<T> {
	type Call = Call<T>;

	fn validate_unsigned(call: &Self::Call) -> TransactionValidity {
    print("OFFCHAIN VALIDATE");
		if let Call::set_test() = call {
	
  	return Ok(ValidTransaction {
				priority: 0,
				requires: vec![],
				provides: vec![b"OFFCHAIN".encode()],
				longevity: TransactionLongevity::max_value(),
				propagate: true,
			});
	
		
		
		} else {
			
     if let  Call::report_result(res, signature)= call
     {
       	return Ok(ValidTransaction {
				priority: 0,
				requires: vec![],
				provides: vec![signature.encode()],
				longevity: TransactionLongevity::max_value(),
				propagate: true,
			});
     }


		}
    	InvalidTransaction::Call.into() 
	}
}
