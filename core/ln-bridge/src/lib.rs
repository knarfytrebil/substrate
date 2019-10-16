extern crate futures;
extern crate futures01;
extern crate exit_future;
extern crate ln_primitives;
extern crate sr_primitives;
// extern crate substrate_service;
extern crate ln_manager;
extern crate client;
extern crate hex;
extern crate inherents;

mod ln_event;

pub use ln_manager::{LnManager, Builder};

use std::mem;
use std::sync::{Arc, Mutex};
use std::marker::PhantomData;
use std::collections::HashMap;

use futures::future;
use futures::future::Future;
use futures::future::FutureExt;
use futures::channel::mpsc;
// use futures::sync::mpsc;
use exit_future::Exit;
use futures01::future::Future as Future01;
use futures01::Stream;
use futures::{StreamExt, TryStreamExt};
// use futures::stream::Stream;

use ln_manager::ln_bridge::settings::Settings;
use ln_manager::executor::Larva;
use ln_manager::ln_cmd::{
  channel::ChannelC, invoice::InvoiceC, peer::PeerC
};

use ln_manager::bitcoin::network::constants;
use ln_manager::ln_bridge::{
  rpc_client::RPCClient,
  connection::SocketDescriptor
};
use ln_manager::lightning::chain;
use ln_manager::lightning::ln::{
  peer_handler, channelmonitor,
  channelmanager::{PaymentHash, PaymentPreimage, ChannelManager}
};
use ln_manager::lightning::util::events::Event;
use ln_manager::lightning::chain::keysinterface::KeysInterface;

use client::runtime_api::HeaderT;
use client::{runtime_api::BlockT, BlockchainEvents, ImportNotifications};
use client::blockchain::HeaderBackend;
use client::runtime_api::BlockId::Number;
use client::BlockImportNotification;
use sr_primitives::traits::{self, ProvideRuntimeApi, NumberFor};
use sr_primitives::generic::{BlockId, OpaqueDigestItemId};

// use sr_primitives::generic::BlockId;
pub use ln_primitives::{LnApi, ConsensusLog, LN_ENGINE_ID, LnNode};
use inherents::{InherentDataProviders, InherentIdentifier};

pub type Executor = tokio::runtime::TaskExecutor;
pub type Task = Box<dyn Future01<Item = (), Error = ()> + Send>;

pub const INHERENT_LN_ID: InherentIdentifier = *b"ltn_data";

#[derive(Clone)]
pub struct Drone {
  // spawn_task_handle: SpawnTaskHandle,
  executor: Executor,
  // exit: Exit
}
impl Drone {
  // fn new(spawn_task_handle: SpawnTaskHandle, exit: Exit) -> Self {
  //   Self { spawn_task_handle, exit }
  // }
  fn new(executor: Executor) -> Self {
    Self { executor }
  }

  fn mine_event(&self, event: &Event) {
    match event {
      Event::FundingGenerationReady { temporary_channel_id, channel_value_satoshis, output_script, .. } => {

      },
      _ => {
        println!("catch some event");
      }
    }
    println!("inject function");
  }
}

impl Larva for Drone {
  fn spawn_task(
    &self,
    task: impl Future<Output = Result<(), ()>> + Send + 'static,
  ) -> Result<(), futures::task::SpawnError> {
    self.executor.spawn(task.map(|_| ()));
    Ok(())
  }
}

pub struct LnBridge {
  ln_manager: Arc<LnManager<Drone>>,
  exit: Exit,
  runtime: tokio::runtime::Runtime,
}

impl LnBridge {
  pub fn new(exit: Exit) -> Self {
    let settings = Settings::new(&String::from("./Settings.toml")).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let executor = runtime.executor();
    let drone = Drone::new(executor);
    let ln_manager = runtime.block_on(LnManager::new(settings, drone)).unwrap();
    let ln_manager = Arc::new(ln_manager);
    let key = ln_manager.node_key_str();
    println!("ln node: {:?}", key);
    // let keys = &ln_manager.keys;
    // println!("ln node: {:?}", LnManager::<Drone>::node_key_str(*keys));
    // let secret = ln_manager.keys.get_node_secret();
    // println!("ln node: {:?}", hex_str(&PublicKey::from_secret_key(&Secp256k1::new(), &secret).serialize()));

    Self {
      ln_manager,
      exit,
      runtime,
    }
  }
  pub fn ln_manager(&self) -> Arc<LnManager<Drone>> {
    self.ln_manager.clone()
  }
  pub fn executor(&self) -> tokio::runtime::TaskExecutor {
    self.runtime.executor()
  }

  pub fn bind_client<B, C>(&self, client: Arc<C>) -> Task where
    B: BlockT,
  // C: ProvideRuntimeApi + BlockchainEvents<B> + HeaderBackend<B> + 'static,
    C: BlockchainEvents<B> + HeaderBackend<B> + 'static,
  // C::Api: LnApi<B>
  {
    let cli = client.clone();
    let ln_manager = self.ln_manager();
    let bridge = self.clone();
    let ln = client.import_notification_stream()
      .map(|v| Ok::<_, ()>(v)).compat()
      .for_each(move |notification| {
        let res = {
          let header = notification.header;
          let id = OpaqueDigestItemId::Consensus(&LN_ENGINE_ID);
          let filter_log = |log: ConsensusLog| match log {
            // ConsensusLog::FundChannel() => Some(1),
            // ConsensusLog::CloseChannel() => Some(1),
            // ConsensusLog::ForceCloseAllChannel() => Some(1),
            // ConsensusLog::PayInvoice() => Some(1),
            // ConsensusLog::CreateInvoice() => Some(1),
            ConsensusLog::ConnectPeer(node) => {
              let hex_str = hex::encode(node);
              let bytes = hex::decode(hex_str).unwrap();
              let addr = String::from_utf8(bytes).unwrap();
              println!("<<<<<<<<<<<<<<<<<<<<<<<<<< addr {:?}", addr);
              Some(ln_manager.connect(addr))
            },
            _ => None,
          };
          header.digest().convert_first(|l| l.try_to(id).and_then(filter_log))
        };
        if let Some(change) = res {
          println!("<<<<<<<<<<<<<<<<<<<<<<<<<<<<< catch log event here");
        }
        Ok(())
      }).select(self.exit.clone()).then(|_| Ok(()));
    Box::new(ln)
  }

  pub fn bind_runtime<C, Block>(&self, client: Arc<C>) where
    Block: traits::Block,
    C: ProvideRuntimeApi,
    C::Api: LnApi<Block>,
  {
    let runtime_api = client.runtime_api();
  }
}

impl Builder<Drone> for LnManager<Drone> {
  fn get_event_handler(
    network: constants::Network,
    data_path: String,
    rpc_client: Arc<RPCClient>,
    peer_manager: Arc<peer_handler::PeerManager<SocketDescriptor<Drone>>>,
    monitor: Arc<channelmonitor::SimpleManyChannelMonitor<chain::transaction::OutPoint>>,
    channel_manager: Arc<ChannelManager>,
    chain_broadcaster: Arc<dyn chain::chaininterface::BroadcasterInterface>,
    payment_preimages: Arc<Mutex<HashMap<PaymentHash, PaymentPreimage>>>,
    larva: Drone,
  ) -> mpsc::Sender<()> {
    ln_event::get_event_notify(
      network,
      data_path,
      rpc_client,
      peer_manager,
      monitor,
      channel_manager,
      chain_broadcaster,
      payment_preimages,
      larva,
    )
  }
}
