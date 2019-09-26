use std::{collections::VecDeque, marker::PhantomData, sync::Arc};

use codec::{Decode, Encode};
use curv::GE;
use futures::{prelude::*, sync::mpsc};
use log::{debug, error, info, warn};
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::{Keys, Parameters};
use rand::prelude::*;
use tokio_executor::DefaultExecutor;
use tokio_timer::Interval;

use client::{
	backend::Backend, error::Error as ClientError, error::Result as ClientResult, BlockchainEvents,
	CallExecutor, Client,
};
use consensus_common::SelectChain;
use inherents::InherentDataProviders;
use network::PeerId;
use primitives::{Blake2Hasher, H256};
use sr_primitives::generic::BlockId;
use sr_primitives::traits::{Block as BlockT, NumberFor, ProvideRuntimeApi};

use super::{
	ConfirmPeersMessage, Environment, Error, GossipMessage, KeyGenMessage, MessageWithSender,
	Network, PeerIndex, SignMessage,
};

struct Buffered<S: Sink> {
	inner: S,
	buffer: VecDeque<S::SinkItem>,
}

impl<S: Sink> Buffered<S> {
	fn new(inner: S) -> Buffered<S> {
		Buffered {
			buffer: VecDeque::new(),
			inner,
		}
	}

	// push an item into the buffered sink.
	// the sink _must_ be driven to completion with `poll` afterwards.
	fn push(&mut self, item: S::SinkItem) {
		self.buffer.push_back(item);
	}

	// returns ready when the sink and the buffer are completely flushed.
	fn poll(&mut self) -> Poll<(), S::SinkError> {
		let polled = self.schedule_all()?;

		match polled {
			Async::Ready(()) => self.inner.poll_complete(),
			Async::NotReady => {
				self.inner.poll_complete()?;
				Ok(Async::NotReady)
			}
		}
	}

	fn schedule_all(&mut self) -> Poll<(), S::SinkError> {
		while let Some(front) = self.buffer.pop_front() {
			match self.inner.start_send(front) {
				Ok(AsyncSink::Ready) => continue,
				Ok(AsyncSink::NotReady(front)) => {
					self.buffer.push_front(front);
					break;
				}
				Err(e) => return Err(e),
			}
		}

		if self.buffer.is_empty() {
			Ok(Async::Ready(()))
		} else {
			Ok(Async::NotReady)
		}
	}
}

pub(crate) struct Signer<B, E, Block: BlockT, N: Network<Block>, RA, In, Out>
where
	In: Stream<Item = MessageWithSender, Error = Error>,
	Out: Sink<SinkItem = MessageWithSender, SinkError = Error>,
{
	env: Arc<Environment<B, E, Block, N, RA>>,
	global_in: In,
	global_out: Buffered<Out>,
	// message_cache_tx: mpsc::UnboundedSender<MessageWithSender>,
}

