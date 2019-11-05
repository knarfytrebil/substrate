#![feature(prelude_import)]
#![no_std]
#![doc = " Substrate Node Template CLI library."]
#![warn(missing_docs)]
#![warn(unused_extern_crates)]
#[prelude_import]
use ::std::prelude::v1::*;
#[macro_use]
extern crate std as std;
mod chain_spec {
    use node_template_runtime::{
        AccountId, AuraConfig, AuraId, BalancesConfig, GenesisConfig, IndicesConfig, SudoConfig,
        SystemConfig, TimestampConfig, WASM_BINARY,
    };
    use primitives::{ed25519, sr25519, Pair};
    use substrate_service;
    #[doc = " Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type."]
    pub type ChainSpec = substrate_service::ChainSpec<GenesisConfig>;
    #[doc = " The chain specification option. This is expected to come in from the CLI and"]
    #[doc = " is little more than one of a number of alternatives which can easily be converted"]
    #[doc = " from a string (`--chain=...`) into a `ChainSpec`."]
    pub enum Alternative {
        #[doc = " Whatever the current runtime is, with just Alice as an auth."]
        Development,
        #[doc = " Whatever the current runtime is, with simple Alice/Bob auths."]
        LocalTestnet,
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::std::clone::Clone for Alternative {
        #[inline]
        fn clone(&self) -> Alternative {
            match (&*self,) {
                (&Alternative::Development,) => Alternative::Development,
                (&Alternative::LocalTestnet,) => Alternative::LocalTestnet,
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::std::fmt::Debug for Alternative {
        fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
            match (&*self,) {
                (&Alternative::Development,) => {
                    let mut debug_trait_builder = f.debug_tuple("Development");
                    debug_trait_builder.finish()
                }
                (&Alternative::LocalTestnet,) => {
                    let mut debug_trait_builder = f.debug_tuple("LocalTestnet");
                    debug_trait_builder.finish()
                }
            }
        }
    }
    fn authority_key(s: &str) -> AuraId {
        ed25519::Pair::from_string(
            &::alloc::fmt::format(::std::fmt::Arguments::new_v1(
                &["//"],
                &match (&s,) {
                    (arg0,) => [::std::fmt::ArgumentV1::new(arg0, ::std::fmt::Display::fmt)],
                },
            )),
            None,
        )
        .expect("static values are valid; qed")
        .public()
    }
    fn account_key(s: &str) -> AccountId {
        sr25519::Pair::from_string(
            &::alloc::fmt::format(::std::fmt::Arguments::new_v1(
                &["//"],
                &match (&s,) {
                    (arg0,) => [::std::fmt::ArgumentV1::new(arg0, ::std::fmt::Display::fmt)],
                },
            )),
            None,
        )
        .expect("static values are valid; qed")
        .public()
    }
    impl Alternative {
        #[doc = " Get an actual chain config from one of the alternatives."]
        pub(crate) fn load(self) -> Result<ChainSpec, String> {
            Ok(match self {
                Alternative::Development => ChainSpec::from_genesis(
                    "Development",
                    "dev",
                    || {
                        testnet_genesis(
                            <[_]>::into_vec(box [authority_key("Alice")]),
                            <[_]>::into_vec(box [account_key("Alice")]),
                            account_key("Alice"),
                        )
                    },
                    <[_]>::into_vec(box []),
                    None,
                    None,
                    None,
                    None,
                ),
                Alternative::LocalTestnet => ChainSpec::from_genesis(
                    "Local Testnet",
                    "local_testnet",
                    || {
                        testnet_genesis(
                            <[_]>::into_vec(box [authority_key("Alice"), authority_key("Bob")]),
                            <[_]>::into_vec(box [
                                account_key("Alice"),
                                account_key("Bob"),
                                account_key("Charlie"),
                                account_key("Dave"),
                                account_key("Eve"),
                                account_key("Ferdie"),
                            ]),
                            account_key("Alice"),
                        )
                    },
                    <[_]>::into_vec(box []),
                    None,
                    None,
                    None,
                    None,
                ),
            })
        }
        pub(crate) fn from(s: &str) -> Option<Self> {
            match s {
                "dev" => Some(Alternative::Development),
                "" | "local" => Some(Alternative::LocalTestnet),
                _ => None,
            }
        }
    }
    fn testnet_genesis(
        initial_authorities: Vec<AuraId>,
        endowed_accounts: Vec<AccountId>,
        root_key: AccountId,
    ) -> GenesisConfig {
        GenesisConfig {
            system: Some(SystemConfig {
                code: WASM_BINARY.to_vec(),
                changes_trie_config: Default::default(),
            }),
            aura: Some(AuraConfig {
                authorities: initial_authorities.clone(),
            }),
            timestamp: Some(TimestampConfig { minimum_period: 5 }),
            indices: Some(IndicesConfig {
                ids: endowed_accounts.clone(),
            }),
            balances: Some(BalancesConfig {
                balances: endowed_accounts
                    .iter()
                    .cloned()
                    .map(|k| (k, 1 << 60))
                    .collect(),
                vesting: <[_]>::into_vec(box []),
            }),
            sudo: Some(SudoConfig { key: root_key }),
        }
    }
}
mod service {
    #![doc = " Service and ServiceFactory implementation. Specialized wrapper over Substrate service."]
    #![warn(unused_extern_crates)]
    use basic_authorship::ProposerFactory;
    use consensus::{import_queue, start_aura, AuraImportQueue, SlotDuration};
    use futures::prelude::*;
    use inherents::InherentDataProviders;
    use log::info;
    use network::construct_simple_protocol;
    use node_template_runtime::{self, opaque::Block, GenesisConfig, RuntimeApi, WASM_BINARY};
    use primitives::{ed25519::Pair, Pair as PairT};
    use std::sync::Arc;
    use substrate_client::{self as client, LongestChain};
    use substrate_executor::native_executor_instance;
    pub use substrate_executor::NativeExecutor;
    use substrate_service::construct_service_factory;
    use substrate_service::{
        error::Error as ServiceError, FactoryFullConfiguration, FullBackend, FullClient,
        FullComponents, FullExecutor, LightBackend, LightClient, LightComponents, LightExecutor,
    };
    use transaction_pool::{self, txpool::Pool as TransactionPool};
    #[doc = " A unit struct which implements `NativeExecutionDispatch` feeding in the hard-coded runtime."]
    pub struct Executor;
    impl ::substrate_executor::NativeExecutionDispatch for Executor {
        fn native_equivalent() -> &'static [u8] {
            WASM_BINARY
        }
        fn dispatch(
            ext: &mut ::substrate_executor::Externalities<::substrate_executor::Blake2Hasher>,
            method: &str,
            data: &[u8],
        ) -> ::substrate_executor::error::Result<Vec<u8>> {
            ::substrate_executor::with_native_environment(ext, move || {
                node_template_runtime::api::dispatch(method, data)
            })?
            .ok_or_else(|| ::substrate_executor::error::Error::MethodNotFound(method.to_owned()))
        }
        fn native_version() -> ::substrate_executor::NativeVersion {
            node_template_runtime::native_version()
        }
        fn new(default_heap_pages: Option<u64>) -> ::substrate_executor::NativeExecutor<Executor> {
            ::substrate_executor::NativeExecutor::new(default_heap_pages)
        }
    }
    pub struct NodeConfig {
        inherent_data_providers: InherentDataProviders,
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::std::default::Default for NodeConfig {
        #[inline]
        fn default() -> NodeConfig {
            NodeConfig {
                inherent_data_providers: ::std::default::Default::default(),
            }
        }
    }
    #[doc = r" Demo protocol attachment for substrate."]
    pub struct NodeProtocol {}
    impl NodeProtocol {
        #[doc = " Instantiate a node protocol handler."]
        pub fn new() -> Self {
            Self {}
        }
    }
    impl ::substrate_network::specialization::NetworkSpecialization<Block> for NodeProtocol {
        fn status(&self) -> Vec<u8> {
            Vec::new()
        }
        fn on_connect(
            &mut self,
            _ctx: &mut ::substrate_network::Context<Block>,
            _who: ::substrate_network::PeerId,
            _status: ::substrate_network::StatusMessage<Block>,
        ) {
        }
        fn on_disconnect(
            &mut self,
            _ctx: &mut ::substrate_network::Context<Block>,
            _who: ::substrate_network::PeerId,
        ) {
        }
        fn on_message(
            &mut self,
            _ctx: &mut ::substrate_network::Context<Block>,
            _who: ::substrate_network::PeerId,
            _message: Vec<u8>,
        ) {
        }
        fn on_event(&mut self, _event: ::substrate_network::specialization::Event) {}
        fn on_abort(&mut self) {}
        fn maintain_peers(&mut self, _ctx: &mut ::substrate_network::Context<Block>) {}
        fn on_block_imported(
            &mut self,
            _ctx: &mut ::substrate_network::Context<Block>,
            _hash: <Block as ::substrate_network::BlockT>::Hash,
            _header: &<Block as ::substrate_network::BlockT>::Header,
        ) {
        }
    }
    pub struct Factory {}
    #[allow(unused_variables)]
    impl ::substrate_service::ServiceFactory for Factory {
        type Block = Block;
        type RuntimeApi = RuntimeApi;
        type NetworkProtocol = NodeProtocol;
        type RuntimeDispatch = Executor;
        type FullTransactionPoolApi = transaction_pool::ChainApi<
            client::Client<FullBackend<Self>, FullExecutor<Self>, Block, RuntimeApi>,
            Block,
        >;
        type LightTransactionPoolApi = transaction_pool::ChainApi<
            client::Client<LightBackend<Self>, LightExecutor<Self>, Block, RuntimeApi>,
            Block,
        >;
        type Genesis = GenesisConfig;
        type Configuration = NodeConfig;
        type FullService = FullComponents<Self>;
        type LightService = LightComponents<Self>;
        type FullImportQueue = AuraImportQueue<Self::Block>;
        type LightImportQueue = AuraImportQueue<Self::Block>;
        type SelectChain = LongestChain<FullBackend<Self>, Self::Block>;
        fn build_full_transaction_pool(
            config: ::substrate_service::TransactionPoolOptions,
            client: ::substrate_service::Arc<::substrate_service::FullClient<Self>>,
        ) -> ::substrate_service::Result<
            ::substrate_service::TransactionPool<Self::FullTransactionPoolApi>,
            ::substrate_service::Error,
        > {
            (|config, client| {
                Ok(TransactionPool::new(
                    config,
                    transaction_pool::ChainApi::new(client),
                ))
            })(config, client)
        }
        fn build_light_transaction_pool(
            config: ::substrate_service::TransactionPoolOptions,
            client: ::substrate_service::Arc<::substrate_service::LightClient<Self>>,
        ) -> ::substrate_service::Result<
            ::substrate_service::TransactionPool<Self::LightTransactionPoolApi>,
            ::substrate_service::Error,
        > {
            (|config, client| {
                Ok(TransactionPool::new(
                    config,
                    transaction_pool::ChainApi::new(client),
                ))
            })(config, client)
        }
        fn build_network_protocol(
            config: &::substrate_service::FactoryFullConfiguration<Self>,
        ) -> ::substrate_service::Result<Self::NetworkProtocol, ::substrate_service::Error>
        {
            (|config| Ok(NodeProtocol::new()))(config)
        }
        fn build_select_chain(
            config: &mut ::substrate_service::FactoryFullConfiguration<Self>,
            client: Arc<::substrate_service::FullClient<Self>>,
        ) -> ::substrate_service::Result<Self::SelectChain, ::substrate_service::Error> {
            (|config: &FactoryFullConfiguration<Self>, client: Arc<FullClient<Self>>| {
                #[allow(deprecated)]
                Ok(LongestChain::new(client.backend().clone()))
            })(config, client)
        }
        fn build_full_import_queue(
            config: &mut ::substrate_service::FactoryFullConfiguration<Self>,
            client: ::substrate_service::Arc<::substrate_service::FullClient<Self>>,
            select_chain: Self::SelectChain,
        ) -> ::substrate_service::Result<Self::FullImportQueue, ::substrate_service::Error>
        {
            (|config: &mut FactoryFullConfiguration<Self>,
              client: Arc<FullClient<Self>>,
              _select_chain: Self::SelectChain| {
                import_queue::<_, _, Pair>(
                    SlotDuration::get_or_compute(&*client)?,
                    client.clone(),
                    None,
                    None,
                    None,
                    client,
                    config.custom.inherent_data_providers.clone(),
                )
                .map_err(Into::into)
            })(config, client, select_chain)
        }
        fn build_light_import_queue(
            config: &mut FactoryFullConfiguration<Self>,
            client: Arc<::substrate_service::LightClient<Self>>,
        ) -> Result<Self::LightImportQueue, ::substrate_service::Error> {
            (|config: &mut FactoryFullConfiguration<Self>, client: Arc<LightClient<Self>>| {
                import_queue::<_, _, Pair>(
                    SlotDuration::get_or_compute(&*client)?,
                    client.clone(),
                    None,
                    None,
                    None,
                    client,
                    config.custom.inherent_data_providers.clone(),
                )
                .map_err(Into::into)
            })(config, client)
        }
        fn build_finality_proof_provider(
            client: Arc<::substrate_service::FullClient<Self>>,
        ) -> Result<
            Option<Arc<::substrate_service::FinalityProofProvider<Self::Block>>>,
            ::substrate_service::Error,
        > {
            (|_client: Arc<FullClient<Self>>| Ok(None))(client)
        }
        fn new_light(
            config: ::substrate_service::FactoryFullConfiguration<Self>,
        ) -> ::substrate_service::Result<Self::LightService, ::substrate_service::Error> {
            (|config| <LightComponents<Factory>>::new(config))(config)
        }
        fn new_full(
            config: ::substrate_service::FactoryFullConfiguration<Self>,
        ) -> Result<Self::FullService, ::substrate_service::Error> {
            (|config: FactoryFullConfiguration<Self>| FullComponents::<Factory>::new(config))(
                config,
            )
            .and_then(|service| {
                (|service: Self::FullService| {
                    if let Some(key) = service.authority_key::<Pair>() {
                        {
                            let lvl = ::log::Level::Info;
                            if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                                ::log::__private_api_log(
                                    ::std::fmt::Arguments::new_v1(
                                        &["Using authority key "],
                                        &match (&key.public(),) {
                                            (arg0,) => [::std::fmt::ArgumentV1::new(
                                                arg0,
                                                ::std::fmt::Display::fmt,
                                            )],
                                        },
                                    ),
                                    lvl,
                                    &(
                                        "node_template::service",
                                        "node_template::service",
                                        "node-template/src/service.rs",
                                        70u32,
                                    ),
                                );
                            }
                        };
                        let proposer = Arc::new(ProposerFactory {
                            client: service.client(),
                            transaction_pool: service.transaction_pool(),
                        });
                        let client = service.client();
                        let select_chain = service
                            .select_chain()
                            .ok_or_else(|| ServiceError::SelectChainRequired)?;
                        let aura = start_aura(
                            SlotDuration::get_or_compute(&*client)?,
                            Arc::new(key),
                            client.clone(),
                            select_chain,
                            client,
                            proposer,
                            service.network(),
                            service.config.custom.inherent_data_providers.clone(),
                            service.config.force_authoring,
                        )?;
                        service
                            .spawn_task(Box::new(aura.select(service.on_exit()).then(|_| Ok(()))));
                    }
                    Ok(service)
                })(service)
            })
        }
    }
}
mod cli {
    use crate::chain_spec;
    use crate::service;
    use futures::{future, sync::oneshot, Future};
    use log::info;
    use std::cell::RefCell;
    use std::ops::Deref;
    pub use substrate_cli::{error, IntoExit, VersionInfo};
    use substrate_cli::{informant, parse_and_execute, NoCustom};
    use substrate_service::{Roles as ServiceRoles, ServiceFactory};
    use tokio::runtime::Runtime;
    #[doc = " Parse command line arguments into service configuration."]
    pub fn run<I, T, E>(args: I, exit: E, version: VersionInfo) -> error::Result<()>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
        E: IntoExit,
    {
        parse_and_execute::<service::Factory, NoCustom, NoCustom, _, _, _, _, _>(
            load_spec,
            &version,
            "substrate-node",
            args,
            exit,
            |exit, _cli_args, _custom_args, config| {
                {
                    let lvl = ::log::Level::Info;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api_log(
                            ::std::fmt::Arguments::new_v1(
                                &[""],
                                &match (&version.name,) {
                                    (arg0,) => [::std::fmt::ArgumentV1::new(
                                        arg0,
                                        ::std::fmt::Display::fmt,
                                    )],
                                },
                            ),
                            lvl,
                            &(
                                "node_template::cli",
                                "node_template::cli",
                                "node-template/src/cli.rs",
                                21u32,
                            ),
                        );
                    }
                };
                {
                    let lvl = ::log::Level::Info;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api_log(
                            ::std::fmt::Arguments::new_v1(
                                &["  version "],
                                &match (&config.full_version(),) {
                                    (arg0,) => [::std::fmt::ArgumentV1::new(
                                        arg0,
                                        ::std::fmt::Display::fmt,
                                    )],
                                },
                            ),
                            lvl,
                            &(
                                "node_template::cli",
                                "node_template::cli",
                                "node-template/src/cli.rs",
                                22u32,
                            ),
                        );
                    }
                };
                {
                    let lvl = ::log::Level::Info;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api_log(
                            ::std::fmt::Arguments::new_v1(
                                &["  by ", ", 2017, 2018"],
                                &match (&version.author,) {
                                    (arg0,) => [::std::fmt::ArgumentV1::new(
                                        arg0,
                                        ::std::fmt::Display::fmt,
                                    )],
                                },
                            ),
                            lvl,
                            &(
                                "node_template::cli",
                                "node_template::cli",
                                "node-template/src/cli.rs",
                                23u32,
                            ),
                        );
                    }
                };
                {
                    let lvl = ::log::Level::Info;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api_log(
                            ::std::fmt::Arguments::new_v1(
                                &["Chain specification: "],
                                &match (&config.chain_spec.name(),) {
                                    (arg0,) => [::std::fmt::ArgumentV1::new(
                                        arg0,
                                        ::std::fmt::Display::fmt,
                                    )],
                                },
                            ),
                            lvl,
                            &(
                                "node_template::cli",
                                "node_template::cli",
                                "node-template/src/cli.rs",
                                24u32,
                            ),
                        );
                    }
                };
                {
                    let lvl = ::log::Level::Info;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api_log(
                            ::std::fmt::Arguments::new_v1(
                                &["Node name: "],
                                &match (&config.name,) {
                                    (arg0,) => [::std::fmt::ArgumentV1::new(
                                        arg0,
                                        ::std::fmt::Display::fmt,
                                    )],
                                },
                            ),
                            lvl,
                            &(
                                "node_template::cli",
                                "node_template::cli",
                                "node-template/src/cli.rs",
                                25u32,
                            ),
                        );
                    }
                };
                {
                    let lvl = ::log::Level::Info;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api_log(
                            ::std::fmt::Arguments::new_v1(
                                &["Roles: "],
                                &match (&config.roles,) {
                                    (arg0,) => {
                                        [::std::fmt::ArgumentV1::new(arg0, ::std::fmt::Debug::fmt)]
                                    }
                                },
                            ),
                            lvl,
                            &(
                                "node_template::cli",
                                "node_template::cli",
                                "node-template/src/cli.rs",
                                26u32,
                            ),
                        );
                    }
                };
                let runtime = Runtime::new().map_err(|e| {
                    ::alloc::fmt::format(::std::fmt::Arguments::new_v1(
                        &[""],
                        &match (&e,) {
                            (arg0,) => [::std::fmt::ArgumentV1::new(arg0, ::std::fmt::Debug::fmt)],
                        },
                    ))
                })?;
                match config.roles {
                    ServiceRoles::LIGHT => run_until_exit(
                        runtime,
                        service::Factory::new_light(config).map_err(|e| {
                            ::alloc::fmt::format(::std::fmt::Arguments::new_v1(
                                &[""],
                                &match (&e,) {
                                    (arg0,) => {
                                        [::std::fmt::ArgumentV1::new(arg0, ::std::fmt::Debug::fmt)]
                                    }
                                },
                            ))
                        })?,
                        exit,
                    ),
                    _ => run_until_exit(
                        runtime,
                        service::Factory::new_full(config).map_err(|e| {
                            ::alloc::fmt::format(::std::fmt::Arguments::new_v1(
                                &[""],
                                &match (&e,) {
                                    (arg0,) => {
                                        [::std::fmt::ArgumentV1::new(arg0, ::std::fmt::Debug::fmt)]
                                    }
                                },
                            ))
                        })?,
                        exit,
                    ),
                }
                .map_err(|e| {
                    ::alloc::fmt::format(::std::fmt::Arguments::new_v1(
                        &[""],
                        &match (&e,) {
                            (arg0,) => [::std::fmt::ArgumentV1::new(arg0, ::std::fmt::Debug::fmt)],
                        },
                    ))
                })
            },
        )
        .map_err(Into::into)
        .map(|_| ())
    }
    fn load_spec(id: &str) -> Result<Option<chain_spec::ChainSpec>, String> {
        Ok(match chain_spec::Alternative::from(id) {
            Some(spec) => Some(spec.load()?),
            None => None,
        })
    }
    fn run_until_exit<T, C, E>(mut runtime: Runtime, service: T, e: E) -> error::Result<()>
    where
        T: Deref<Target = substrate_service::Service<C>>
            + Future<Item = (), Error = ()>
            + Send
            + 'static,
        C: substrate_service::Components,
        E: IntoExit,
    {
        let (exit_send, exit) = exit_future::signal();
        let informant = informant::build(&service);
        runtime.executor().spawn(exit.until(informant).map(|_| ()));
        let _telemetry = service.telemetry();
        let _ = runtime.block_on(service.select(e.into_exit()));
        exit_send.fire();
        Ok(())
    }
    pub struct Exit;
    impl IntoExit for Exit {
        type Exit = future::MapErr<oneshot::Receiver<()>, fn(oneshot::Canceled) -> ()>;
        fn into_exit(self) -> Self::Exit {
            let (exit_send, exit) = oneshot::channel();
            let exit_send_cell = RefCell::new(Some(exit_send));
            ctrlc::set_handler(move || {
                if let Some(exit_send) = exit_send_cell
                    .try_borrow_mut()
                    .expect("signal handler not reentrant; qed")
                    .take()
                {
                    exit_send.send(()).expect("Error sending exit notification");
                }
            })
            .expect("Error setting Ctrl-C handler");
            exit.map_err(drop)
        }
    }
}
pub use substrate_cli::{error, IntoExit, VersionInfo};
fn main() {
    let version = VersionInfo {
        name: "Substrate Node",
        commit: "d5dc301e",
        version: "2.0.0",
        executable_name: "node-template",
        author: "Anonymous",
        description: "Template Node",
        support_url: "support.anonymous.an",
    };
    if let Err(e) = cli::run(::std::env::args(), cli::Exit, version) {
        {
            ::std::io::_eprint(::std::fmt::Arguments::new_v1(
                &["Error starting the node: ", "\n\n", "\n"],
                &match (&e, &e) {
                    (arg0, arg1) => [
                        ::std::fmt::ArgumentV1::new(arg0, ::std::fmt::Display::fmt),
                        ::std::fmt::ArgumentV1::new(arg1, ::std::fmt::Debug::fmt),
                    ],
                },
            ));
        };
        std::process::exit(1)
    }
}
