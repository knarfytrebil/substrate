// Copyright 2018-2019 Parity Technologies (UK) Ltd.
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

//! The Substrate runtime. This can be compiled with ``#[no_std]`, ready for Wasm.

#![cfg_attr(not(feature = "std"), no_std)]
// `construct_runtime!` does a lot of recursion and requires us to increase the limit to 256.
#![recursion_limit = "256"]

// Make the WASM binary available.
#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

use rstd::prelude::*;

//use badger_primitives::AuthorityId as BadgerId;
use badger_primitives::AuthorityList as BadgerList;
pub use contracts;
pub use contracts::Gas;
use contracts_rpc_runtime_api::ContractExecResult;
use hb_node_primitives::{
	AccountId, AccountIndex, Balance, BlockNumber, Hash, Index, Moment, Signature,
};
pub use srml_keygen::sr25519::AuthorityId as KeygenId;

use client::{
	block_builder::api::{self as block_builder_api, CheckInherentsResult, InherentData},
	impl_runtime_apis, runtime_api as client_api,
};
use elections::VoteIndex;
use finality_tracker::{DEFAULT_REPORT_LATENCY, DEFAULT_WINDOW_SIZE};
use primitives::OpaqueMetadata;
use sr_primitives::traits::{BlakeTwo256, Block as BlockT, NumberFor, StaticLookup,OpaqueKeys};
use sr_primitives::transaction_validity::TransactionValidity;
use sr_primitives::weights::Weight;
use sr_primitives::{create_runtime_str, generic, impl_opaque_keys,  ApplyResult}; //key_types
#[cfg(any(feature = "std", test))]
use version::NativeVersion;
use version::RuntimeVersion;

pub use balances::Call as BalancesCall;

#[cfg(any(feature = "std", test))]
pub use sr_primitives::BuildStorage;

pub use sr_primitives::{Perbill, Permill};
pub use support::{
	construct_runtime, parameter_types,
	traits::{Currency, Randomness},
	StorageValue,
};
pub use timestamp::Call as TimestampCall;

/// Implementations of some helper traits passed into runtime modules as associated types.
pub mod impls;
use impls::{  LinearWeightToFee, TargetedFeeAdjustment};//Author,CurrencyToVoteHandler

/// Constant values used within the runtime.
pub mod constants;
use constants::{currency::*, time::*};

/// Runtime version.
pub const VERSION: RuntimeVersion = RuntimeVersion {
	spec_name: create_runtime_str!("hb-node"),
	impl_name: create_runtime_str!("substrate-hb-node"),
	authoring_version: 10,
	// Per convention: if the runtime behavior changes, increment spec_version
	// and set impl_version to equal spec_version. If only runtime
	// implementation changes and behavior does not, then leave spec_version as
	// is and increment impl_version.
	spec_version: 140,
	impl_version: 140,
	apis: RUNTIME_API_VERSIONS,
};

/// Native version.
#[cfg(any(feature = "std", test))]
pub fn native_version() -> NativeVersion {
	NativeVersion {
		runtime_version: VERSION,
		can_author_with: Default::default(),
	}
}

type NegativeImbalance = <Balances as Currency<AccountId>>::NegativeImbalance;

pub type SessionHandlers = (Badger,);

impl_opaque_keys! {
	pub struct SessionKeys {
		pub hbbft: Badger,
	}
}

parameter_types! {
	pub const BlockHashCount: BlockNumber = 250;
	pub const MaximumBlockWeight: Weight = 1_000_000_000;
	pub const AvailableBlockRatio: Perbill = Perbill::from_percent(75);
	pub const MaximumBlockLength: u32 = 5 * 1024 * 1024;
	pub const Version: RuntimeVersion = VERSION;
}

impl system::Trait for Runtime {
	type Origin = Origin;
	type Call = Call;
	type Index = Index;
	type BlockNumber = BlockNumber;
	type Hash = Hash;
	type Hashing = BlakeTwo256;
	type AccountId = AccountId;
	type Lookup = Indices;
	type Header = generic::Header<BlockNumber, BlakeTwo256>;
	type Event = Event;
	type BlockHashCount = BlockHashCount;
	type MaximumBlockWeight = MaximumBlockWeight;
	type MaximumBlockLength = MaximumBlockLength;
	type AvailableBlockRatio = AvailableBlockRatio;
	type Version = Version;
}

