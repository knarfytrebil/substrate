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

#![warn(unused_extern_crates)]

//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.

use badger::{self, badger_import_queue, run_honey_badger, Config as BadgerConfig};
use badger_primitives::app::Public;
use badger_primitives::app::Signature;
use client::{self, LongestChain};
                    //use futures::prelude::*;
                    //use futures03::compat;
use futures03::compat::Future01CompatExt;
use futures03::future::FutureExt;
use futures03::future::TryFutureExt;
use hb_node_executor;
use hb_node_primitives::Block;
use client::{Client, LocalCallExecutor};
use substrate_service::{Service, NetworkStatus};

use hb_node_runtime::{GenesisConfig,RuntimeApi};
                                    //use primitives::Pair;
use keygen::{self};
use sr_primitives::traits::Block as BlockT;

use std::sync::Arc;
//use std::time::Duration;
use substrate_service::{
  config::Configuration,
  error::Error as ServiceError,
  AbstractService, //ServiceBuilder,
};

use client_db::Backend;


use inherents::InherentDataProviders;
//use network::config::DummyFinalityProofRequestBuilder;
use network::construct_simple_protocol;
use transaction_pool::{self, txpool::{Pool as TransactionPool}};
//use substrate_service::construct_service_factory;
use log::info;

use hb_node_executor::NativeExecutor;
use network::NetworkService;
use offchain::OffchainWorkers;
use primitives::Blake2Hasher;
use parking_lot::Mutex;

/// Node specific configuration
pub struct NodeConfig
{
  pub inherent_data_providers: InherentDataProviders,
}

impl Default for NodeConfig
{
  fn default() -> NodeConfig
  {
    NodeConfig {
      inherent_data_providers: InherentDataProviders::new(),
    }
  }
}

construct_simple_protocol! {
	/// Demo protocol attachment for substrate.
	pub struct NodeProtocol where Block = Block { }
}

/// Starts a `ServiceBuilder` for a full service.
///
/// Use this macro if you don't actually need the full service, but just the builder in order to
/// be able to perform chain operations.
macro_rules! new_full_start {
	($config:expr) => {{
		type RpcExtension = jsonrpc_core::IoHandler<substrate_rpc::Metadata>;
		//let mut import_setup = None;
		let inherent_data_providers = inherents::InherentDataProviders::new();
		let mut import_setup = None;
		let builder = substrate_service::ServiceBuilder::new_full::<
			hb_node_primitives::Block, hb_node_runtime::RuntimeApi, hb_node_executor::Executor
		>($config)?
			.with_select_chain(|_config, backend| {
				Ok(client::LongestChain::new(backend.clone()))
			})?
			.with_transaction_pool(|config, client|
				Ok(transaction_pool::txpool::Pool::new(config, transaction_pool::FullChainApi::new(client)))
			)?
			.with_import_queue(|_config, client,  select_chain, _transaction_pool| {
        #[allow(deprecated)]
        // let fprb = Box::new(DummyFinalityProofRequestBuilder::default()) as Box<_>;
		let block_import = badger::block_importer(client.clone(), &*client.clone(), select_chain.unwrap(),).expect("Invalid setup. QWOP.");
		//let justification_import = block_import.clone();
		import_setup=Some(block_import.clone());
        badger_import_queue::<_, _, Public, Signature>(
          Box::new(block_import),
          None,
          None,
          client,
          inherent_data_providers.clone(),
        )
        .map_err(Into::into)
			
			})?
			.with_rpc_extensions_key(|client, pool, _backend,ks| -> RpcExtension {
				hb_node_rpc::create(client, pool,ks)
			})?;

		(builder,  import_setup,inherent_data_providers)
	}}
}

