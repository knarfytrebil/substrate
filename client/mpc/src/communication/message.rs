use std::str;

use bincode;
use codec::{Decode, Encode, Error as CodecError, Input};
use curv::{
	cryptographic_primitives::{
		proofs::{sigma_correct_homomorphic_elgamal_enc::HomoELGamalProof, sigma_dlog::DLogProof},
		secret_sharing::feldman_vss::VerifiableSS,
	},
	FE,// GE,
};
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::{
	mta::{MessageA, MessageB},
	party_i::{
		KeyGenBroadcastMessage1 as KeyGenCommit, KeyGenDecommitMessage1 as KeyGenDecommit, Phase5ADecom1, Phase5Com1,
		Phase5Com2, Phase5DDecom2, SignBroadcastPhase1, SignDecommitPhase1,
	},
};
use serde::{Deserialize, Serialize};

pub type PeerIndex = u16;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum KeyGenMessage {
	CommitAndDecommit(PeerIndex, KeyGenCommit, KeyGenDecommit),
	VSS(PeerIndex, VerifiableSS),
	SecretShare(PeerIndex, FE),
	Proof(PeerIndex, DLogProof),
}

impl KeyGenMessage {
	pub fn get_index(&self) -> PeerIndex {
		match self {
			Self::CommitAndDecommit(index, _, _) => *index,
			Self::VSS(index, _) => *index,
			Self::SecretShare(index, _) => *index,
			Self::Proof(index, _) => *index,
		}
	}
}

impl PartialEq for KeyGenMessage {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Self::CommitAndDecommit(ia, ca, da), Self::CommitAndDecommit(ib, cb, db)) => {
				ia == ib && ca.com == cb.com && ca.e == cb.e && da.blind_factor == db.blind_factor && da.y_i == db.y_i
			}
			(Self::VSS(ia, vssa), Self::VSS(ib, vssb)) => ia == ib && vssa == vssb,
			(Self::SecretShare(ia, ssa), Self::SecretShare(ib, ssb)) => ia == ib && ssa == ssb,
			(Self::Proof(ia, pa), Self::Proof(ib, pb)) => ia == ib && pa == pb,
			_ => false,
		}
	}
}

impl Encode for KeyGenMessage {
	fn encode(&self) -> Vec<u8> {
		let encoded = bincode::serialize(&self).unwrap();
		Encode::encode(&encoded)
	}
}

impl Decode for KeyGenMessage {
	fn decode<I: Input>(value: &mut I) -> Result<Self, CodecError> {
		let decoded: Vec<u8> = Decode::decode(value)?;
		bincode::deserialize(&decoded).map_err(|_| CodecError::from("bincode error"))
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SigGenMessage {
	Round1(SignBroadcastPhase1, MessageA),
	Round2(MessageB, MessageB), // mb gamma, mb w
	Round3(FE),
	Round4(SignDecommitPhase1),
	Round5(Phase5Com1, Phase5ADecom1, HomoELGamalProof),
	Round6(Phase5Com2, Phase5DDecom2),
	Round7(FE),
}

impl Encode for SigGenMessage {
	fn encode(&self) -> Vec<u8> {
		let encoded = bincode::serialize(&self).unwrap();
		Encode::encode(&encoded)
	}
}

impl Decode for SigGenMessage {
	fn decode<I: Input>(value: &mut I) -> Result<Self, CodecError> {
		let decoded: Vec<u8> = Decode::decode(value)?;
		bincode::deserialize(&decoded).map_err(|_| CodecError::from("bincode error"))
	}
}

impl PartialEq for SigGenMessage {
	fn eq(&self, other: &Self) -> bool {
		self.encode() == other.encode()
	}
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub enum ConfirmPeersMessage {
	Confirming(PeerIndex), // from_index
	Confirmed(String),
}

#[cfg(test)]
mod tests {
	use super::*;

	use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::Keys;

	#[test]
	fn test_message_encode_decode() {
		let key = Keys::create(0);
		let (commit, decommit) = key.phase1_broadcast_phase3_proof_of_correct_key();
		let kgm_commit = KeyGenMessage::CommitAndDecommit(0, commit, decommit);

		let encoded: Vec<u8> = kgm_commit.encode();
		let decoded = KeyGenMessage::decode(&mut encoded.as_slice()).unwrap();
		assert_eq!(kgm_commit, decoded);
	}
}
