use crate::Drone;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use futures::channel::mpsc;
use ln_manager::bitcoin::network::constants;
use ln_manager::ln_bridge::rpc_client::RPCClient;
use ln_manager::ln_bridge::connection::SocketDescriptor;
use ln_manager::lightning::chain;
use ln_manager::lightning::ln::{
    peer_handler, channelmonitor,
    channelmanager::{PaymentHash, PaymentPreimage, ChannelManager}
};

pub fn get_event_notify(
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
    ln_manager::ln_bridge::event_handler::setup(
        network,
        data_path,
        rpc_client.clone(),
        peer_manager.clone(),
        monitor.clone(),
        channel_manager.clone(),
        chain_broadcaster.clone(),
        payment_preimages.clone(),
        larva.clone(),
    )
}
