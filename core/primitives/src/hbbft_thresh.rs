use rstd::vec::Vec;

use codec::{Decode, Encode};
use rstd::cmp::Ordering;

//#[cfg(feature = "full_crypto")]
//use core::convert::{TryFrom, TryInto};

#[cfg(feature = "std")]
use substrate_bip39::seed_from_entropy;


#[cfg(feature = "std")]
use log::info;

#[cfg(feature = "std")]
use bip39::{Language, Mnemonic, MnemonicType};

#[cfg(feature = "full_crypto")]
use crate::{
	crypto::{DeriveJunction, Pair as TraitPair, SecretStringError},
	};

use runtime_interface::pass_by::PassByInner;


#[cfg(feature = "std")]
use crate::crypto::Ss58Codec;

#[cfg(feature = "std")]
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use crate::crypto::{CryptoType, Derive, Public as TraitPublic, UncheckedFrom};

#[cfg(feature = "full_crypto")]
use threshold_crypto::{serde_impl::SerdeSecret, PublicKey, SecretKey, Signature as RawSignature};

/// Public key size
pub const PK_SIZE: usize = 48;

/// Secret key... seed? size
pub const SK_SIZE: usize = 32;

/// Signature size
pub const SIG_SIZE: usize = 96;

#[cfg(feature = "full_crypto")]
type Seed = [u8; SK_SIZE];

#[derive(Clone, Encode, Decode,PassByInner)]
pub struct Public(pub [u8; PK_SIZE]);

impl PartialOrd for Public {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(&other))
	}
}
impl Ord for Public {
	fn cmp(&self, other: &Self) -> Ordering {
		self.0.iter().cmp(other.0.iter())
	}
}
impl PartialEq for Public {
	fn eq(&self, other: &Self) -> bool {
		self.0.iter().eq(other.0.iter())
	}
}

impl Eq for Public {}

impl Default for Public {
	fn default() -> Self {
		Public([0u8; PK_SIZE])
	}
}

#[cfg(feature = "full_crypto")]
#[derive(Clone)]
pub struct Pair {
	pub public: PublicKey,
	pub secret: SecretKey,
}

impl AsRef<[u8; PK_SIZE]> for Public {
	fn as_ref(&self) -> &[u8; PK_SIZE] {
		&self.0
	}
}

impl AsRef<[u8]> for Public {
	fn as_ref(&self) -> &[u8] {
		&self.0[..]
	}
}

impl AsMut<[u8]> for Public {
	fn as_mut(&mut self) -> &mut [u8] {
		&mut self.0[..]
	}
}

impl rstd::convert::TryFrom<&[u8]> for Public {
	type Error = ();

	fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
		if data.len() == PK_SIZE {
			let mut inner = [0u8; PK_SIZE];
			inner.copy_from_slice(data);
			Ok(Public(inner))
		} else {
			Err(())
		}
	}
}

impl From<Public> for [u8; PK_SIZE] {
	fn from(x: Public) -> Self {
		x.0
	}
}

#[cfg(feature = "full_crypto")]
impl From<threshold_crypto::PublicKey> for Public
{ 
  fn from(x:threshold_crypto::PublicKey) ->Self
  {
	let arr= bincode::serialize(&x).unwrap();
	let mut inner = [0u8; PK_SIZE];
	inner.copy_from_slice(&arr);
	Public(inner)
  }
}

#[cfg(feature = "full_crypto")]
impl From<Pair> for Public {
	fn from(x: Pair) -> Self {
		x.public()
	}
}

impl UncheckedFrom<[u8; PK_SIZE]> for Public {
	fn unchecked_from(x: [u8; PK_SIZE]) -> Self {
		Public::from_raw(x)
	}
}

#[cfg(feature = "std")]
impl std::fmt::Display for Public {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{}", self.to_ss58check())
	}
}

impl rstd::fmt::Debug for Public {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut std::fmt::Formatter) -> rstd::fmt::Result {
		let s = self.to_ss58check();
		write!(
			f,
			"{} ({}...)",
			crate::hexdisplay::HexDisplay::from(&&self.0[..]),
			&s[0..8]
		)
	}
	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut rstd::fmt::Formatter) -> rstd::fmt::Result {
		Ok(())
	}
}

#[cfg(feature = "std")]
impl Serialize for Public {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_str(&self.to_ss58check())
	}
}

