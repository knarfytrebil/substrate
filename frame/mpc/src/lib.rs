#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::type_complexity)]

use codec::{Decode, Encode};

//use app_crypto::RuntimeAppPublic;
use sp_core::offchain::StorageKind;
use sp_io::offchain::local_storage_get;
use sp_runtime::{
	generic::DigestItem,
	//traits::{ Member, One, SimpleArithmetic, StaticLookup, Zero}, //IdentifyAccount 
	RuntimeDebug,
};
use sp_std::{collections::btree_set::BTreeSet, prelude::*};
use support::{
	debug, decl_event, decl_module, decl_storage, dispatch::DispatchResult, ensure, traits::Time, Parameter,
};
use system::{
	ensure_signed,
	offchain::{ SubmitSignedTransaction}, //CreateTransaction
};

pub use sp_mpc::{crypto, get_storage_key, ConsensusLog, MpcRequest, OffchainStorageType, KEY_TYPE, MPC_ENGINE_ID};

#[derive(Encode, Decode)]
pub enum MpcResult {
	KeyGen { req_id: u64, pk: Vec<u8> },
	SigGen { req_id: u64, pk_id: u64, sig: Vec<u8> },
}

pub trait Trait: system::Trait {
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;

	type Call: From<Call<Self>>;

	type SubmitTransaction: SubmitSignedTransaction<Self, <Self as Trait>::Call>;
}

decl_storage! {
	trait Store for Module<T: Trait> as Mpc {
		Authorities get(authorities): BTreeSet<T::AccountId>;

		Results get(fn result_of): map u64 => Option<MpcResult>;

		Requests get(fn request_of): map u64 => Option<MpcRequest>; // TODO: request timeout?

		PendingReqIds: BTreeSet<u64>;

		ActiveKeyIds: BTreeSet<u64>;

		RetiredKeyIds: BTreeSet<u64>;
	}
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn deposit_event() = default;

		fn request_key(origin, req_id: u64) -> DispatchResult {
			let _who = ensure_signed(origin)?;
			ensure!(!<Requests>::exists(req_id), "req id exists");
			ensure!(!<Results>::exists(req_id), "req id exists");

			<PendingReqIds>::mutate(|ids| {
				ids.insert(req_id);
			});
			Self::send_keygen_log(req_id);
			// deposit event
			Ok(())
		}

		pub fn save_key(origin, req_id: u64, data: Vec<u8>) -> DispatchResult {
			let _who = ensure_signed(origin)?;
			ensure!(<Requests>::exists(req_id), "req id does not exist");
			ensure!(!<Results>::exists(req_id), "req id exists");
			Ok(())
		}

		fn request_sig(origin, req_id: u64, pk_id: u64, data: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			ensure!(!<Requests>::exists(req_id), "req id exists");
			ensure!(!<Results>::exists(req_id), "req id exists");

			<PendingReqIds>::mutate(|ids| {
				ids.insert(req_id);
			});
			<Requests>::insert(req_id, MpcRequest::SigGen(req_id, pk_id, data.clone()));
			Self::send_siggen_log(req_id, pk_id, data);
			Self::deposit_event(RawEvent::MpcRequest(
				req_id, who
			));
			Ok(())
		}

		pub fn save_sig(origin, req_id: u64, pk_id: u64, sig: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?; // more restriction?
			ensure!(<Requests>::exists(req_id), "req id does not exist");
			ensure!(!<Results>::exists(req_id), "req id exists");

			// remove req
			<PendingReqIds>::mutate(|ids| {
				ids.remove(&req_id);
			});
			<Requests>::remove(req_id);

			// save res
			<Results>::insert(req_id, MpcResult::SigGen { req_id, pk_id, sig });
			Self::deposit_event(RawEvent::MpcResponse(
				req_id, who
			));
			debug::warn!("save sig");
			Ok(())
		}

		fn offchain_worker(_now: T::BlockNumber) {
			debug::RuntimeLogger::init();
			let req_ids = PendingReqIds::get();
			for id in req_ids {
				let key = get_storage_key(id, OffchainStorageType::Signature);
				debug::warn!("key {:?}", key);
				if let Some(value) = local_storage_get(StorageKind::PERSISTENT, &key) {
					// StorageKind::LOCAL ?
					Self::call_save_sig(id, 0, value);
					debug::warn!("insert ok");
				} else {
					debug::warn!("nothing");
				}
			}
		}

		pub fn add_authority(origin, who: T::AccountId) -> DispatchResult {
			let _me = ensure_signed(origin)?; // ensure root?

			if !Self::is_authority(&who){
				<Authorities<T>>::mutate(|l| l.insert(who));
			}

			Ok(())
		}
	}
}

decl_event!(
	pub enum Event<T>
	where
		AccountId = <T as system::Trait>::AccountId,
	{
		// id, requester
		MpcRequest(u64, AccountId),
		// id, responser
		MpcResponse(u64, AccountId),
	}
);

impl<T: Trait> Module<T> {
	fn submit_signed(call: Call<T>) {
		let res = T::SubmitTransaction::submit_signed(call);

		if res.is_empty() {
			debug::error!("No local accounts found.");
		} else {
			debug::info!("Sent transactions from: {:?}", res);
		}
	}

	fn call_save_sig(req_id: u64, pk_id: u64, sig: Vec<u8>) {
		let call = Call::save_sig(req_id, pk_id, sig);
		Self::submit_signed(call);
	}

	fn call_save_key(req_id: u64, pk: Vec<u8>) {
		let call = Call::save_key(req_id, pk);
		Self::submit_signed(call);
	}

	fn is_authority(who: &T::AccountId) -> bool {
		Self::authorities().contains(who)
	}

	fn send_keygen_log(id: u64) {
		Self::deposit_log(ConsensusLog::RequestForKey(id));
	}

	fn send_siggen_log(req_id: u64, pk_id: u64, data: Vec<u8>) {
		Self::deposit_log(ConsensusLog::RequestForSig(req_id, pk_id, data));
	}

	fn deposit_log(log: ConsensusLog) {
		let log: DigestItem<T::Hash> = DigestItem::Consensus(MPC_ENGINE_ID, log.encode());
		<system::Module<T>>::deposit_log(log.into());
	}
}