impl contracts::Trait for Runtime {
	type Currency = Balances;
	type Call = Call;
	type Event = Event;
	type Randomness = RandomnessCollectiveFlip;
	type DetermineContractAddress = contracts::SimpleAddressDeterminator<Runtime>;
	type ComputeDispatchFee = contracts::DefaultDispatchFeeComputor<Runtime>;
	type TrieIdGenerator = contracts::TrieIdFromParentCounter<Runtime>;
	type GasPayment = ();
	type RentPayment = ();
	type SignedClaimHandicap = contracts::DefaultSignedClaimHandicap;
	type TombstoneDeposit = TombstoneDeposit;
	type StorageSizeOffset = contracts::DefaultStorageSizeOffset;
	type RentByteFee = RentByteFee;
	type RentDepositOffset = RentDepositOffset;
	type SurchargeReward = SurchargeReward;
	type TransferFee = ContractTransferFee;
	type CreationFee = ContractCreationFee;
	type TransactionBaseFee = ContractTransactionBaseFee;
	type TransactionByteFee = ContractTransactionByteFee;
	type ContractFee = ContractFee;
	type CallBaseFee = contracts::DefaultCallBaseFee;
	type InstantiateBaseFee = contracts::DefaultInstantiateBaseFee;
	//type CreateBaseFee = contracts::DefaultCreateBaseFee;
	type MaxDepth = contracts::DefaultMaxDepth;
	type MaxValueSize = contracts::DefaultMaxValueSize;
	type BlockGasLimit = contracts::DefaultBlockGasLimit;
	type Time = Timestamp;
}

parameter_types! {
	pub const EpochDuration: u64 = EPOCH_DURATION_IN_SLOTS;
	pub const ExpectedBlockTime: Moment = MILLISECS_PER_BLOCK;
	pub const SurchargeReward: Balance = 150 * DOLLARS;
	pub const RentDepositOffset: Balance = 1000 * DOLLARS;
	pub const TombstoneDeposit: Balance = 1 * DOLLARS;
	pub const RentByteFee: Balance = 1 * DOLLARS;
}

impl indices::Trait for Runtime {
	type AccountIndex = AccountIndex;
	type IsDeadAccount = Balances;
	type ResolveHint = indices::SimpleResolveHint<Self::AccountId, Self::AccountIndex>;
	type Event = Event;
}

parameter_types! {
	pub const ExistentialDeposit: Balance = 1 * CENTS;
	pub const TransferFee: Balance = 1 * CENTS;
	pub const CreationFee: Balance = 1 * CENTS;
}

impl balances::Trait for Runtime {
	/// The type for recording an account's balance.
	type Balance = Balance;
	/// What to do if an account's free balance gets zeroed.
	type OnFreeBalanceZero = ();
	/// What to do if a new account is created.
	type OnNewAccount = Indices;
	/// The ubiquitous event type.
	type Event = Event;
	type DustRemoval = ();
	type TransferPayment = ();
	type ExistentialDeposit = ExistentialDeposit;
	type TransferFee = TransferFee;
	type CreationFee = CreationFee;
}

parameter_types! {
	pub const MinimumPeriod: Moment = SLOT_DURATION / 2;
}
impl timestamp::Trait for Runtime {
	type Moment = Moment;
	type OnTimestampSet = ();
	type MinimumPeriod = MinimumPeriod;
}

parameter_types! {
	pub const UncleGenerations: BlockNumber = 5;
}

impl authorship::Trait for Runtime {
	type FindAuthor = ();
	type UncleGenerations = UncleGenerations;
	type FilterUncle = ();
	type EventHandler = ();
}

parameter_types! {
	pub const LaunchPeriod: BlockNumber = 28 * 24 * 60 * MINUTES;
	pub const VotingPeriod: BlockNumber = 28 * 24 * 60 * MINUTES;
	pub const EmergencyVotingPeriod: BlockNumber = 3 * 24 * 60 * MINUTES;
	pub const MinimumDeposit: Balance = 100 * DOLLARS;
	pub const EnactmentPeriod: BlockNumber = 30 * 24 * 60 * MINUTES;
	pub const CooloffPeriod: BlockNumber = 28 * 24 * 60 * MINUTES;
}

parameter_types! {
	pub const CandidacyBond: Balance = 10 * DOLLARS;
	pub const VotingBond: Balance = 1 * DOLLARS;
	pub const VotingFee: Balance = 2 * DOLLARS;
	pub const PresentSlashPerVoter: Balance = 1 * CENTS;
	pub const CarryCount: u32 = 6;
	// one additional vote should go by before an inactive voter can be reaped.
	pub const InactiveGracePeriod: VoteIndex = 1;
	pub const ElectionsVotingPeriod: BlockNumber = 2 * DAYS;
	pub const DecayRatio: u32 = 0;
}

parameter_types! {
	pub const ProposalBond: Permill = Permill::from_percent(5);
	pub const ProposalBondMinimum: Balance = 1 * DOLLARS;
	pub const SpendPeriod: BlockNumber = 1 * DAYS;
	pub const Burn: Permill = Permill::from_percent(50);
}

