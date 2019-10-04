extern crate futures;
extern crate futures01;
extern crate exit_future;
// extern crate ln_primitives;
// extern crate sr_primitives;
// extern crate substrate_service;
extern crate ln_manager;
extern crate client;

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

use client::runtime_api::HeaderT;
use client::{runtime_api::BlockT, BlockchainEvents, ImportNotifications};
use client::blockchain::HeaderBackend;
use client::runtime_api::BlockId::Number;
// use sr_primitives::traits::{self, ProvideRuntimeApi};
// use sr_primitives::generic::BlockId;
// use client::blockchain::HeaderBackend;

// pub use ln_primitives::LnApi;
pub type Executor = tokio::runtime::TaskExecutor;
pub type Task = Box<dyn Future01<Item = (), Error = ()> + Send>;

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

// impl LnBuilder for LnManager<Drone> {}

impl LnBridge {
  pub fn new(exit: Exit) -> Self {
    let settings = Settings::new(&String::from("./Settings.toml")).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let executor = runtime.executor();
      let drone = Drone::new(executor);
      println!("new lnmanager");
    let ln_manager = runtime.block_on(LnManager::new(settings, drone)).unwrap();
    let ln_manager = Arc::new(ln_manager);

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

  pub fn bind_client<B, C>(&self, client: Arc<C>) -> Task
    where B: BlockT, C: BlockchainEvents<B> + HeaderBackend<B> + 'static {
    let cli = client.clone();
    let ln_manager = self.ln_manager();
    self.runtime.spawn(client.import_notification_stream().for_each(move |n| {
      let num = 7;
      let number = cli.info().best_number;
      if (number == num.into()) {
        // let addr = "03bd08935ad23d0439e8ac9a6b87fff354b6cb4bed51c663471c994e1ab61adbb6@127.0.0.1:19011";
        // let addr = "039b6205940b3e1f3563aa4d2b459439ce5ed5b160838c4a53f82cb1b4ab2c05c6@127.0.0.1:19001";
        let addr = "028a49956934ad2ea0428aa661f52241a6853de4cda9ec75f2cd9869c646aa7614@127.0.0.1:9736";
        // // let addr = "02ea3e4997fdc1b63174b4f6ccffd34f80ef4f183b4e9253655b2d79c969d5def5@127.0.0.1:19001";
        ln_manager.connect(addr.to_string());
        println!(">>>>>>>>>>> peer in 3 list: {}", ln_manager.list().len());
      }
      let checknum = 15;
      if (number == checknum.into()) {
        println!(">>>>>>>>>>> check peer in 3 list: {}", ln_manager.list().len());
      }
      futures::future::ready(())
    }));
    let ln = client.import_notification_stream()
      .map(|v| Ok::<_, ()>(v)).compat()
      .for_each(move |notification| {
        // let num = 7;
        // let number = cli.info().best_number;
        // if (number == num.into()) {
        //   let addr = "03bd08935ad23d0439e8ac9a6b87fff354b6cb4bed51c663471c994e1ab61adbb6@127.0.0.1:19011";
        //   // // let addr = "02ea3e4997fdc1b63174b4f6ccffd34f80ef4f183b4e9253655b2d79c969d5def5@127.0.0.1:19001";
        //   ln_manager.connect(addr.to_string());
        //   println!(">>>>>>>>>>> peer list: {}", ln_manager.list().len());
        // }
        // let checknum = 15;
        // if (number == checknum.into()) {
        //   println!(">>>>>>>>>>> check peer list: {}", ln_manager.list().len());
        // }
        Ok(())
      }).select(self.exit.clone()).then(|_| Ok(()));
    Box::new(ln)
  }
}
