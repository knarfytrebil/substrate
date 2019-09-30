extern crate futures;
extern crate futures01;
extern crate exit_future;
// extern crate ln_primitives;
// extern crate sr_primitives;
// extern crate substrate_service;
extern crate ln_manager;
extern crate client;

pub use ln_manager::LnManager;

use std::mem;
use std::sync::Arc;
use std::marker::PhantomData;

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
use client::runtime_api::HeaderT;

use ln_manager::ln_bridge::settings::Settings;
use ln_manager::executor::Larva;
use ln_manager::ln_cmd::channel::ChannelC;
use ln_manager::ln_cmd::invoice::InvoiceC;
use ln_manager::ln_cmd::peer::PeerC;

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

impl LnBridge {
  pub fn new(exit: Exit) -> Self {
    let settings = Settings::new(&String::from("./Settings.toml")).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let executor = runtime.executor();
    let drone = Drone::new(executor);
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
    let ln = client.import_notification_stream()
      .map(|v| Ok::<_, ()>(v)).compat()
      .for_each(move |notification| {
        let number = cli.info().best_number;
        // let addr = "03bd08935ad23d0439e8ac9a6b87fff354b6cb4bed51c663471c994e1ab61adbb6@127.0.0.1:19011";
        // // let addr = "02ea3e4997fdc1b63174b4f6ccffd34f80ef4f183b4e9253655b2d79c969d5def5@127.0.0.1:19001";
        // ln_manager.connect(addr.to_string());
        Ok(())
      }).select(self.exit.clone()).then(|_| Ok(()));
    Box::new(ln)
  }
}