/// Creates a full service from the configuration.
///
/// We need to use a macro because the test suit doesn't work with an opaque service. It expects
/// concrete types instead.
macro_rules! new_full {
	($config:expr, $with_startup_data: expr) => {{
		use futures::sync::mpsc;
		use network::DhtEvent;
    //use futures::Future;
    //let nconf_name = $config.n_conf_file.clone();
    let node_name = $config.name.clone();
    let node_key = $config.node_key.clone();
    let dev_seed = $config.dev_key_seed.clone();
    let (
			is_authority,
			_force_authoring,
			_name,
     ) = (
			$config.roles.is_authority(),
			$config.force_authoring,
			$config.name.clone(),
		);

		// sentry nodes announce themselves as authorities to the network
		// and should run the same protocols authorities do, but it should
		// never actively participate in any consensus process.
		let participates_in_consensus = is_authority && !$config.sentry_mode;

		let (builder, import_setup, inherent_data_providers) = new_full_start!($config);
    let back = builder.backend().clone();
		
		// Dht event channel from the network to the authority discovery module. Use bounded channel to ensure
		// back-pressure. Authority discovery is triggering one event per authority within the current authority set.
		// This estimates the authority set size to be somewhere below 10 000 thereby setting the channel buffer size to
		// 10 000.
		let (dht_event_tx, _dht_event_rx) =
			mpsc::channel::<DhtEvent>(10_000);
      let service = builder
      .with_network_protocol(|_| Ok(crate::service::NodeProtocol::new()))?
      .with_opt_finality_proof_provider(|_client, _| Ok(None))?
      .with_dht_event_tx(dht_event_tx)?
      .build()?;

		//($with_startup_data)(&block_import, &babe_link);

		if participates_in_consensus { //may need to be changed

      let client = service.client().clone();
      let t_pool = service.transaction_pool();
      let select_chain = service
        .select_chain()
        .ok_or(ServiceError::SelectChainRequired)?;

     //  let nconf = match &nconf_name
     // {
     //   Some(name) => PathBuf::from(name),
     //   None => PathBuf::from("./nodes.json"),
     // };
      let bc=BadgerConfig{
		  name: Some(node_name.to_string()),
          batch_size:20,  
	  };
	  let b_i=import_setup.expect("Should be initialized by now");
      let badger = run_honey_badger(
        client,
		t_pool,
		bc,
        //BadgerConfig::from_json_file_with_name(nconf, &node_name).unwrap(),
        service.network(),
        service.on_exit().clone().compat().map(|_| {
          info!("OnExit");
          ()
        }),
        Arc::new(Mutex::new(b_i)), //block_import?
        //service.config().custom.inherent_data_providers.clone(),
        inherent_data_providers.clone(),
        select_chain,
		service.keystore(),
		node_key,
		dev_seed,
      )?;    
      let key_gen = keygen::run_key_gen(
        service.network().local_peer_id(),
        (2, 5),
        3,
        service.keystore(),
        service.client(),
        service.network(),
        back,
      )?;
      let svc = futures03::future::select(service.on_exit().clone().compat(), key_gen).map(|_| Ok::<(), ()>(())).compat();

      
      service.spawn_essential_task( svc);
			service.spawn_essential_task(badger.map(|_| Ok::<(), ()>(())).compat());
		}

		// if the node isn't actively participating in consensus then it doesn't
		// need a keystore, regardless of which protocol we use below. <-- not true for HB, i think. also, weird; 
		//let _keystore = 	Some(service.keystore())  ;

		Ok((service, inherent_data_providers))
	}};
	($config:expr) => {{
		new_full!($config, |_, _| {})
	}}
}

#[allow(dead_code)]
type ConcreteBlock = hb_node_primitives::Block;
#[allow(dead_code)]
type ConcreteClient =
	Client<
		Backend<ConcreteBlock>,
		LocalCallExecutor<Backend<ConcreteBlock>,
		NativeExecutor<hb_node_executor::Executor>>,
		ConcreteBlock,
		hb_node_runtime::RuntimeApi
	>;
#[allow(dead_code)]
type ConcreteBackend = Backend<ConcreteBlock>;

/// A specialized configuration object for setting up the node..
pub type NodeConfiguration<C> = Configuration<C, GenesisConfig, crate::chain_spec::Extensions>;

/// Builds a new service for a full client.
pub fn new_full<C: Send + Default + 'static>(config: NodeConfiguration<C>)
-> Result<
	Service<
		ConcreteBlock,
		ConcreteClient,
		LongestChain<ConcreteBackend, ConcreteBlock>,
		NetworkStatus<ConcreteBlock>,
		NetworkService<ConcreteBlock, crate::service::NodeProtocol, <ConcreteBlock as BlockT>::Hash>,
		TransactionPool<transaction_pool::FullChainApi<ConcreteClient, ConcreteBlock>>,
		OffchainWorkers<
			ConcreteClient,
			<ConcreteBackend as client::backend::Backend<Block, Blake2Hasher>>::OffchainStorage,
			ConcreteBlock,
		>
	>,
	ServiceError,
>
{
	new_full!(config).map(|(service, _)| service)
}

/// Builds a new service for a light client.
pub fn new_light<C: Send + Default + 'static>(config: NodeConfiguration<C>)
-> Result<impl AbstractService, ServiceError> {
	type RpcExtension = jsonrpc_core::IoHandler<substrate_rpc::Metadata>;
	let inherent_data_providers = InherentDataProviders::new();

	let service = substrate_service::ServiceBuilder::new_light::<Block, RuntimeApi, hb_node_executor::Executor>(config)?
		.with_select_chain(|_config, backend| {
			Ok(client::LongestChain::new(backend.clone()))
		})?
		.with_transaction_pool(|config, client|
			Ok(TransactionPool::new(config, transaction_pool::FullChainApi::new(client)))
		)?
		.with_import_queue(|_config, client, mut _select_chain, _transaction_pool| {
      #[allow(deprecated)]
      // let fprb = Box::new(DummyFinalityProofRequestBuilder::default()) as Box<_>;
      let block_import = client.clone();
      //let justification_import = block_import.clone();

		//	let finality_proof_import = grandpa_block_import.clone();
		//	let finality_proof_request_builder =
	//			finality_proof_import.create_finality_proof_request_builder();

  badger_import_queue::<_, _, Public, Signature>(
    Box::new(block_import),
    None,
    None,
    client,
    inherent_data_providers.clone(),
  )
  .map_err(Into::into)

		})?
		.with_network_protocol(|_| Ok(NodeProtocol::new()))?
    .with_opt_finality_proof_provider(|_client, _| Ok(None))?  //may need to add it
		.with_rpc_extensions_key(|client, pool, _backend,ks| -> RpcExtension {
			hb_node_rpc::create(client, pool,ks)
		})?
		.build()?;

	Ok(service)
}

