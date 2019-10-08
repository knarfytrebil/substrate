#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
use serde::Serialize;
use codec::{Encode, Decode, Codec};
use client::decl_runtime_apis;
use sr_primitives::ConsensusEngineId;
use rstd::vec::Vec;
// use runtime_primitives::traits::NumberFor;
pub const LN_ENGINE_ID: ConsensusEngineId = *b"LNID";
#[cfg_attr(feature = "std", derive(Debug, Serialize))]
#[derive(Decode, Encode, PartialEq, Eq, Clone)]
pub struct Account {
  // pub id: Vec<u8>,
  pub id: u8,
  pub wallet_id: u8,
}
#[cfg_attr(feature = "std", derive(Debug, Serialize))]
#[derive(Decode, Encode, PartialEq, Eq, Clone)]
pub struct Tx {
  pub amount: u64,
}

#[cfg_attr(feature = "std", derive(Debug, Serialize))]
#[derive(Decode, Encode, PartialEq, Eq, Clone)]
pub struct LnNode {
  pub node_key: Vec<u8>,
}

#[cfg_attr(feature = "std", derive(Debug, Serialize))]
#[derive(Decode, Encode, PartialEq, Eq, Clone)]
pub enum ConsensusLog {
  #[codec(index = "1")]
  FundChannel(Vec<Vec<u8>>),// args: Vec<String>
  #[codec(index = "2")]
  CloseChannel(Vec<u8>),// line: String
  #[codec(index = "3")]
  ForceCloseAllChannel(),
  // ListChannel(),
  #[codec(index = "4")]
  PayInvoice(Vec<Vec<u8>>),// args: Vec<String>
  #[codec(index = "5")]
  CreateInvoice(Vec<u8>),// line: String
  #[codec(index = "6")]
  ConnectPeer(Vec<u8>),// node: String
  // ListPeer(),
}

decl_runtime_apis! {
	pub trait LnApi {
		fn link_bridge();
	}
}
