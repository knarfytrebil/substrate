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

//! Substrate chain configurations.

use primitives::{sr25519, Pair,Public, crypto::UncheckedInto};// ed25519, 
use hb_node_primitives::{AccountId,  Balance};
use substrate_service;
use hex_literal::hex;
use substrate_telemetry::TelemetryEndpoints;
//use badger_primitives::AuthorityId as BadgerId;

use hb_node_runtime::{
	GenesisConfig,  BalancesConfig,
	SudoConfig, IndicesConfig, SystemConfig,ContractsConfig, WASM_BINARY
};
//use hb_node_runtime::SessionKeys;
use hb_node_runtime::constants::currency::DOLLARS;
//use hb_node_runtime::ContractsConfig;

//use hb_node_runtime::constants::time::SECS_PER_BLOCK;
use hb_node_runtime::constants::currency::MILLICENTS;

const STAGING_TELEMETRY_URL: &str = "wss://telemetry.polkadot.io/submit/";

/// Specialized `ChainSpec`.
pub type ChainSpec = substrate_service::ChainSpec<GenesisConfig>;


/// Flaming Fir testnet generator
pub fn flaming_fir_config() -> Result<ChainSpec, String> {
	ChainSpec::from_json_bytes(&include_bytes!("../res/flaming-fir.json")[..])
}

fn staging_testnet_config_genesis() -> GenesisConfig {
	// stash, controller, session-key
	// generated with secret:
	// for i in 1 2 3 4 ; do for j in stash controller; do subkey inspect "$secret"/fir/$j/$i; done; done
	// and
	// for i in 1 2 3 4 ; do for j in session; do subkey --ed25519 inspect "$secret"//fir//$j//$i; done; done


	// generated with secret: subkey inspect "$secret"/fir
	let endowed_accounts: Vec<AccountId> = vec![
		// 5Ff3iXP75ruzroPWRP2FYBHWnmGGBSb63857BgnzCoXNxfPo
		hex!["9ee5e5bdc0ec239eb164f865ecc345ce4c88e76ee002e0f7e318097347471809"].unchecked_into(),
	];

	const ENDOWMENT: Balance = 10_000_000 * DOLLARS;
	//const STASH: Balance = 100 * DOLLARS;

	GenesisConfig {
		system: Some(SystemConfig {
			code: WASM_BINARY.to_vec(),
			changes_trie_config: Default::default(),
		}),
		balances: Some(BalancesConfig {
			balances: endowed_accounts.iter().cloned()
				.map(|k| (k, ENDOWMENT))
				.collect(),
			vesting: vec![],
		}),
		contracts: Some(ContractsConfig {
			current_schedule: Default::default(),
			gas_price: 1 * MILLICENTS,
		}),
		indices: Some(IndicesConfig {
			ids: endowed_accounts.iter().cloned()
				.collect::<Vec<_>>(),
		}),
		sudo: Some(SudoConfig {
			key: endowed_accounts[0].clone(),
		}),
  /*		badger: Some(BadgerConfig {

		}),*/
	}
}

/// Staging testnet config.
pub fn staging_testnet_config() -> ChainSpec {
	let boot_nodes = vec![];
	ChainSpec::from_genesis(
		"Staging Testnet",
		"staging_testnet",
		staging_testnet_config_genesis,
		boot_nodes,
		Some(TelemetryEndpoints::new(vec![(STAGING_TELEMETRY_URL.to_string(), 0)])),
		None,
		None,
		None,
	)
}

/// Helper function to generate AccountId from seed
pub fn get_account_id_from_seed(seed: &str) -> AccountId {
	sr25519::Pair::from_string(&format!("//{}", seed), None)
		.expect("static values are valid; qed")
		.public()
}





// Note this is the URL for the telemetry server
//const STAGING_TELEMETRY_URL: &str = "wss://telemetry.polkadot.io/submit/";

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.

/// The chain specification option. This is expected to come in from the CLI and
/// is little more than one of a number of alternatives which can easily be converted
/// from a string (`--chain=...`) into a `ChainSpec`.
#[derive(Clone, Debug)]
pub enum Alternative {
	/// Whatever the current runtime is, with just Alice as an auth.
	Development,
	/// Whatever the current runtime is, with simple Alice/Bob auths.
	LocalTestnet,
}

/// Get it from seed
pub fn get_from_seed<TPublic: Public>(seed: &str) -> <TPublic::Pair as Pair>::Public {
	TPublic::Pair::from_string(&format!("//{}", seed), None)
		.expect("static values are valid; qed")
		.public()
}


//fn account_key(s: &str) -> AccountId {
//	get_from_seed::<AccountId>(s)
//}

//impl Alternative {
	/// Get an actual chain config from one of the alternatives.
	/*pub(crate) fn load(self) -> Result<ChainSpec, String> {
		Ok(match self {
			Alternative::Development => ChainSpec::from_genesis(
				"Development",
				"dev",
				|| testnet_genesis(vec![
					account_key("Alice")
				],
					account_key("Alice")
				),
				vec![],
				None,
				None,
				None,
				None
			),
			Alternative::LocalTestnet => ChainSpec::from_genesis(
				"Local Testnet",
				"local_testnet",
				|| testnet_genesis( vec![
					account_key("Alice"),
					account_key("Bob"),
					account_key("Charlie"),
					account_key("Dave"),
					account_key("Eve"),
					account_key("Ferdie"),
				],
					account_key("Alice"),
				),
				vec![],
				None,
				None,
				None,
				None
			),
		})
	}*/

	/*pub(crate) fn from(s: &str) -> Option<Self> {
		match s {
			"dev" => Some(Alternative::Development),
			"" | "local" => Some(Alternative::LocalTestnet),
			_ => None,
		}
	}*/
//}

fn development_config_genesis() -> GenesisConfig {
	testnet_genesis(
		vec![ get_account_id_from_seed("Alice")],
		get_account_id_from_seed("Alice"),
	)
}

/// Development config (single validator Alice)
pub fn development_config() -> ChainSpec {
	ChainSpec::from_genesis("Development", "dev", development_config_genesis, vec![], None, None, None, None)
}

/// Local testnet config (multivalidator Alice + Bob)
pub fn local_testnet_config() -> ChainSpec {
	ChainSpec::from_genesis("Local Testnet", "local_testnet", local_testnet_genesis, vec![], None, None, None, None)
}



fn local_testnet_genesis() -> GenesisConfig {
	testnet_genesis(
		vec![
			get_account_id_from_seed("Alice"),
			get_account_id_from_seed("Bob")
		],
		get_account_id_from_seed("Alice"),
	)
}

fn testnet_genesis(endowed_accounts: Vec<AccountId>, root_key: AccountId) -> GenesisConfig {
	GenesisConfig {
		system: Some(SystemConfig {
			code: WASM_BINARY.to_vec(),
			changes_trie_config: Default::default(),
		}),
		indices: Some(IndicesConfig {
			ids: endowed_accounts.clone(),
		}),
		contracts: Some(ContractsConfig {
			current_schedule: Default::default(),
			gas_price: 1 * MILLICENTS,
		}),
		balances: Some(BalancesConfig {
			balances: endowed_accounts.iter().cloned().map(|k|(k, 1 << 60)).collect(),
			vesting: vec![],
		}),
		sudo: Some(SudoConfig {
			key: root_key,
		}),
	}
}
