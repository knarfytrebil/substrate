use crate::Drone;
use std::fs;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use futures::channel::mpsc;
use futures::{FutureExt, StreamExt};
use ln_manager::bitcoin::network::constants;
use ln_manager::executor::Larva;
use ln_manager::ln_bridge::rpc_client::RPCClient;
use ln_manager::ln_bridge::connection::SocketDescriptor;
use ln_manager::ln_bridge::event_handler::{self, Handler, handle_event};
use ln_manager::lightning::chain;
use ln_manager::lightning::ln::{
  peer_handler, channelmonitor,
  channelmanager::{PaymentHash, PaymentPreimage, ChannelManager}
};
use ln_manager::lightning::util::events::{Event, EventsProvider};
use ln_manager::lightning::util::ser::Writeable;

pub fn get_event_notify(
  network: constants::Network,
  file_prefix: String,
  rpc_client: Arc<RPCClient>,
  peer_manager: Arc<peer_handler::PeerManager<SocketDescriptor<Drone>>>,
  monitor: Arc<channelmonitor::SimpleManyChannelMonitor<chain::transaction::OutPoint>>,
  channel_manager: Arc<ChannelManager>,
  broadcaster: Arc<dyn chain::chaininterface::BroadcasterInterface>,
  payment_preimages: Arc<Mutex<HashMap<PaymentHash, PaymentPreimage>>>,
  larva: Drone,
) -> mpsc::Sender<()> {
  let (sender, receiver) = mpsc::channel(2);
  let handler = Arc::new(Handler::new(
    network, file_prefix, rpc_client,
    peer_manager, channel_manager,
    monitor, broadcaster, payment_preimages,
    sender.clone(),
  ));

  let _ = larva.clone().spawn_task(async move {
    receiver.for_each(|_| async {
      handler.peer_manager().process_events();
      let mut events = handler.channel_manager().get_and_clear_pending_events();
      events.append(&mut handler.monitor().get_and_clear_pending_events());
      for event in events {
        larva.mine_event(&event);
        handle_event(event, handler.clone(), larva.clone()).await
      }

      //TODO: functional
      let filename = format!("{}/manager_data", handler.file_prefix());
      let tmp_filename = filename.clone() + ".tmp";

      {
        let mut f = fs::File::create(&tmp_filename).unwrap();
        handler.channel_manager().write(&mut f).unwrap();
      }
      fs::rename(&tmp_filename, &filename).unwrap();
    }).map(|_| Ok(())).await
  });
  sender
}