parameter_types! {
	pub const ContractTransferFee: Balance = 1 * CENTS;
	pub const ContractCreationFee: Balance = 1 * CENTS;
	pub const ContractTransactionBaseFee: Balance = 1 * CENTS;
	pub const ContractTransactionByteFee: Balance = 10 * MILLICENTS;
	pub const ContractFee: Balance = 1 * CENTS;
}

impl sudo::Trait for Runtime {
	type Event = Event;
	type Proposal = Call;
}

impl srml_badger::Trait for Runtime {
	type Event = Event;
}
use system::offchain::TransactionSubmitter;

type SubmitTransaction = TransactionSubmitter<KeygenId, Runtime, UncheckedExtrinsic>;
impl srml_keygen::Trait for Runtime {
	type Event = Event;
	type AuthorityId = KeygenId;
	type Call = Call;
	type SubmitTransaction = SubmitTransaction;
}

parameter_types! {
	pub const WindowSize: BlockNumber = DEFAULT_WINDOW_SIZE.into();
	pub const ReportLatency: BlockNumber = DEFAULT_REPORT_LATENCY.into();
}

impl finality_tracker::Trait for Runtime {
	type OnFinalizationStalled = ();
	type WindowSize = WindowSize;
	type ReportLatency = ReportLatency;
}

construct_runtime!(
	pub enum Runtime where
		Block = Block,
		NodeBlock = hb_node_primitives::Block,
		UncheckedExtrinsic = UncheckedExtrinsic
	{
		System: system::{Module, Call, Storage, Config, Event},
		Timestamp: timestamp::{Module, Call, Storage, Inherent},
		Authorship: authorship::{Module, Call, Storage, Inherent},
		Badger: srml_badger::{Module, Call, Storage, Event},
		Session: session::{Module, Call, Storage, Event, Config<T>},
		Keygen: srml_keygen::{Module, Call, Storage, Event,ValidateUnsigned},
		Indices: indices,
		Balances: balances,
		Contracts: contracts,
		FinalityTracker: finality_tracker::{Module, Call, Inherent},
		RandomnessCollectiveFlip: randomness_collective_flip::{Module, Call, Storage},
		Sudo: sudo,
	}
);

parameter_types! {
	pub const TransactionBaseFee: Balance = 1 * CENTS;
	pub const TransactionByteFee: Balance = 10 * MILLICENTS;
	// setting this to zero will disable the weight fee.
	pub const WeightFeeCoefficient: Balance = 1_000;
	// for a sane configuration, this should always be less than `AvailableBlockRatio`.
	pub const TargetBlockFullness: Perbill = Perbill::from_percent(25);
}

impl transaction_payment::Trait for Runtime {
	type Currency = Balances;
	type OnTransactionPayment = ();
	type TransactionBaseFee = TransactionBaseFee;
	type TransactionByteFee = TransactionByteFee;
	type WeightToFee = LinearWeightToFee<WeightFeeCoefficient>;
	type FeeMultiplierUpdate = TargetedFeeAdjustment<TargetBlockFullness>;
}

parameter_types! {
	pub const DisabledValidatorsThreshold: Perbill = Perbill::from_percent(33);
}
impl session::Trait for Runtime {
	type OnSessionEnding = Badger;
	type SessionHandler = <SessionKeys as OpaqueKeys>::KeyTypeIdProviders;
	type ShouldEndSession = Badger;
	type Event = Event;
	type Keys = SessionKeys;
	type ValidatorId = <Self as system::Trait>::AccountId;
	type ValidatorIdOf = ();
	type SelectInitialValidators = ();
	type DisabledValidatorsThreshold = DisabledValidatorsThreshold;
}


/// The address format for describing accounts.
pub type Address = <Indices as StaticLookup>::Source;
/// Block header type as expected by this runtime.
pub type Header = generic::Header<BlockNumber, BlakeTwo256>;
/// Block type as expected by this runtime.
pub type Block = generic::Block<Header, UncheckedExtrinsic>;
/// A Block signed with a Justification
pub type SignedBlock = generic::SignedBlock<Block>;
/// BlockId type as expected by this runtime.
pub type BlockId = generic::BlockId<Block>;
/// The SignedExtension to the basic transaction logic.
pub type SignedExtra = (
	system::CheckVersion<Runtime>,
	system::CheckGenesis<Runtime>,
	system::CheckEra<Runtime>,
	system::CheckNonce<Runtime>,
	system::CheckWeight<Runtime>,
	transaction_payment::ChargeTransactionPayment<Runtime>,
	contracts::CheckBlockGasLimit<Runtime>,
);
/// Unchecked extrinsic type as expected by this runtime.
pub type UncheckedExtrinsic = generic::UncheckedExtrinsic<Address, Call, Signature, SignedExtra>;
/// Extrinsic type that has already been checked.
pub type CheckedExtrinsic = generic::CheckedExtrinsic<AccountId, Call, SignedExtra>;
/// Executive: handles dispatch to the various modules.
pub type Executive =
	executive::Executive<Runtime, Block, system::ChainContext<Runtime>, Runtime, AllModules>;

