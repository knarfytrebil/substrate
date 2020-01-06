#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::type_complexity)]

use codec::{Decode, Encode};

use app_crypto::RuntimeAppPublic;
use sp_core::offchain::StorageKind;
use sp_io::offchain::local_storage_get;
use sp_runtime::{
	generic::DigestItem,
	traits::{IdentifyAccount, Member, One, SimpleArithmetic, StaticLookup, Zero},
	RuntimeDebug,
};
use sp_std::{collections::btree_set::BTreeSet, prelude::*};
use support::{
	debug, decl_event, decl_module, decl_storage, dispatch::DispatchResult, ensure, traits::Time, Parameter,
};
use system::{
	ensure_signed,
	offchain::{CreateTransaction, SubmitSignedTransaction},
};

pub use sp_mpc::{crypto, ConsensusLog, KEY_TYPE, MPC_ENGINE_ID};

pub trait Trait: system::Trait {
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;

	type Call: From<Call<Self>>;

	type SubmitTransaction: SubmitSignedTransaction<Self, <Self as Trait>::Call>;
}

decl_storage! {
	trait Store for Module<T: Trait> as Mpc {
		Authorities get(authorities): BTreeSet<T::AccountId>;

		Results get(fn result_of): map u64 => Vec<u8>;

		Requests get(fn request_of): map u64 => Vec<u8>; // TODO: request timeout

		PendingReqIds: BTreeSet<u64>;
	}
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn deposit_event() = default;

		fn request_key(origin, id: u64) -> DispatchResult {
			let _who = ensure_signed(origin)?;
			ensure!(!<Requests>::exists(id), "req id exists");
			ensure!(!<Results>::exists(id), "req id exists");
			<PendingReqIds>::mutate(|ids| {
				ids.insert(id);
			});
			Self::send_log_for_key(id);
			Ok(())
		}

		fn request_sig(origin, id: u64, data: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			ensure!(!<Requests>::exists(id), "req id exists");
			ensure!(!<Results>::exists(id), "req id exists");

			<PendingReqIds>::mutate(|ids| {
				ids.insert(id);
			});
			<Requests>::insert(id, data.clone());
			Self::send_log_for_sig(id, data.clone());
			Self::deposit_event(RawEvent::MpcRequest(
				id, data, who
			));
			Ok(())
		}

		pub fn save_sig(origin, id: u64, data: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?; // more restriction?
			ensure!(<Requests>::exists(id), "req id does not exist");
			ensure!(!<Results>::exists(id), "req id exists");
			// remove req
			<PendingReqIds>::mutate(|ids| {
				ids.remove(&id);
			});
			<Requests>::remove(id);
			// save res
			<Results>::insert(id, data.clone());
			Self::deposit_event(RawEvent::MpcResponse(
				id, data, who
			));
			debug::warn!("save sig");
			Ok(())
		}

		fn offchain_worker(_now: T::BlockNumber) {
			debug::RuntimeLogger::init();
			let req_ids = PendingReqIds::get();
			for id in req_ids {
				let key = id.to_le_bytes();
				debug::warn!("key {:?}", key);
				if let Some(value) = local_storage_get(StorageKind::PERSISTENT, &key) {
					// StorageKind::LOCAL ?
					Self::submit_result(id, value);
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
		// id, request data, requester
		MpcRequest(u64, Vec<u8>, AccountId),
		// id, result data, responser
		MpcResponse(u64, Vec<u8>, AccountId),
	}
);

impl<T: Trait> Module<T> {
	fn submit_result(id: u64, data: Vec<u8>) {
		let call = Call::save_sig(id, data);
		let res = T::SubmitTransaction::submit_signed(call);

		if res.is_empty() {
			debug::error!("No local accounts found.");
		} else {
			debug::info!("Sent transactions from: {:?}", res);
		}
	}

	fn is_authority(who: &T::AccountId) -> bool {
		Self::authorities().contains(who)
	}

	fn send_log_for_key(id: u64) {
		Self::deposit_log(ConsensusLog::RequestForKey(id));
	}

	fn send_log_for_sig(id: u64, data: Vec<u8>) {
		Self::deposit_log(ConsensusLog::RequestForSig(id, data));
	}

	fn deposit_log(log: ConsensusLog) {
		let log: DigestItem<T::Hash> = DigestItem::Consensus(MPC_ENGINE_ID, log.encode());
		<system::Module<T>>::deposit_log(log.into());
	}
}
