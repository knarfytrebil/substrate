// Copyright 2017-2019 Parity Technologies (UK) Ltd.
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

// tag::description[]
//! Simple pairing API.
//!
//! Note: `CHAIN_CODE_LENGTH` must be equal to `crate::crypto::JUNCTION_ID_LEN`
//! for this to work.
// end::description[]
//#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use rand_old::SeedableRng;

//#[cfg(feature = "std")]
//use rand::SeedableRng;
#[cfg(feature = "std")]
use sha2::Sha512;

#[cfg(feature = "std")]
use bincode;

#[cfg(feature = "std")]
use pbkdf2::pbkdf2;

#[cfg(feature = "std")]
use hmac::Hmac;

#[cfg(feature = "std")]
use threshold_crypto::serde_impl::SerdeSecret;

#[cfg(feature = "std")]
use badger::crypto::{PublicKey, SecretKey, Signature as B_Signature};
//PublicKeySet PublicKeyShare SecretKeyShare

#[cfg(feature = "std")]
use rand_old::distributions::Distribution;

#[cfg(feature = "std")]
use rand_chacha::ChaChaRng;

#[cfg(feature = "std")]
use rand_old::distributions::Standard;

#[cfg(feature = "std")]
use bip39::{Language, Mnemonic, MnemonicType};

#[cfg(feature = "std")]
use crate::crypto::{DeriveJunction, Infallible, Pair as TraitPair, SecretStringError, Ss58Codec};

//#[cfg(feature = "std")]
use rstd::cmp::Ordering;

use crate::crypto::{CryptoType, Derive, Public as TraitPublic, UncheckedFrom};
//use crate::hash::{H256, H512};
use codec::{Decode, Encode};

#[cfg(feature = "std")]
use serde::{
  de::Error, de::SeqAccess, de::Visitor, ser::SerializeSeq, Deserialize, Deserializer, Serialize,
  Serializer,
};

use rstd::marker::PhantomData;

/// Public key size
pub const PK_SIZE: usize = 48;
/// Secret key size
pub const SK_SIZE: usize = 32;
/// Signature size
pub const SIG_SIZE: usize = 96;

/// A Badger  public key.
#[derive(Clone, Encode, Decode)]
pub struct Public(pub [u8; PK_SIZE]);

impl Default for Public
{
  fn default() -> Self
  {
    Public([0u8; PK_SIZE])
  }
}

impl PartialOrd for Public
{
  fn partial_cmp(&self, other: &Self) -> Option<Ordering>
  {
    Some(self.cmp(&other))
  }
}
impl Ord for Public
{
  fn cmp(&self, other: &Self) -> Ordering
  {
    self.0.iter().cmp(other.0.iter())
  }
}

impl PartialEq for Public
{
  fn eq(&self, other: &Self) -> bool
  {
    self.0.iter().eq(other.0.iter())
  }
}
impl Eq for Public {}

/// A Badger keypair
#[cfg(feature = "std")]
pub struct Pair(pub Public, pub SecretKey);

#[cfg(feature = "std")]
impl Clone for Pair
{
  fn clone(&self) -> Self
  {
    Pair(self.0.clone(), self.1.clone())
  }
}

impl AsRef<[u8; PK_SIZE]> for Public
{
  fn as_ref(&self) -> &[u8; PK_SIZE]
  {
    &self.0
  }
}

impl AsRef<[u8]> for Public
{
  fn as_ref(&self) -> &[u8]
  {
    &self.0[..]
  }
}

impl AsMut<[u8]> for Public
{
  fn as_mut(&mut self) -> &mut [u8]
  {
    &mut self.0[..]
  }
}

impl From<Public> for [u8; PK_SIZE]
{
  fn from(x: Public) -> [u8; PK_SIZE]
  {
    x.0
  }
}

#[cfg(feature = "std")]
impl From<PublicKey> for Public
{
  fn from(x: PublicKey) -> Public
  {
    let mut ret = [0; PK_SIZE];
    let ser = bincode::serialize(&x).expect("Public key not serialized");
    ret.copy_from_slice(&ser[..PK_SIZE]);
    Public(ret)
  }
}

#[cfg(feature = "std")]
impl From<Signature> for B_Signature
{
  fn from(x: Signature) -> B_Signature
  {
    bincode::deserialize(&x.0).expect("Signature is incorrect")
  }
}

impl rstd::convert::TryFrom<&[u8]> for Public
{
  type Error = ();

  fn try_from(data: &[u8]) -> Result<Self, Self::Error>
  {
    if data.len() == PK_SIZE
    {
      let mut inner = [0u8; PK_SIZE];
      inner.copy_from_slice(data);
      Ok(Public(inner))
    }
    else
    {
      Err(())
    }
  }
}

