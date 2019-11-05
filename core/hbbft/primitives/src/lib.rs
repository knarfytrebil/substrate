// Copyright 2018-2019 Parity Technologies (UK) Ltd.
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

//! Primitives for Badger integration, suitable for WASM compilation.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use serde::Serialize;

#[cfg(feature = "std")]
use serde::Deserialize;

pub mod app
{
  use sr_primitives::app_crypto::{app_crypto, hbbft_thresh, key_types::HB_NODE};
  app_crypto!(hbbft_thresh, HB_NODE);
}

use badger::crypto::{
  PublicKey, PublicKeySet, PublicKeyShare, SecretKey, SecretKeyShare, Signature,
};
use parity_codec::Input; //Encode, Decode, Codec,
use sr_primitives::ConsensusEngineId;
/// The hbbft crypto scheme defined via the keypair type.
#[cfg(feature = "std")]
pub type AuthorityPair = (PublicKey, SecretKey);

use bincode;
use threshold_crypto::serde_impl::SerdeSecret;

pub const SK_SIZE: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretKeyWrap(pub SecretKey);

impl parity_codec::Encode for SecretKeyWrap
{
  fn encode_to<T: parity_codec::Output>(&self, dest: &mut T)
  {
    //let len = <C::Affine as CurveAffine>::Compressed::size();
    for bt in bincode::serialize(&SerdeSecret(&self.0))
      .expect("Failed serialize")
      .iter()
    {
      dest.push_byte(*bt);
    }
  }
}

impl parity_codec::Decode for SecretKeyWrap
{
  fn decode<I: Input>(value: &mut I) -> Result<Self, parity_codec::Error>
  {
    let mut mt: [u8; SK_SIZE] = [0; SK_SIZE];
    match value.read(&mut mt)
    {
      Ok(_) =>
      {}
      Err(_) => return Err("Error decoding field SecretKeyWrap".into()),
    }

    match bincode::deserialize(&mt)
    {
      Ok(val) => Ok(Self { 0: val }),
      Err(_) => Err("Error decoding field SecretKeyWrap".into()),
    }
  }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct PublicKeySetWrap(pub PublicKeySet);

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct PublicKeyWrap(pub PublicKey);

use badger::crypto::PK_SIZE;
use badger::crypto::SIG_SIZE;

//use badger::pairing::{CurveAffine, CurveProjective, EncodedPoint};

impl parity_codec::Encode for PublicKeyWrap
{
  fn encode_to<T: parity_codec::Output>(&self, dest: &mut T)
  {
    //let len = <C::Affine as CurveAffine>::Compressed::size();
    for byte in self.0.to_bytes().iter()
    {
      dest.push_byte(*byte);
    }
  }
}

impl parity_codec::Decode for PublicKeyWrap
{
  fn decode<I: Input>(value: &mut I) -> Result<Self, parity_codec::Error>
  {
    let mut mt: [u8; PK_SIZE] = [0; PK_SIZE];
    match value.read(&mut mt)
    {
      Ok(_) =>
      {}
      Err(_) => return Err("Error decoding field SecretKeyWrap".into()),
    }
    match PublicKey::from_bytes(&mt)
    {
      Ok(pk) => Ok(Self { 0: pk }),
      Err(_) => Err("Error decoding field PublicKeyWrap".into()),
    }
  }
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq, Hash)]
pub struct PublicKeyShareWrap(PublicKeyShare);

impl parity_codec::Encode for PublicKeyShareWrap
{
  fn encode_to<T: parity_codec::Output>(&self, dest: &mut T)
  {
    //let len = <C::Affine as CurveAffine>::Compressed::size();
    for byte in self.0.to_bytes().iter()
    {
      dest.push_byte(*byte);
    }
  }
}
impl parity_codec::Decode for PublicKeyShareWrap
{
  fn decode<I: Input>(value: &mut I) -> Result<Self, parity_codec::Error>
  {
    let mut mt: [u8; PK_SIZE] = [0; PK_SIZE];
    match value.read(&mut mt)
    {
      Ok(_) =>
      {}
      Err(_) => return Err("Error decoding field SecretKeyWrap".into()),
    }
    match PublicKeyShare::from_bytes(&mt)
    {
      Ok(pk) => Ok(Self { 0: pk }),
      Err(_) => Err("Error decoding field PublicKeyShareWrap".into()),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretKeyShareWrap(pub SecretKeyShare);

impl parity_codec::Encode for SecretKeyShareWrap
{
  fn encode_to<T: parity_codec::Output>(&self, dest: &mut T)
  {
    for bt in bincode::serialize(&SerdeSecret(&self.0))
      .expect("Falied serialize")
      .iter()
    {
      dest.push_byte(*bt);
    }
  }
}
impl parity_codec::Decode for SecretKeyShareWrap
{
  fn decode<I: Input>(value: &mut I) -> Result<Self, parity_codec::Error>
  {
    let mut mt: [u8; SK_SIZE] = [0; SK_SIZE];
    match value.read(&mut mt)
    {
      Ok(_) =>
      {}
      Err(_) => return Err("Error decoding field SecretKeyWrap".into()),
    }
    match bincode::deserialize(&mt)
    {
      Ok(val) => Ok(Self { 0: val }),
      Err(_) => Err("Error decoding field SecretKeyShareWrap".into()),
    }
  }
}

/// Identity of a HBBFT  proposer.
pub type AuthorityId = PublicKeyWrap;

#[derive(Debug, Serialize, Clone, PartialEq, Eq, Hash)]
pub struct SignatureWrap(pub Signature);

impl parity_codec::Encode for SignatureWrap
{
  fn encode_to<T: parity_codec::Output>(&self, dest: &mut T)
  {
    //let len = <C::Affine as CurveAffine>::Compressed::size();
    for byte in self.0.to_bytes().iter()
    {
      dest.push_byte(*byte);
    }
  }
}
impl parity_codec::Decode for SignatureWrap
{
  fn decode<I: Input>(value: &mut I) -> Result<Self, parity_codec::Error>
  {
    let mut mt: [u8; SIG_SIZE] = [0; SIG_SIZE];
    match value.read(&mut mt)
    {
      Ok(_) =>
      {}
      Err(_) => return Err("Error decoding field SecretKeyWrap".into()),
    }
    match Signature::from_bytes(&mt)
    {
      Ok(pk) => Ok(Self { 0: pk }),
      Err(_) => Err("Error decoding field SignatureWrap".into()),
    }
  }
}

/// Signature for a HBBFT  proposer.
pub type AuthoritySignature = SignatureWrap;

/// The `ConsensusEngineId` of BADGER.
pub const HBBFT_ENGINE_ID: ConsensusEngineId = *b"BDGR";
