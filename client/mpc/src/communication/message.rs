use std::str;

use codec::{Decode, Encode, Error as CodecError, Input};
use curv::cryptographic_primitives::proofs::sigma_dlog::DLogProof;
use curv::cryptographic_primitives::secret_sharing::feldman_vss::VerifiableSS;
use curv::FE;
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::{
	KeyGenBroadcastMessage1 as KeyGenCommit, KeyGenDecommitMessage1 as KeyGenDecommit,
};
use serde::{Deserialize, Serialize};
use serde_json;

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
		match *self {
			Self::CommitAndDecommit(index, _, _) => index,
			Self::VSS(index, _) => index,
			Self::SecretShare(index, _) => index,
			Self::Proof(index, _) => index,
		}
	}
}

impl PartialEq for KeyGenMessage {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Self::CommitAndDecommit(ia, ca, da), Self::CommitAndDecommit(ib, cb, db)) => {
				ia == ib
					&& ca.com == cb.com && ca.e == cb.e
					&& da.blind_factor == db.blind_factor
					&& da.y_i == db.y_i
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
		let encoded = serde_json::to_string(&self).unwrap();
		let bytes = encoded.as_bytes();
		Encode::encode(&bytes)
	}
}

impl Decode for KeyGenMessage {
	fn decode<I: Input>(value: &mut I) -> Result<Self, CodecError> {
		let decoded: Vec<u8> = Decode::decode(value)?;
		let s = str::from_utf8(&decoded).unwrap();
		Ok(serde_json::from_str(s).unwrap())
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SignMessage {
	BroadCast,
	Decommit,
	Proof,
}

impl Encode for SignMessage {
	fn encode(&self) -> Vec<u8> {
		let encoded = serde_json::to_string(&self).unwrap();
		let bytes = encoded.as_bytes();
		Encode::encode(&bytes)
	}
}

impl Decode for SignMessage {
	fn decode<I: Input>(value: &mut I) -> Result<Self, CodecError> {
		let decoded: Vec<u8> = Decode::decode(value)?;
		let s = str::from_utf8(&decoded).unwrap();
		Ok(serde_json::from_str(s).unwrap())
	}
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
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
