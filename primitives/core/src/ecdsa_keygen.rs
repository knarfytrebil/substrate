#[cfg(feature = "std")]
use curv::elliptic::curves::*;

#[cfg(feature = "std")]
use curv::arithmetic::big_gmp::BigInt;

#[cfg(feature = "std")]
use curv::elliptic::curves::traits::*;

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use bincode;

//#[cfg(feature = "std")]
//use digest::digest::Digest;

//#[cfg(feature = "std")]
//use crate::Public as PTrait;

#[cfg(feature = "std")]
use rand_old::SeedableRng;

#[cfg(feature = "std")]
use sha2::Sha512;

#[cfg(feature = "std")]
use sha2::{Digest, Sha256};

#[cfg(feature = "std")]
use pbkdf2::pbkdf2;

#[cfg(feature = "std")]
use hmac::Hmac;

//#[cfg(feature = "std")]
//use rand::distributions::Distribution;

#[cfg(feature = "std")]
use rand_chacha::ChaChaRng;

#[cfg(feature = "std")]
use rand_old::RngCore;

#[cfg(feature = "std")]
use bip39::{Language, Mnemonic, MnemonicType};

#[cfg(feature = "std")]
use crate::crypto::{DeriveJunction, Infallible, Pair as TraitPair, SecretStringError, Ss58Codec};

//#[cfg(feature = "std")]
use rstd::cmp::Ordering;

#[cfg(feature = "std")]
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::{
  verify as sig_verify, Signature as ECSig,
};

use crate::crypto::{CryptoType, Derive, Public as TraitPublic};
//UncheckedFrom
//use crate::hash::{H256, H512};
use codec::{Decode, Encode};

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize}; // Serializer,de::Visitor,de::SeqAccess,ser::SerializeSeq,de::Error}; //

// use rstd::marker::PhantomData;

/// Public key size
pub const PK_SIZE: usize = 33;
/// Secret key size
pub const SK_SIZE: usize = 32;
/// Signature size
pub const SIG_SIZE: usize = 65;

/// Public key for ecdsa
#[derive(Clone, Encode, Decode)]
#[cfg_attr(feature = "std", derive(Serialize, Hash, Deserialize))]
pub struct Public(pub Vec<u8>);

impl Default for Public
{
  fn default() -> Self
  {
    Public(Vec::new())
  }
}

