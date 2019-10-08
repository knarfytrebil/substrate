use std::{collections::VecDeque, marker::PhantomData, str::FromStr, sync::Arc};

use codec::{Decode, Encode};
use curv::GE;
use futures03::{prelude::*, channel::mpsc};
use futures03::Poll;
use futures03::core_reexport::pin::Pin;

use log::{debug, error, info, warn};
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::{Keys, Parameters};
use rand::prelude::Rng;
use tokio_executor::DefaultExecutor;
use tokio_timer::Interval;
use client::backend::OffchainStorage;
use crate::communication::MWSStream;
use crate::communication::MWSSink;


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

struct Buffered<A,S>
where S: futures03::sink::Sink<A, Error = Error>+Unpin
 {
	inner:S,
	buffer: VecDeque<A>,
}

impl<A,S>  Unpin for  Buffered<A,S>
where S: futures03::sink::Sink<A, Error = Error>+Unpin
{}


impl<SinkItem,S> Buffered<SinkItem,S> 
where S: futures03::sink::Sink<SinkItem, Error = Error>+Unpin,
{
	fn new(inner: S) -> Buffered<SinkItem,S>
	 {
		Buffered {
			buffer: VecDeque::new(),
			inner,
		}
	}
    fn is_empty(&self) -> bool {
		self.buffer.is_empty()
	}
	// push an item into the buffered sink.
	// the sink _must_ be driven to completion with `poll` afterwards.
	fn push(&mut self, item: SinkItem) {
		self.buffer.push_back(item);
	}
	// returns ready when the sink and the buffer are completely flushed.
	fn poll(mut self: Pin<&mut Self>,cx: &mut std::task::Context<'_>) -> Poll<()> {
		let polled =  (*self).schedule_all(cx);

		let respoll=match polled {
			futures03::Poll::Ready(()) => Pin::new((&mut (*self).inner)).poll_flush(cx),
			futures03::Poll::Pending => {
				Pin::new((&mut (*self).inner)).poll_flush(cx);
				futures03::Poll::Pending 
			}
		};
		match respoll
		{
        Poll::Pending => Poll::Pending,
		Poll::Ready(Ok(_)) =>Poll::Ready(()),
		Poll::Ready(Err(_)) =>Poll::Pending,
		}
	}

	fn schedule_all(&mut self,cx: &mut std::task::Context<'_>) -> Poll<()> {
		match Pin::new(&mut self.inner).poll_ready(cx)
		{
			Poll::Pending => return Poll::Pending,
			Poll::Ready(Ok( () )) => {},
			Poll::Ready(Err(_)) => return Poll::Pending,
		}
		while let Some(front) = self.buffer.pop_front() {
              info!("Some in FRONT {:?}",self.buffer.len());
			match Pin::new(&mut self.inner).start_send(front) {
				Ok(()) => {
             match Pin::new(&mut self.inner).poll_ready(cx)
		 {
			Poll::Pending => return Poll::Pending,
			Poll::Ready(Ok( () )) => continue,
			Poll::Ready(Err(_)) => return Poll::Pending, //todo::convert
		 }

				},
				Err(_)=> {
				//	self.buffer.push_front(front);
					break;
				}
			}
			
		}

		if self.buffer.is_empty() {
			Poll::Ready(())
		} else {
			Poll::Pending
		}
	}
}



pub(crate) struct Signer<B, E, Block: BlockT, N: Network<Block>, RA, In, Out,Storage>
where
	In: Stream<Item = MessageWithSender>,
	Out: Sink<MessageWithSender, Error = Error>+Unpin,
{
	env: Arc<Environment<B, E, Block, N, RA,Storage>>,
	global_in: In,
	global_out: Buffered<MessageWithSender,Out>,
	should_rebuild: bool,
	last_message_ok: bool,
}