impl_runtime_apis! {
	impl client_api::Core<Block> for Runtime {
		fn version() -> RuntimeVersion {
			VERSION
		}

		fn execute_block(block: Block) {
			Executive::execute_block(block)
		}

		fn initialize_block(header: &<Block as BlockT>::Header) {
			Executive::initialize_block(header)
		}
	}
	
	impl badger_primitives::BadgerApi<Block> for Runtime {
		fn badger_authorities() -> BadgerList {
			Badger::badger_authorities()
		}
	}

	impl client_api::Metadata<Block> for Runtime {
		fn metadata() -> OpaqueMetadata {
			Runtime::metadata().into()
		}
	}

	impl block_builder_api::BlockBuilder<Block> for Runtime {
		fn apply_extrinsic(extrinsic: <Block as BlockT>::Extrinsic) -> ApplyResult {
			Executive::apply_extrinsic(extrinsic)
		}

		fn finalize_block() -> <Block as BlockT>::Header {
			Executive::finalize_block()
		}

		fn inherent_extrinsics(data: InherentData) -> Vec<<Block as BlockT>::Extrinsic> {
			data.create_extrinsics()
		}

		fn check_inherents(block: Block, data: InherentData) -> CheckInherentsResult {
			data.check_extrinsics(&block)
		}

		fn random_seed() -> <Block as BlockT>::Hash {
			RandomnessCollectiveFlip::random_seed()
		}
	}

	impl client_api::TaggedTransactionQueue<Block> for Runtime {
		fn validate_transaction(tx: <Block as BlockT>::Extrinsic) -> TransactionValidity {
			Executive::validate_transaction(tx)
		}
	}

	impl offchain_primitives::OffchainWorkerApi<Block> for Runtime {
		fn offchain_worker(number: NumberFor<Block>) {
			Executive::offchain_worker(number)
		}
	}



	impl substrate_session::SessionKeys<Block> for Runtime {
		fn generate_session_keys(seed: Option<Vec<u8>>) -> Vec<u8> {
			//let seed = seed.as_ref().map(|s| rstd::str::from_utf8(&s).expect("Seed is an utf8 string"));
			//Vec::new()
			SessionKeys::generate(seed)
		}
	}

	impl system_rpc_runtime_api::AccountNonceApi<Block, AccountId, Index> for Runtime {
		fn account_nonce(account: AccountId) -> Index {
			System::account_nonce(account)
		}
	}

	impl contracts_rpc_runtime_api::ContractsApi<Block, AccountId, Balance> for Runtime {
		fn call(
			origin: AccountId,
			dest: AccountId,
			value: Balance,
			gas_limit: u64,
			input_data: Vec<u8>,
		) -> ContractExecResult {
			let exec_result = Contracts::bare_call(
				origin,
				dest.into(),
				value,
				gas_limit,
				input_data,
			);
			match exec_result {
				Ok(v) => ContractExecResult::Success {
					status: v.status,
					data: v.data,
				},
				Err(_) => ContractExecResult::Error,
			}
		}

		fn get_storage(
			address: AccountId,
			key: [u8; 32],
		) -> contracts_rpc_runtime_api::GetStorageResult {
			Contracts::get_storage(address, key).map_err(|rpc_err| {
				use contracts::GetStorageError;
				use contracts_rpc_runtime_api::{GetStorageError as RpcGetStorageError};
				/// Map the contract error into the RPC layer error.
				match rpc_err {
					GetStorageError::ContractDoesntExist => RpcGetStorageError::ContractDoesntExist,
					GetStorageError::IsTombstone => RpcGetStorageError::IsTombstone,
				}
			})
		}
	}

	//not used for now and has no_std problems
	/*	impl HbbftApi<Block> for Runtime
		{

		 fn badger_pending_change(digest: &DigestFor<Block>)
			-> Option<ScheduledChange<NumberFor<Block>>>
			{
				None
			}



		fn badger_forced_change(digest: &DigestFor<Block>)
			-> Option<(NumberFor<Block>, ScheduledChange<NumberFor<Block>>)>
			{
				None
			}

		fn badger_authorities() -> Vec<PAuthorityId>
		{
			Vec::new()
		}


	}*/
}
