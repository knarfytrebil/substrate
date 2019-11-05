// Copyright 2019 Parity Technologies (UK) Ltd.
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

//! Tests for the communication portion of the GRANDPA crate.

use futures::prelude::*;
use futures::sync::mpsc;
use keyring::AuthorityKeyring;
use network::consensus_gossip as network_gossip;
use network::test::{Block, Hash};
use network_gossip::Validator;
use parity_codec::Encode;
use std::sync::Arc;
use tokio::runtime::current_thread;

use super::gossip::{self, GossipValidator};
use super::{AuthorityId, Round, SetId, VoterSet};
use crate::environment::SharedVoterSetState;

enum Event
{
  MessagesFor(
    Hash,
    mpsc::UnboundedSender<network_gossip::TopicNotification>,
  ),
  RegisterValidator(Arc<dyn network_gossip::Validator<Block>>),
  GossipMessage(Hash, Vec<u8>, bool),
  SendMessage(Vec<network::PeerId>, Vec<u8>),
  Report(network::PeerId, i32),
  Announce(Hash),
}

#[derive(Clone)]
struct TestNetwork
{
  sender: mpsc::UnboundedSender<Event>,
}

impl super::Network<Block> for TestNetwork
{
  type In = mpsc::UnboundedReceiver<network_gossip::TopicNotification>;

  /// Get a stream of messages for a specific gossip topic.
  fn messages_for(&self, topic: Hash) -> Self::In
  {
    let (tx, rx) = mpsc::unbounded();
    let _ = self.sender.unbounded_send(Event::MessagesFor(topic, tx));

    rx
  }

  /// Register a gossip validator.
  fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<Block>>)
  {
    let _ = self
      .sender
      .unbounded_send(Event::RegisterValidator(validator));
  }

  /// Gossip a message out to all connected peers.
  ///
  /// Force causes it to be sent to all peers, even if they've seen it already.
  /// Only should be used in case of consensus stall.
  fn gossip_message(&self, topic: Hash, data: Vec<u8>, force: bool)
  {
    let _ = self
      .sender
      .unbounded_send(Event::GossipMessage(topic, data, force));
  }

  /// Send a message to a bunch of specific peers, even if they've seen it already.
  fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>)
  {
    let _ = self.sender.unbounded_send(Event::SendMessage(who, data));
  }

  /// Register a message with the gossip service, it isn't broadcast right
  /// away to any peers, but may be sent to new peers joining or when asked to
  /// broadcast the topic. Useful to register previous messages on node
  /// startup.
  fn register_gossip_message(&self, _topic: Hash, _data: Vec<u8>)
  {
    // NOTE: only required to restore previous state on startup
    //       not required for tests currently
  }

  /// Report a peer's cost or benefit after some action.
  fn report(&self, who: network::PeerId, cost_benefit: i32)
  {
    let _ = self.sender.unbounded_send(Event::Report(who, cost_benefit));
  }

  /// Inform peers that a block with given hash should be downloaded.
  fn announce(&self, block: Hash)
  {
    let _ = self.sender.unbounded_send(Event::Announce(block));
  }
}

impl network_gossip::ValidatorContext<Block> for TestNetwork
{
  fn broadcast_topic(&mut self, _: Hash, _: bool) {}

  fn broadcast_message(&mut self, _: Hash, _: Vec<u8>, _: bool) {}

  fn send_message(&mut self, who: &network::PeerId, data: Vec<u8>)
  {
    <Self as super::Network<Block>>::send_message(self, vec![who.clone()], data);
  }

  fn send_topic(&mut self, _: &network::PeerId, _: Hash, _: bool) {}
}

struct Tester
{
  net_handle: super::NetworkBridge<Block, TestNetwork>,
  gossip_validator: Arc<GossipValidator<Block>>,
  events: mpsc::UnboundedReceiver<Event>,
}

impl Tester
{
  fn filter_network_events<F>(self, mut pred: F) -> impl Future<Item = Self, Error = ()>
  where
    F: FnMut(Event) -> bool,
  {
    let mut s = Some(self);
    futures::future::poll_fn(move || {
      loop
      {
        match s.as_mut().unwrap().events.poll().expect("concluded early")
        {
          Async::Ready(None) => panic!("concluded early"),
          Async::Ready(Some(item)) =>
          {
            if pred(item)
            {
              return Ok(Async::Ready(s.take().unwrap()));
            }
          }
          Async::NotReady => return Ok(Async::NotReady),
        }
      }
    })
  }
}

// some random config (not really needed)
fn config() -> crate::Config
{
  crate::Config {
    gossip_duration: std::time::Duration::from_millis(10),
    justification_period: 256,
    local_key: None,
    name: None,
  }
}

// dummy voter set state
fn voter_set_state() -> SharedVoterSetState<Block>
{
  use crate::authorities::AuthoritySet;
  use crate::environment::{CompletedRound, CompletedRounds, HasVoted, VoterSetState};
  use grandpa::round::State as RoundState;
  use substrate_primitives::H256;

  let state = RoundState::genesis((H256::zero(), 0));
  let base = state.prevote_ghost.unwrap();
  let voters = AuthoritySet::genesis(Vec::new());
  let set_state = VoterSetState::Live {
    completed_rounds: CompletedRounds::new(
      CompletedRound {
        state,
        number: 0,
        votes: Vec::new(),
        base,
      },
      0,
      &voters,
    ),
    current_round: HasVoted::No,
  };

  set_state.into()
}

// needs to run in a tokio runtime.
fn make_test_network() -> (impl Future<Item = Tester, Error = ()>, TestNetwork)
{
  let (tx, rx) = mpsc::unbounded();
  let net = TestNetwork { sender: tx };

  #[derive(Clone)]
  struct Exit;

  impl Future for Exit
  {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()>
    {
      Ok(Async::NotReady)
    }
  }

  let (bridge, startup_work) =
    super::NetworkBridge::new(net.clone(), config(), voter_set_state(), Exit);

  (
    startup_work.map(move |()| Tester {
      gossip_validator: bridge.validator.clone(),
      net_handle: bridge,
      events: rx,
    }),
    net,
  )
}

fn make_ids(keys: &[AuthorityKeyring]) -> Vec<(AuthorityId, u64)>
{
  keys
    .iter()
    .map(|key| AuthorityId(key.to_raw_public()))
    .map(|id| (id, 1))
    .collect()
}

struct NoopContext;

impl network_gossip::ValidatorContext<Block> for NoopContext
{
  fn broadcast_topic(&mut self, _: Hash, _: bool) {}
  fn broadcast_message(&mut self, _: Hash, _: Vec<u8>, _: bool) {}
  fn send_message(&mut self, _: &network::PeerId, _: Vec<u8>) {}
  fn send_topic(&mut self, _: &network::PeerId, _: Hash, _: bool) {}
}
