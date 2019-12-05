#![cfg_attr(not(feature = "std"), no_std)]

use rstd::prelude::*;
use codec::{self as codec, Encode, Decode, Error};
use support::{decl_module, decl_storage, decl_event, print};
use system::{ensure_none, ensure_signed, offchain::SubmitUnsignedTransaction};
use sr_primitives::{
  generic::DigestItem,
  transaction_validity::{
    TransactionValidity, TransactionLongevity, ValidTransaction, InvalidTransaction,
  }
};
use ln_primitives::{
  LN_ENGINE_ID, ConsensusLog,
  Account, Tx
};
use primitives::offchain::StorageKind;
use runtime_io::offchain::local_storage_get;

pub trait Trait: system::Trait {
  type Call: From<Call<Self>>;
  type SubmitTransaction: SubmitUnsignedTransaction<Self, <Self as Trait>::Call>;
}

decl_storage! {
  trait Store for Module<T: Trait> as Lightning {
    SomeFlag: u64;
    PubKey: Vec<u8>;
  }
}

// decl_event!(
//   pub enum Event {
//     Light
//   }
// );

decl_module! {
  pub struct Module<T: Trait> for enum Call where origin: T::Origin {
    fn request_deposit(origin, amount: u64) {
    }
    fn request_atomic_swap_deposit(origin, amount: u64) {
      // let account_id = ensure_signed(origin)?;
      // let account = Account { id: 1, wallet_id: 1 };
      // let tx = Tx { amount };
      // let log = ConsensusLog::DepositReq(account, tx);
      // let log = ConsensusLog::ConnectPeer();
      // let log: DigestItem<T::Hash> = DigestItem::Consensus(LN_ENGINE_ID, log.encode());
      // <system::Module<T>>::deposit_log(log.into());
    }
    fn request_atomic_swap_deposit_with_x(origin, amount: u64) {
    }
    fn connect_peer(origin, node_key: Vec<u8>) {
      // let account_id = ensure_signed(origin)?;
      let log = ConsensusLog::ConnectPeer(node_key);
      let log: DigestItem<T::Hash> = DigestItem::Consensus(LN_ENGINE_ID, log.encode());
      <system::Module<T>>::deposit_log(log.into());
    }

    fn sync_pub_key(origin, key: Vec<u8>) {
      PubKey::put(key);
    }

    fn on_finalize(block_number: T::BlockNumber) {
    }

    fn offchain_worker(now: T::BlockNumber) {
      let pub_key = match local_storage_get(StorageKind::PERSISTENT, b"ltn_keys") {
          Some(key) => {
              key
          }
          None => {
              panic!("key is none");
          }
      };
      let call = Call::sync_pub_key(pub_key);
      T::SubmitTransaction::submit_unsigned(call);
    }
  }
}

const DB_KEY: &[u8] = b"ltn_keys";

impl<T: Trait> Module<T> {
  pub(crate) fn offchain(now: T::BlockNumber) {
  }
}
impl<T: Trait> support::unsigned::ValidateUnsigned for Module<T> {
	type Call = Call<T>;
  fn validate_unsigned(call: &Self::Call) -> TransactionValidity {
    if let Call::sync_pub_key(_) = call {
      Ok(ValidTransaction {
        priority: 0,
				requires: vec![],
        provides: vec![("abcdefg").encode()],
				// provides: vec![(current_session, ()).encode()],
				longevity: TransactionLongevity::max_value(),
				propagate: true,
      })
    } else {
      InvalidTransaction::Call.into()
    }
  }
}
// impl<T: Trait> Module<T> {
//   fn deposit_log(log: ConsensusLog<T::BlockNumber>) {
// 		let log: DigestItem<T::Hash> = DigestItem::Consensus(LN_ENGINE_ID, log.encode());
// 		<system::Module<T>>::deposit_log(log.into());
// 	}
// }