impl<B, E, Block, N, RA, In, Out> Signer<B, E, Block, N, RA, In, Out>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + 'static,
	Block: BlockT<Hash = H256>,
	Block::Hash: Ord,
	N: Network<Block> + Sync,
	N::In: Send + 'static,
	RA: Send + Sync + 'static,
	In: Stream<Item = MessageWithSender, Error = Error>,
	Out: Sink<SinkItem = MessageWithSender, SinkError = Error>,
{
	pub fn new(
		env: Arc<Environment<B, E, Block, N, RA>>,
		global_in: In,
		global_out: Out,
		// tx: mpsc::UnboundedSender<MessageWithSender>,
	) -> Self {
		Self {
			env,
			global_in,
			global_out: Buffered::new(global_out),
			// message_cache_tx: tx,
		}
	}

	fn generate_shared_keys(&mut self) {
		let players = self.env.config.players as usize;
		let mut state = self.env.state.write();
		if state.complete
			|| state.vsss.len() != players
			|| state.secret_shares.len() != players
			|| state.decommits.len() != players
		{
			return;
		}

		let params = Parameters {
			threshold: self.env.config.threshold,
			share_count: self.env.config.players,
		};

		let key: Keys = state.local_key.clone().unwrap();

		let vsss = state.vsss.values().cloned().collect::<Vec<_>>();

		let secret_shares = state.secret_shares.values().cloned().collect::<Vec<_>>();

		let points = state.decommits.values().map(|x| x.y_i).collect::<Vec<_>>();

		let (shared_keys, proof) = key
			.phase2_verify_vss_construct_keypair_phase3_pok_dlog(
				&params,
				points.as_slice(),
				secret_shares.as_slice(),
				vsss.as_slice(),
				key.party_index + 1,
			)
			.unwrap();

		let i = key.party_index as PeerIndex;
		state.proofs.insert(i, proof.clone());
		state.complete = true;
		state.shared_keys = Some(shared_keys);

		drop(state);

		let proof_msg = KeyGenMessage::Proof(i, proof);
		let validator = self.env.bridge.validator.inner.read();
		let hash = validator.get_peers_hash();
		self.global_out
			.push((GossipMessage::KeyGen(proof_msg, hash), None));
	}

	fn handle_incoming(&mut self, msg: &GossipMessage, sender: &Option<PeerId>) -> bool {
		let players = self.env.config.players;

		match msg {
			GossipMessage::ConfirmPeers(cpm, all_peers_hash) => {
				let validator = self.env.bridge.validator.inner.read();
				println!("cpm msg local state {:?}", validator.local_state());

				if !validator.is_local_awaiting_peers() {
					return false;
				}
				drop(validator);

				match cpm {
					ConfirmPeersMessage::Confirming(from_index) => {
						println!("recv confirming msg");
						let sender = sender.clone().unwrap();

						let validator = self.env.bridge.validator.inner.read();
						let index = validator.get_peer_index(&sender) as PeerIndex;
						let our_hash = validator.get_peers_hash();
						let peer_len = validator.peers.len();
						println!("{:?}  {:?} {:?}", *all_peers_hash, our_hash, peer_len);

						// if *from_index != index || *all_peers_hash != our_hash {
						// 	return;
						// }
						self.global_out.push((
							GossipMessage::ConfirmPeers(
								ConfirmPeersMessage::Confirmed(validator.local_string_peer_id()),
								our_hash,
							),
							Some(sender),
						));
					}
					ConfirmPeersMessage::Confirmed(sender_string_id) => {
						println!("recv confirmed msg");

						let sender = sender.clone().unwrap();
						println!("sender id {:?} {:?}", sender.to_base58(), *sender_string_id);
						// if sender.to_base58() != *sender_string_id {
						// 	return;
						// }

						let mut validator = self.env.bridge.validator.inner.write();
						validator.set_peer_generating(&sender);

						if validator.get_peers().len() == players as usize - 1 {
							validator.set_local_generating();
							let local_index = validator.get_local_index();
							drop(validator);

							let key = Keys::create(local_index);
							let (commit, decommit) =
								key.phase1_broadcast_phase3_proof_of_correct_key();

							let index = local_index as PeerIndex;

							{
								let mut state = self.env.state.write();
								state.commits.insert(index, commit.clone());
								state.decommits.insert(index, decommit.clone());
								state.local_key = Some(key);
							}

							let cad_msg = KeyGenMessage::CommitAndDecommit(index, commit, decommit);

							self.global_out
								.push((GossipMessage::KeyGen(cad_msg, *all_peers_hash), None));
						}
					}
				}
			}

			GossipMessage::KeyGen(kgm, all_peers_hash) => {
				println!("recv key gen msg");
				let validator = self.env.bridge.validator.inner.read();
				println!("kgm local state {:?}", validator.local_state());
				if !validator.is_local_generating() {
					return false;
				}
				drop(validator);

				match kgm {
					KeyGenMessage::CommitAndDecommit(from_index, commit, decommit) => {
						let mut rng = rand::thread_rng();
						let b = rng.gen_bool(0.5);
						if b {
							return false;
						}
						let mut state = self.env.state.write();

						state.commits.insert(*from_index, commit.clone());
						state.decommits.insert(*from_index, decommit.clone());

						if state.commits.len() == players as usize {
							let params = Parameters {
								threshold: self.env.config.threshold,
								share_count: self.env.config.players,
							};

							let key = state.local_key.clone().unwrap();

							let commits = state.commits.values().cloned().collect::<Vec<_>>();
							let decommits = state.decommits.values().cloned().collect::<Vec<_>>();

							let (vss, secret_shares, index) = key
								.phase1_verify_com_phase3_verify_correct_key_phase2_distribute(
									&params,
									decommits.as_slice(),
									commits.as_slice(),
								)
								.unwrap();

							let share = secret_shares[index].clone();
							state.vsss.insert(index as PeerIndex, vss.clone());
							state.secret_shares.insert(index as PeerIndex, share);

							drop(state);

							self.global_out.push((
								GossipMessage::KeyGen(
									KeyGenMessage::VSS(index as PeerIndex, vss),
									*all_peers_hash,
								),
								None,
							));

							let validator = self.env.bridge.validator.inner.read();

							for (i, &ss) in secret_shares.iter().enumerate() {
								if i != index {
									let ss_msg = KeyGenMessage::SecretShare(index as PeerIndex, ss);
									let peer = validator.get_peer_id_by_index(i).unwrap();
									self.global_out.push((
										GossipMessage::KeyGen(ss_msg, *all_peers_hash),
										Some(peer),
									));
								}
							}
						}
					}
					KeyGenMessage::VSS(from_index, vss) => {
						let mut state = self.env.state.write();

						state.vsss.insert(*from_index, vss.clone());
					}
					KeyGenMessage::SecretShare(from_index, ss) => {
						let mut state = self.env.state.write();

						state.secret_shares.insert(*from_index, ss.clone());
					}
					KeyGenMessage::Proof(from_index, proof) => {
						let mut state = self.env.state.write();

						state.proofs.insert(*from_index, proof.clone());

						if state.proofs.len() == players as usize
							&& state.decommits.len() == players as usize
						{
							let params = Parameters {
								threshold: self.env.config.threshold,
								share_count: self.env.config.players,
							};

							let proofs = state.proofs.values().cloned().collect::<Vec<_>>();
							let points =
								state.decommits.values().map(|x| x.y_i).collect::<Vec<_>>();

							let mut validator = self.env.bridge.validator.inner.write();

							if Keys::verify_dlog_proofs(
								&params,
								proofs.as_slice(),
								points.as_slice(),
							)
							.is_ok()
							{
								info!("Key generation complete");
								println!("key gen complete");
								validator.set_local_complete();
							} else {
								// reset everything?
								error!("Key generation failed");
								validator.set_local_canceled();
							}
						}
					}
				}
			}
			GossipMessage::Sign(_) => {}
			_ => {}
		}

		true
	}
}

impl<B, E, Block, N, RA, In, Out> Future for Signer<B, E, Block, N, RA, In, Out>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + 'static,
	Block: BlockT<Hash = H256>,
	Block::Hash: Ord,
	N: Network<Block> + Sync,
	N::In: Send + 'static,
	RA: Send + Sync + 'static,
	In: Stream<Item = MessageWithSender, Error = Error>,
	Out: Sink<SinkItem = MessageWithSender, SinkError = Error>,
{
	type Item = ();
	type Error = Error;
	fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
		while let Async::Ready(Some(item)) = self.global_in.poll()? {
			let (msg, sender) = item;
			let is_ok = self.handle_incoming(&msg, &sender);
		}

		// send messages generated by `handle_incoming`
		self.global_out.poll()?;
		{
			let state = self.env.state.read();
			println!(
				"state: commits {:?} decommits {:?} vss {:?} ss {:?}  proof {:?} has key {:?} complete {:?}",
				state.commits.len(),
				state.decommits.len(),
				state.vsss.len(),
				state.secret_shares.len(),
				state.proofs.len(),
				state.local_key.is_some(),
				state.complete
			);
		}
		self.generate_shared_keys();

		Ok(Async::NotReady)
	}
}