use rstd::convert::TryFrom;

impl UncheckedFrom<[u8; PK_SIZE]> for Public
{
  fn unchecked_from(x: [u8; PK_SIZE]) -> Self
  {
    Public::from_raw(x)
  }
}

#[cfg(feature = "std")]
impl std::fmt::Display for Public
{
  fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result
  {
    write!(f, "{}", self.to_ss58check())
  }
}

impl rstd::fmt::Debug for Public
{
  #[cfg(feature = "std")]
  fn fmt(&self, f: &mut std::fmt::Formatter) -> ::std::fmt::Result
  {
    let s = self.to_ss58check();
    write!(
      f,
      "{} ({}...)",
      crate::hexdisplay::HexDisplay::from(&self.0),
      &s[0..8]
    )
  }
  #[cfg(not(feature = "std"))]
  fn fmt(&self, _: &mut rstd::fmt::Formatter) -> rstd::fmt::Result
  {
    Ok(())
  }
}

#[cfg(feature = "std")]
impl Serialize for Public
{
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
    for e in self.0.iter()
    {
      seq.serialize_element(e)?;
    }
    seq.end()
    //serializer.serialize_str(&self.to_ss58check())
  }
}

#[cfg(feature = "std")]
struct PkVisitor<T>
{
  element: PhantomData<T>,
}

#[cfg(feature = "std")]
impl<'de, T> Visitor<'de> for PkVisitor<T>
where
  T: Default + Copy + Deserialize<'de>,
{
  type Value = [T; PK_SIZE];

  fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result
  {
    formatter.write_str(concat!("an array of length ", 32))
  }

  fn visit_seq<A>(self, mut seq: A) -> Result<[T; PK_SIZE], A::Error>
  where
    A: SeqAccess<'de>,
  {
    let mut arr = [T::default(); PK_SIZE];
    for i in 0..PK_SIZE
    {
      arr[i] = seq
        .next_element()?
        .ok_or_else(|| A::Error::invalid_length(i, &self))?;
    }
    Ok(arr)
  }
}

#[cfg(feature = "std")]
impl<'de> Deserialize<'de> for Public
{
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let visitor = PkVisitor {
      element: PhantomData,
    };
    match deserializer.deserialize_seq(visitor)
    {
      Ok(res) => Ok(Public(res)),
      Err(e) => Err(e),
    }
  }
}

#[cfg(feature = "std")]
impl std::hash::Hash for Public
{
  fn hash<H: std::hash::Hasher>(&self, state: &mut H)
  {
    self.0.hash(state);
  }
}

/// An Schnorrkel/Ristretto x25519 ("sr25519") signature.
///
/// Instead of importing it for the local module, alias it to be available as a public type
#[derive(Encode, Decode)]
pub struct Signature(pub [u8; SIG_SIZE]);

impl rstd::convert::TryFrom<&[u8]> for Signature
{
  type Error = ();

  fn try_from(data: &[u8]) -> Result<Self, Self::Error>
  {
    if data.len() == SIG_SIZE
    {
      let mut inner = [0u8; SIG_SIZE];
      inner.copy_from_slice(data);
      Ok(Signature(inner))
    }
    else
    {
      Err(())
    }
  }
}

impl Clone for Signature
{
  fn clone(&self) -> Self
  {
    let mut r = [0u8; SIG_SIZE];
    r.copy_from_slice(&self.0[..]);
    Signature(r)
  }
}

impl Default for Signature
{
  fn default() -> Self
  {
    Signature([0u8; SIG_SIZE])
  }
}

impl PartialEq for Signature
{
  fn eq(&self, b: &Self) -> bool
  {
    self.0[..] == b.0[..]
  }
}

impl Eq for Signature {}

impl From<Signature> for [u8; SIG_SIZE]
{
  fn from(v: Signature) -> [u8; SIG_SIZE]
  {
    v.0
  }
}

impl AsRef<[u8; SIG_SIZE]> for Signature
{
  fn as_ref(&self) -> &[u8; SIG_SIZE]
  {
    &self.0
  }
}

impl AsRef<[u8]> for Signature
{
  fn as_ref(&self) -> &[u8]
  {
    &self.0[..]
  }
}

impl AsMut<[u8]> for Signature
{
  fn as_mut(&mut self) -> &mut [u8]
  {
    &mut self.0[..]
  }
}

#[cfg(feature = "std")]
impl From<B_Signature> for Signature
{
  fn from(s: B_Signature) -> Signature
  {
    Signature(s.to_bytes())
  }
}