impl<B, E, Block, N, RA, In, Out,Storage> Signer<B, E, Block, N, RA, In, Out,Storage>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + 'static,
	Block: BlockT<Hash = H256>,
	Block::Hash: Ord,
	N: Network<Block> + Sync,
	N::In: Send + 'static,
	RA: Send + Sync + 'static,
	In: MWSStream,
	Out: MWSSink,
	Storage:OffchainStorage,
{
	pub fn new(
		env: Arc<Environment<B, E, Block, N, RA,Storage>>,
		global_in: In,
		global_out: Out,
		last_message_ok: bool,
	) -> Self {
		Self {
			env,
			global_in,
			global_out: Buffered::new(global_out),
			should_rebuild: false,
			last_message_ok,
		}
	}

	fn generate_shared_keys(&mut self) {
		let players = self.env.config.players as usize;
		let mut state = self.env.state.write();
		if state.complete
			|| state.shared_keys.is_some()
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

		let key = state.local_key.clone().unwrap();

		let vsss = state.vsss.values().cloned().collect::<Vec<_>>();
		let secret_shares = state.secret_shares.values().cloned().collect::<Vec<_>>();
		let points = state.decommits.values().map(|x| x.y_i).collect::<Vec<_>>();

		let res = key.phase2_verify_vss_construct_keypair_phase3_pok_dlog(
			&params,
			points.as_slice(),
			secret_shares.as_slice(),
			vsss.as_slice(),
			key.party_index + 1,
		);

		if res.is_err() {
			println!("{:?} \n {:?} \n {:?}", secret_shares, vsss, res);
			panic!("ss error of {:?}", key.party_index);
			return;
		}

		let (shared_keys, proof) = res.unwrap();

		let i = key.party_index as PeerIndex;
		state.proofs.insert(i, proof.clone());
		state.shared_keys = Some(shared_keys);

		drop(state);

		println!("{:?} CREATE PROOF OK", key.party_index);

		let proof_msg = KeyGenMessage::Proof(i, proof);
		let validator = self.env.bridge.validator.inner.read();
		let hash = validator.get_peers_hash();
		self.global_out
			.push((GossipMessage::KeyGen(proof_msg, hash), None));
	}

	fn handle_cpm(
		&mut self,
		cpm: ConfirmPeersMessage,
		sender: PeerId,
		all_peers_hash: u64,
	) -> bool {
		let players = self.env.config.players;

		match cpm {
			ConfirmPeersMessage::Confirming(from_index) => {
				println!("recv confirming msg");
				// let sender = sender.clone().unwrap();

				let validator = self.env.bridge.validator.inner.read();
				let receiver = validator.get_peer_id_by_index(from_index as usize);
				if receiver.is_none() {
					return false;
				}
				// let index = validator.get_peer_index(&sender) as PeerIndex;
				// let peer_len = validator.peers.len();
				// println!("{:?}  {:?} {:?}", *all_peers_hash, our_hash, peer_len);

				// if *from_index != index || *all_peers_hash != our_hash {
				// 	return;
				// }
				self.global_out.push((
					GossipMessage::ConfirmPeers(
						ConfirmPeersMessage::Confirmed(validator.local_string_peer_id()),
						all_peers_hash,
					),
					receiver,
				));
			}
			ConfirmPeersMessage::Confirmed(sender_string_id) => {
				println!("recv confirmed msg");

				println!("sender id {:?} {:?}", sender.to_base58(), sender_string_id);
				if sender.to_base58() != sender_string_id {
					return false;
				}

				// let sender = PeerId::from_str(&sender_string_id);
				// if sender.is_err() {
				// 	return false;
				// }

				{
					let state = self.env.state.read();
					if state.local_key.is_some() {
						return true;
					}
				}

				let mut validator = self.env.bridge.validator.inner.write();
				validator.set_peer_generating(&sender);

				if validator.get_peers_len() == players as usize {
					let local_index = validator.get_local_index();

					let key = Keys::create(local_index);
					let (commit, decommit) = key.phase1_broadcast_phase3_proof_of_correct_key();
					let index = local_index as PeerIndex;

					{
						let mut state = self.env.state.write();
						state.commits.insert(index, commit.clone());
						state.decommits.insert(index, decommit.clone());
						state.local_key = Some(key);
						validator.set_local_generating();
						println!("LOCAL KEY GEN OF {:?}", index);
						drop(validator);
					}

					let cad_msg = KeyGenMessage::CommitAndDecommit(index, commit, decommit);
					self.global_out.push(
						// broadcast
						(GossipMessage::KeyGen(cad_msg, all_peers_hash), None),
					);
				}
			}
		}
		true
	}

	fn handle_kgm(&mut self, kgm: KeyGenMessage, all_peers_hash: u64) -> bool {
		let players = self.env.config.players;

		match kgm {
			KeyGenMessage::CommitAndDecommit(from_index, commit, decommit) => {
				println!("CAD MSG from {:?}", from_index);
				let mut state = self.env.state.write();
				if state.local_key.is_none() {
					return false;
				}

				if state.commits.contains_key(&from_index) {
					return true;
				}

				state.commits.insert(from_index, commit);
				state.decommits.insert(from_index, decommit);

				let key = state.local_key.clone().unwrap();

				let index = key.party_index as PeerIndex;
				if state.vsss.contains_key(&index) {
					// we already created vss
					assert!(state.secret_shares.contains_key(&index));
					return true;
				}

				if state.commits.len() == players as usize {
					let params = self.env.config.get_params();

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

					println!(
						"vss and share {:?} \n {:?}\n of {:?}",
						vss, secret_shares, index
					);

					state.vsss.insert(index as PeerIndex, vss.clone());
					state.secret_shares.insert(index as PeerIndex, share);

					drop(state);

					self.global_out.push((
						GossipMessage::KeyGen(
							KeyGenMessage::VSS(index as PeerIndex, vss),
							all_peers_hash,
						),
						None,
					));

					let validator = self.env.bridge.validator.inner.read();

					for (i, &ss) in secret_shares.iter().enumerate() {
						if i != index {
							let ss_msg = KeyGenMessage::SecretShare(index as PeerIndex, ss);
							let peer = validator.get_peer_id_by_index(i);
							self.global_out
								.push((GossipMessage::KeyGen(ss_msg, all_peers_hash), peer));
						}
					}
				}
			}
			KeyGenMessage::VSS(from_index, vss) => {
				let mut state = self.env.state.write();
				if state.vsss.contains_key(&from_index) {
					return true;
				}

				state.vsss.insert(from_index, vss.clone());
			}
			KeyGenMessage::SecretShare(from_index, ss) => {
				let mut state = self.env.state.write();
				if state.secret_shares.contains_key(&from_index) {
					return true;
				}

				state.secret_shares.insert(from_index, ss.clone());
			}
			KeyGenMessage::Proof(from_index, proof) => {
				let mut state = self.env.state.write();
				println!("RECV PROOF from {:?}", from_index);

				state.proofs.insert(from_index, proof.clone());

				if state.proofs.len() == players as usize
					&& state.decommits.len() == players as usize
				{
					let params = Parameters {
						threshold: self.env.config.threshold,
						share_count: self.env.config.players,
					};

					let proofs = state.proofs.values().cloned().collect::<Vec<_>>();
					let points = state.decommits.values().map(|x| x.y_i).collect::<Vec<_>>();

					let mut validator = self.env.bridge.validator.inner.write();

					if Keys::verify_dlog_proofs(&params, proofs.as_slice(), points.as_slice())
						.is_ok()
					{
						info!("Key generation complete");
						println!("key gen complete");
						state.complete = true;
						validator.set_local_complete();
					} else {
						// reset everything?
						error!("Key generation failed");
						state.reset();
						validator.set_local_canceled();
					}
				}
			}
		}
		true
	}

	fn handle_incoming(&mut self, msg: GossipMessage, sender: Option<PeerId>) -> bool {
		match msg {
			GossipMessage::ConfirmPeers(cpm, all_peers_hash) => {
				let validator = self.env.bridge.validator.inner.read();
				let our_hash = validator.get_peers_hash();

				println!("cpm msg local state {:?}", validator.local_state());

				if sender.is_none() {
					return false;
				}
				drop(validator);
				return self.handle_cpm(cpm, sender.unwrap(), all_peers_hash);
			}
			GossipMessage::KeyGen(kgm, all_peers_hash) => {
				println!("recv key gen msg");
				let validator = self.env.bridge.validator.inner.read();
				println!("kgm local state {:?}", validator.local_state());

				if validator.is_local_complete() || validator.is_local_canceled() {
					return true;
				}

				drop(validator);
				return self.handle_kgm(kgm, all_peers_hash);
			}
			GossipMessage::Sign(_) => {}
		}

		true
	}
}