#[cfg(feature = "std")]
impl<'de> Deserialize<'de> for Public {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		Public::from_ss58check(&String::deserialize(deserializer)?)
			.map_err(|e| de::Error::custom(format!("{:?}", e)))
	}
}

#[cfg(feature = "full_crypto")]
impl rstd::hash::Hash for Public {
	fn hash<H: rstd::hash::Hasher>(&self, state: &mut H) {
		self.0.hash(state);
	}
}

#[derive(Encode, Decode,PassByInner)]
pub struct Signature(pub [u8; SIG_SIZE]);

impl rstd::convert::TryFrom<&[u8]> for Signature {
	type Error = ();

	fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
		if data.len() == SIG_SIZE {
			let mut inner = [0u8; SIG_SIZE];
			inner.copy_from_slice(data);
			Ok(Signature(inner))
		} else {
			Err(())
		}
	}
}

impl Clone for Signature {
	fn clone(&self) -> Self {
		let mut r = [0u8; SIG_SIZE];
		r.copy_from_slice(&self.0[..]);
		Signature(r)
	}
}

impl Default for Signature {
	fn default() -> Self {
		Signature([0u8; SIG_SIZE])
	}
}

impl PartialEq for Signature {
	fn eq(&self, b: &Self) -> bool {
		self.0[..] == b.0[..]
	}
}

impl Eq for Signature {}

impl From<Signature> for [u8; SIG_SIZE] {
	fn from(v: Signature) -> [u8; SIG_SIZE] {
		v.0
	}
}

impl AsRef<[u8; SIG_SIZE]> for Signature {
	fn as_ref(&self) -> &[u8; SIG_SIZE] {
		&self.0
	}
}

impl AsRef<[u8]> for Signature {
	fn as_ref(&self) -> &[u8] {
		&self.0[..]
	}
}

impl AsMut<[u8]> for Signature {
	fn as_mut(&mut self) -> &mut [u8] {
		&mut self.0[..]
	}
}

impl rstd::fmt::Debug for Signature {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut rstd::fmt::Formatter) -> rstd::fmt::Result {
		write!(f, "{}", crate::hexdisplay::HexDisplay::from(&self.0))
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut rstd::fmt::Formatter) -> rstd::fmt::Result {
		Ok(())
	}
}

#[cfg(feature = "full_crypto")]
impl rstd::hash::Hash for Signature {
	fn hash<H: rstd::hash::Hasher>(&self, state: &mut H) {
		rstd::hash::Hash::hash(&self.0[..], state);
	}
}

impl Signature {
	pub fn from_raw(data: [u8; SIG_SIZE]) -> Signature {
		Signature(data)
	}
}

#[cfg(feature = "std")]
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum PublicError {
	/// Bad alphabet.
	BadBase58,
	/// Bad length.
	BadLength,
	/// Unknown version.
	UnknownVersion,
	/// Invalid checksum.
	InvalidChecksum,
}

impl Public {
	pub fn from_raw(data: [u8; PK_SIZE]) -> Self {
		Public(data)
	}

	/// Return a slice filled with raw data.
	pub fn as_array_ref(&self) -> &[u8; PK_SIZE] {
		self.as_ref()
	}
}

impl TraitPublic for Public {
	fn from_slice(data: &[u8]) -> Self {
		let mut r = [0u8; PK_SIZE];
		r.copy_from_slice(data);
		Public(r)
	}
}

impl Derive for Public {}

/// Derive a single hard junction.
#[cfg(feature = "full_crypto")]
fn derive_hard_junction(secret_seed: &Seed, cc: &[u8; 32]) -> Seed {
	("threshold_crypto", secret_seed, cc).using_encoded(|data| {
		let mut res = [0u8; SK_SIZE];
		res.copy_from_slice(blake2_rfc::blake2b::blake2b(SK_SIZE, &[], data).as_bytes());
		res
	})
}

/// An error when deriving a key.
#[cfg(feature = "full_crypto")]
pub enum DeriveError {
	/// A soft key was found in the path (and is unsupported).
	SoftKeyInPath,
}

#[cfg(feature = "std")]
use rand_chacha::ChaChaRng;


#[cfg(feature = "full_crypto")]
impl TraitPair for Pair {
	type Public = Public;
	type Seed = Seed;
	type Signature = Signature;
	type DeriveError = DeriveError;

