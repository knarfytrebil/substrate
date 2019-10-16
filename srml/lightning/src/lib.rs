#![cfg_attr(not(feature = "std"), no_std)]

use rstd::prelude::*;
use codec::{self as codec, Encode, Decode, Error};
use support::{decl_module, decl_storage, decl_event};
use system::{ensure_none, ensure_signed};
use sr_primitives::{generic::DigestItem};
use ln_primitives::{
  LN_ENGINE_ID, ConsensusLog,
  Account, Tx
};

pub trait Trait: system::Trait {
}

decl_storage! {
  trait Store for Module<T: Trait> as Lightning {
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
  }
}

// impl<T: Trait> Module<T> {
//   fn deposit_log(log: ConsensusLog<T::BlockNumber>) {
// 		let log: DigestItem<T::Hash> = DigestItem::Consensus(LN_ENGINE_ID, log.encode());
// 		<system::Module<T>>::deposit_log(log.into());
// 	}
// }