impl rstd::fmt::Debug for Signature
{
  #[cfg(feature = "std")]
  fn fmt(&self, f: &mut rstd::fmt::Formatter) -> rstd::fmt::Result
  {
    write!(f, "{}", crate::hexdisplay::HexDisplay::from(&self.0))
  }

  #[cfg(not(feature = "std"))]
  fn fmt(&self, _: &mut rstd::fmt::Formatter) -> rstd::fmt::Result
  {
    Ok(())
  }
}

#[cfg(feature = "std")]
impl std::hash::Hash for Signature
{
  fn hash<H: std::hash::Hasher>(&self, state: &mut H)
  {
    std::hash::Hash::hash(&self.0[..], state);
  }
}

/// A localized signature also contains sender information.
/// NOTE: Encode and Decode traits are supported in ed25519 but not possible for now here.
#[cfg(feature = "std")]
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct LocalizedSignature
{
  /// The signer of the signature.
  pub signer: Public,
  /// The signature itself.
  pub signature: Signature,
}

impl Signature
{
  /// A new instance from the given 64-byte `data`.
  ///
  /// NOTE: No checking goes on to ensure this is a real signature. Only use
  /// it if you are certain that the array actually is a signature, or if you
  /// immediately verify the signature.  All functions that verify signatures
  /// will fail if the `Signature` is not actually a valid signature.
  pub fn from_raw(data: [u8; SIG_SIZE]) -> Signature
  {
    Signature(data)
  }

  /// A new instance from the given slice that should be 64 bytes long.
  ///
  /// NOTE: No checking goes on to ensure this is a real signature. Only use it if
  /// you are certain that the array actually is a signature. GIGO!
  pub fn from_slice(data: &[u8]) -> Self
  {
    let mut r = [0u8; SIG_SIZE];
    r.copy_from_slice(data);
    Signature(r)
  }
}

impl Derive for Public
{
  /// Derive a child key from a series of given junctions.
  ///
  /// `None` if there are any hard junctions in there.
  #[cfg(feature = "std")]
  fn derive<Iter: Iterator<Item = DeriveJunction>>(&self, _path: Iter) -> Option<Public>
  {
    /*let mut acc = PublicKey::from_bytes(self.as_ref()).ok()?;
    for j in path {
      match j {
        DeriveJunction::Soft(cc) => acc = acc.derived_key_simple(ChainCode(cc), &[]).0,
        DeriveJunction::Hard(_cc) => return None,
      }
    }
    Some(Self(acc.to_bytes()))*/
    //no derivation...

    Some(self.clone())
  }
}

impl Public
{
  /// A new instance from the given 32-byte `data`.
  ///
  /// NOTE: No checking goes on to ensure this is a real public key. Only use it if
  /// you are certain that the array actually is a pubkey. GIGO!
  pub fn from_raw(data: [u8; PK_SIZE]) -> Self
  {
    Public(data)
  }

  /// Return a slice filled with raw data.
  pub fn as_array_ref(&self) -> &[u8; PK_SIZE]
  {
    self.as_ref()
  }
}

impl TraitPublic for Public
{
  /// A new instance from the given slice that should be 32 bytes long.
  ///
  /// NOTE: No checking goes on to ensure this is a real public key. Only use it if
  /// you are certain that the array actually is a pubkey. GIGO!
  fn from_slice(data: &[u8]) -> Self
  {
    let mut r = [0u8; PK_SIZE];
    r.copy_from_slice(data);
    Public(r)
  }
}

#[cfg(feature = "std")]
impl From<SecretKey> for Pair
{
  fn from(sec: SecretKey) -> Pair
  {
    Pair(sec.public_key().into(), sec)
  }
}

#[cfg(feature = "std")]
impl From<Pair> for SecretKey
{
  fn from(p: Pair) -> SecretKey
  {
    p.1
  }
}

/// The raw secret key, which can be used to recreate the `Pair`.
#[cfg(feature = "std")]
type Seed = [u8; SK_SIZE];

#[cfg(feature = "std")]
impl TraitPair for Pair
{
  type Public = Public;
  type Seed = Seed;
  type Signature = Signature;
  type DeriveError = Infallible;

  /// Make a new key pair from raw secret seed material.
  ///
  /// This is generated using schnorrkel's Mini-Secret-Keys.
  ///
  /// A MiniSecretKey is literally what Ed25519 calls a SecretKey, which is just 32 random bytes.
  fn from_seed(seed: &Seed) -> Pair
  {
    Self::from_seed_slice(&seed[..]).expect("32 bytes can always build a key; qed")
  }

  /// Get the public key.
  fn public(&self) -> Public
  {
    let mut pk = [0u8; PK_SIZE];
    pk.copy_from_slice(self.0.as_ref());
    Public(pk)
  }