impl From<Public> for Vec<u8>
{
  fn from(x: Public) -> Vec<u8>
  {
    x.0
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

#[cfg(feature = "std")]
impl From<secp256_k1::Secp256k1Point> for Public
{
  fn from(x: secp256_k1::Secp256k1Point) -> Public
  {
    Public(bincode::serialize(&x).expect("Public serialize error"))
  }
}

#[cfg(feature = "std")]
impl From<Public> for secp256_k1::Secp256k1Point
{
  fn from(x: Public) -> secp256_k1::Secp256k1Point
  {
    bincode::deserialize(&x.0).unwrap_or(secp256_k1::Secp256k1Point::random_point())
  }
}

#[cfg(feature = "std")]
impl std::fmt::Display for Public
{
  fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result
  {
    write!(f, "PublicKey:{}", self.to_ss58check())
  }
}

impl rstd::convert::TryFrom<&[u8]> for Public
{
  type Error = ();

  fn try_from(data: &[u8]) -> Result<Self, Self::Error>
  {
    if data.len() == PK_SIZE
    {
      let mut inner = Vec::new();
      inner.copy_from_slice(data);
      Ok(Public(inner))
    }
    else
    {
      Err(())
    }
  }
}
#[cfg(feature = "std")]
impl std::fmt::Debug for Public
{
  fn fmt(&self, f: &mut std::fmt::Formatter) -> ::std::fmt::Result
  {
    let s = self.to_ss58check();
    write!(
      f,
      "PublicKey:{} ({}...)",
      crate::hexdisplay::HexDisplay::from(&self.0),
      &s[0..8]
    )
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

/// A esdsa (pure) keypair
#[cfg(feature = "std")]
pub struct Pair(pub Public, pub secp256_k1::Secp256k1Scalar);

#[cfg(feature = "std")]
impl Clone for Pair
{
  fn clone(&self) -> Self
  {
    Pair(self.0.clone(), self.1.clone())
  }
}

impl Derive for Public
{
  /// Derive a child key from a series of given junctions.
  ///
  /// `None` if there are any hard junctions in there.
  /// Faked out for now, since we don't derive in keygen
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

/// signature for ecdsa
#[derive(Encode, Decode)]
pub struct Signature(pub [u8; SIG_SIZE]);

#[cfg(feature = "std")]
impl From<Signature> for ECSig
{
  fn from(x: Signature) -> ECSig
  {
    bincode::deserialize(&x.0).expect("Signature is incorrect")
  }
}

/*
#[cfg(feature = "std")]
impl From<&Signature> for &ECSig {
  fn from(x: &Signature) -> &ECSig {
    &bincode::deserialize(&x.0).expect("Signature is incorrect")
  }
}*/

use rstd::convert::TryFrom;
use rstd::vec::Vec;

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
impl From<ECSig> for Signature
{
  fn from(s: ECSig) -> Signature
  {
    let bts = bincode::serialize(&s).expect("Could not serialize");
    let mut arr = [0u8; SIG_SIZE];
    arr.copy_from_slice(&bts);
    Signature(arr)
  }
}

#[cfg(feature = "std")]
impl std::fmt::Debug for Signature
{
  fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result
  {
    write!(
      f,
      "Signature:{}",
      crate::hexdisplay::HexDisplay::from(&self.0)
    )
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

impl Public
{
  /// A new instance from the given 32-byte `data`.
  ///
  /// NOTE: No checking goes on to ensure this is a real public key. Only use it if
  /// you are certain that the array actually is a pubkey. GIGO!
  pub fn from_raw(data: Vec<u8>) -> Self
  {
    Public(data)
  }

  /// Return a slice filled with raw data.
  pub fn as_array_ref(&self) -> &Vec<u8>
  {
    &self.0
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
    let mut r = Vec::<u8>::new();
    r.copy_from_slice(data);
    Public(r)
  }
}

#[cfg(feature = "std")]
impl From<secp256_k1::Secp256k1Scalar> for Pair
{
  fn from(sec: secp256_k1::Secp256k1Scalar) -> Pair
  {
    let gen = secp256_k1::Secp256k1Point::generator();
    let pk = gen.scalar_mul(&sec.get_element());
    Pair(pk.into(), sec)
  }
}

#[cfg(feature = "std")]
impl From<Pair> for secp256_k1::Secp256k1Scalar
{
  fn from(p: Pair) -> secp256_k1::Secp256k1Scalar
  {
    p.1
  }
}

/// The raw secret key, which can be used to recreate the `Pair`.
#[cfg(feature = "std")]
type Seed = [u8; SK_SIZE];

#[cfg(feature = "std")]
use secp256k1_wrap::{Message, Secp256k1};

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
    let mut pk = Vec::new();
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
        let secr: secp256_k1::Secp256k1Scalar =
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
    Ok((self.clone(), seed)) //Self(result.into()))
  }

  fn sign(&self, message: &[u8]) -> Signature
  {
    //let context = signing_context(SIGNING_CTX);
    //don't actually use it!
    let mut hasher = Sha256::new();
    hasher.input(message);
    let result = hasher.result();
    let message = Message::from_slice(&result[..32]).expect("32 bytes");
    let secp = Secp256k1::new();
    let sig = secp.sign(&message, &self.1.get_element());
    let mut buf = [0u8; SIG_SIZE];
    buf.copy_from_slice(&bincode::serialize(&sig).unwrap()[..]);
    Signature { 0: buf }
    //self.1.sign(message).into()
  }

  /// Verify a signature on a message. Returns true if the signature is good.
  fn verify<M: AsRef<[u8]>>(sig: &Self::Signature, message: M, pubkey: &Self::Public) -> bool
  {
    let mut hasher = Sha256::new();
    hasher.input(message.as_ref());
    let result = hasher.result();
    let sg_tr = sig.clone();

    match secp256_k1::Secp256k1Point::from_bytes(&pubkey.0)
    {
      Ok(pk) => match sig_verify(&sg_tr.into(), &pk, &BigInt::from(&result[..]))
      {
        Ok(_) => true,
        Err(_) => false,
      }, //pk.verify(&sig.clone().into(), message),
      Err(_) => return false,
    }
  }

  /// Verify a signature on a message. Returns true if the signature is good.
  fn verify_weak<P: AsRef<[u8]>, M: AsRef<[u8]>>(sig: &[u8], message: M, pubkey: P) -> bool
  {
    let mut ser = Vec::<u8>::new();
    let mut hasher = Sha256::new();
    hasher.input(message.as_ref());
    let result = hasher.result();
    ser.copy_from_slice(pubkey.as_ref());
    match secp256_k1::Secp256k1Point::from_bytes(&ser)
    {
      Ok(pk) => match sig_verify(
        &Signature::try_from(sig).expect("Length error").into(),
        &pk,
        &BigInt::from(&result[..]),
      )
      {
        Ok(_) => true,
        Err(_) => false,
      }, // pk.verify(& Signature::try_from(sig).expect("Length error").into(), message),
      Err(_) => return false,
    }
  }
  /// Return a vec filled with raw data.
  fn to_raw_vec(&self) -> Vec<u8>
  {
    bincode::serialize(&self.1).expect("Failed serialize")
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
    let mut arr = [0u8; 32];
    ChaChaRng::from_seed(seed_half).fill_bytes(&mut arr[..]);

    //let big:BigInt= BigInt::from(&arr);

    let mut sk: secp256_k1::Secp256k1Scalar = ECScalar::zero();
    sk.set_element(secp256_k1::SK::from_slice(&arr[0..arr.len()]).unwrap());
    let ser = bincode::serialize(&sk).expect("Failed serialize qed");
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