impl<B, E, Block, N, RA, In, Out,Storage> Future for Signer<B, E, Block, N, RA, In, Out,Storage>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + Send + Sync + 'static,
	Block: BlockT<Hash = H256>,
	Block::Hash: Ord,
	N: Network<Block> + Sync,
	N::In: Send + 'static,
	RA: Send + Sync + 'static,
	In:MWSStream,
	Out: MWSSink,
	Storage: OffchainStorage,
{
	type Output=Result<(),Error>;

fn poll(mut self: Pin<&mut Self>,cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {

	let mut rebuild_state_changed = false;


		while let Poll::Ready(Some(item)) = Pin::new(&mut self.global_in).poll_next(cx) {
			let (msg, sender) = item;
			
			let is_ok = self.handle_incoming(msg.clone(), sender);
			if !is_ok {
				info!("NOT OK MSG {:?}", msg);
			}

			if !rebuild_state_changed {
				let should_rebuild = self.last_message_ok ^ is_ok;
				// TF or FT makes it rebuild, i.e. "edge triggering"
				self.last_message_ok = is_ok;
				if self.should_rebuild != should_rebuild {
					self.should_rebuild = should_rebuild;
					rebuild_state_changed = true;
				}
			}

		}



	
		self.generate_shared_keys();

			// send messages generated by `handle_incoming`
		 Pin::new(&mut self.global_out).poll(cx);
		

        if self.should_rebuild {
			return  Poll::Ready(Err(Error::Rebuild));
		}
		Poll::Pending
	}

}