  /// Make a new key pair from secret seed material. The slice must be 32 bytes long or it
  /// will return `None`.
  ///
  /// You should never need to use this; generate(), generate_with_phrase(), from_phrase()
  fn from_seed_slice(seed: &[u8]) -> Result<Pair, SecretStringError>
  {
    match seed.len()
    {
      SK_SIZE =>
      {
        let secr: SecretKey =
          bincode::deserialize(seed).map_err(|_| SecretStringError::InvalidSeed)?;
        Ok(secr.into())
      }
      _ => Err(SecretStringError::InvalidSeedLength),
    }
  }
  fn generate_with_phrase(password: Option<&str>) -> (Pair, String, Seed)
  {
    let mnemonic = Mnemonic::new(MnemonicType::Words12, Language::English);
    let phrase = mnemonic.phrase();
    let (pair, seed) = Self::from_phrase(phrase, password)
      .expect("All phrases generated by Mnemonic are valid; qed");
    (pair, phrase.to_owned(), seed)
  }

  fn from_phrase(phrase: &str, password: Option<&str>) -> Result<(Pair, Seed), SecretStringError>
  {
    Mnemonic::from_phrase(phrase, Language::English)
      .map_err(|_| SecretStringError::InvalidPhrase)
      .map(|m| Self::from_entropy(m.entropy(), password))
  }

  fn derive<Iter: Iterator<Item = DeriveJunction>>(
    &self,
    _path: Iter,
    seed: Option<Seed>,
  ) -> Result<(Pair, Option<Seed>), Self::DeriveError>
  {
    /*let init = self.0.secret.clone();
    let result = path.fold(init, |acc, j| match j {
      DeriveJunction::Soft(cc) => acc.derived_key_simple(ChainCode(cc), &[]).0,
      DeriveJunction::Hard(cc) => derive_hard_junction(&acc, &cc),
    });*/
    Ok((self.clone(), None)) //Self(result.into()))
  }

  fn sign(&self, message: &[u8]) -> Signature
  {
    //let context = signing_context(SIGNING_CTX);
    self.1.sign(message).into()
  }

  /// Verify a signature on a message. Returns true if the signature is good.
  fn verify<M: AsRef<[u8]>>(sig: &Self::Signature, message: M, pubkey: &Self::Public) -> bool
  {
    match PublicKey::from_bytes(pubkey.0)
    {
      Ok(pk) => pk.verify(&sig.clone().into(), message),
      Err(_) => return false,
    }
  }

  /// Verify a signature on a message. Returns true if the signature is good.
  fn verify_weak<P: AsRef<[u8]>, M: AsRef<[u8]>>(sig: &[u8], message: M, pubkey: P) -> bool
  {
    let mut ser: [u8; PK_SIZE] = [0; PK_SIZE];
    ser.copy_from_slice(pubkey.as_ref());
    match PublicKey::from_bytes(ser)
    {
      Ok(pk) => pk.verify(
        &Signature::try_from(sig).expect("Length error").into(),
        message,
      ),
      Err(_) => return false,
    }
  }
  /// Return a vec filled with raw data.
  fn to_raw_vec(&self) -> Vec<u8>
  {
    bincode::serialize(&SerdeSecret(&self.1)).expect("Failed serialize")
  }
}

#[cfg(feature = "std")]
impl Pair
{
  /// Make a new key pair from binary data derived from a valid seed phrase.
  ///
  /// This uses a key derivation function to convert the entropy into a seed, then returns
  /// the pair generated from it.
  pub fn from_entropy(entropy: &[u8], password: Option<&str>) -> (Pair, Seed)
  {
    let salt = format!("snakemushroom{}", password.unwrap_or(""));
    let mut seed = [0u8; 64];
    pbkdf2::<Hmac<Sha512>>(entropy, salt.as_bytes(), 2048, &mut seed);
    let mut seed_half = [0u8; SK_SIZE];
    seed_half.copy_from_slice(&seed[..SK_SIZE]);
    let sk: SecretKey = Standard.sample(&mut ChaChaRng::from_seed(seed_half));
    let ser = bincode::serialize(&SerdeSecret(&sk)).expect("Failed serialize qed");
    seed_half.copy_from_slice(&ser[..SK_SIZE]);
    (sk.into(), seed_half)
  }
}

impl CryptoType for Public
{
  #[cfg(feature = "std")]
  type Pair = Pair;
}

impl CryptoType for Signature
{
  #[cfg(feature = "std")]
  type Pair = Pair;
}

#[cfg(feature = "std")]
impl CryptoType for Pair
{
  type Pair = Pair;
}