	#[cfg(feature = "std")]
	fn generate_with_phrase(password: Option<&str>) -> (Pair, String, Seed) {
		let mnemonic = Mnemonic::new(MnemonicType::Words12, Language::English);
		let phrase = mnemonic.phrase();
		let (pair, seed) = Self::from_phrase(phrase, password)
			.expect("All phrases generated by Mnemonic are valid; qed");
		(pair, phrase.to_owned(), seed)
	}

	#[cfg(feature = "std")]
	fn from_phrase(
		phrase: &str,
		password: Option<&str>,
	) -> Result<(Pair, Seed), SecretStringError> {
		let big_seed = seed_from_entropy(
			Mnemonic::from_phrase(phrase, Language::English)
				.map_err(|_| SecretStringError::InvalidPhrase)?
				.entropy(),
			password.unwrap_or(""),
		)
		.map_err(|_| SecretStringError::InvalidSeed)?; // 64 bytes

		let mut seed = Seed::default();
		seed.copy_from_slice(&big_seed[0..SK_SIZE]);

		Self::from_seed_slice(&big_seed[0..SK_SIZE]).map(|x| (x, seed))
	}

	fn from_seed(seed: &Seed) -> Pair {
		Self::from_seed_slice(&seed[..]).expect("seed has valid length; qed")
	}

	fn from_seed_slice(seed: &[u8]) -> Result<Pair, SecretStringError> {
		use rand_old::distributions::Standard;
		use rand_old::SeedableRng;
		use rand_old::Rng;
		//let ss:SecretKey =
		let secret: SecretKey = match seed.len() {
			SK_SIZE => { 
				let mut acc: Seed = [0u8; SK_SIZE];
				acc.copy_from_slice(seed);
                Ok(ChaChaRng::from_seed(acc).sample(Standard))
			}
			//bincode::deserialize(seed).map_err(|_| SecretStringError::InvalidSeed),
			_ => Err(SecretStringError::InvalidSeedLength),
		}?;
		let public = secret.public_key();
		info!("PUBLIC generated: {:?}",crate::hexdisplay::HexDisplay::from(&bincode::serialize(&public).unwrap()));
		Ok(Pair { secret, public })
	}

	fn derive<Iter: Iterator<Item = DeriveJunction>>(
		&self,
		path: Iter,
		seed: Option<Seed>,
	) -> Result<(Pair, Option<Seed>), Self::DeriveError> {
		let secret: Vec<u8> = bincode::serialize(&SerdeSecret(&self.secret)).unwrap();
		assert_eq!(secret.len(), SK_SIZE);

		let mut acc: Seed = [0u8; SK_SIZE];
		acc.copy_from_slice(secret.as_slice());

		for j in path {
			match j {
				DeriveJunction::Soft(_) => return Err(DeriveError::SoftKeyInPath),
				DeriveJunction::Hard(cc) => acc = derive_hard_junction(&acc, &cc),
			}
		}
		Ok((Self::from_seed(&acc), Some(acc)))
	}

	fn public(&self) -> Public {
		Public(self.public.to_bytes())
	}

	fn sign(&self, message: &[u8]) -> Signature {
		Signature(self.secret.sign(message).to_bytes())
	}

	fn verify<M: AsRef<[u8]>>(sig: &Self::Signature, message: M, pubkey: &Self::Public) -> bool {
		Self::verify_weak(&sig.0, message, pubkey.0.to_vec())
	}

	fn verify_weak<P: AsRef<[u8]>, M: AsRef<[u8]>>(sig: &[u8], message: M, pubkey: P) -> bool {
		let mut pk_arr = [0u8; PK_SIZE];
		let mut sig_arr = [0u8; SIG_SIZE];
		pk_arr.copy_from_slice(pubkey.as_ref());
		sig_arr.copy_from_slice(sig);

		let pk = PublicKey::from_bytes(pk_arr);
		let sig = RawSignature::from_bytes(sig_arr);
		if pk.is_err() || sig.is_err() {
			return false;
		}

		let (pk, sig) = (pk.unwrap(), sig.unwrap());

		pk.verify(&sig, message)
	}

	fn to_raw_vec(&self) -> Vec<u8> {
		bincode::serialize(&SerdeSecret(&self.secret)).expect("Failed serialize")
	}
}

impl CryptoType for Public {
	#[cfg(feature = "full_crypto")]
	type Pair = Pair;
}

impl CryptoType for Signature {
	#[cfg(feature = "full_crypto")]
	type Pair = Pair;
}

#[cfg(feature = "full_crypto")]
impl CryptoType for Pair {
	type Pair = Pair;
}
