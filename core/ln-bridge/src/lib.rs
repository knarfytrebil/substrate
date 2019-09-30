extern crate futures;
extern crate exit_future;
// extern crate ln_primitives;
// extern crate sr_primitives;
// extern crate substrate_service;
extern crate ln_manager;
// extern crate client;

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

use ln_manager::ln_bridge::settings::Settings;
use ln_manager::executor::Larva;

// use sr_primitives::traits::{self, ProvideRuntimeApi};
// use sr_primitives::generic::BlockId;
// use client::blockchain::HeaderBackend;

// pub use ln_primitives::LnApi;
pub type Executor = tokio::runtime::TaskExecutor;

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
    }
  }
  pub fn ln_manager(&self) -> Arc<LnManager<Drone>> {
    self.ln_manager.clone()
  }

  pub fn new_block(&self) {
    println!("get new block");
  }
}
