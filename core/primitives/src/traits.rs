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

//! Shareable Substrate traits.

#[cfg(feature = "std")]
use crate::{crypto::KeyTypeId, ed25519, sr25519, hbbft_thresh};//child_storage_key::ChildStorageKey,
#[cfg(feature = "std")]
use std::{fmt::{Debug, Display}, panic::UnwindSafe};

pub use externalities::{Externalities, ExternalitiesExt};

/// Something that generates, stores and provides access to keys.
pub trait BareCryptoStore: Send + Sync {
	/// Returns all sr25519 public keys for the given key type.
	fn sr25519_public_keys(&self, id: KeyTypeId) -> Vec<sr25519::Public>;
	/// Generate a new sr25519 key pair for the given key type and an optional seed.
	///
	/// If the given seed is `Some(_)`, the key pair will only be stored in memory.
	///
	/// Returns the public key of the generated key pair.
	fn sr25519_generate_new(
		&mut self,
		id: KeyTypeId,
		seed: Option<&str>,
	) -> Result<sr25519::Public, String>;
	/// Returns the sr25519 key pair for the given key type and public key combination.
	fn sr25519_key_pair(&self, id: KeyTypeId, pub_key: &sr25519::Public) -> Option<sr25519::Pair>;

	/// Returns all ed25519 public keys for the given key type.
	fn ed25519_public_keys(&self, id: KeyTypeId) -> Vec<ed25519::Public>;

	/// Generate a new ed25519 key pair for the given key type and an optional seed.
	///
	/// If the given seed is `Some(_)`, the key pair will only be stored in memory.
	///
	/// Returns the public key of the generated key pair.
	fn ed25519_generate_new(
		&mut self,
		id: KeyTypeId,
		seed: Option<&str>,
	) -> Result<ed25519::Public, String>;

	/// Generate a new HBBFT key pair for the given key type and an optional seed.
	///
	fn hb_node_generate_new(
		&mut self,
		id: KeyTypeId,
		seed: Option<&str>,
	) -> Result<hbbft_thresh::Public, String>;

   /// Returns all ed25519 public keys for the given key type.
	fn hb_node_public_keys(&self, id: KeyTypeId) -> Vec<hbbft_thresh::Public>;

  /// Returns the ed25519 key pair for the given key type and public key combination.
	fn hb_node_key_pair(&self, id: KeyTypeId, pub_key: &hbbft_thresh::Public) -> Option<hbbft_thresh::Pair>;


	/// Returns the ed25519 key pair for the given key type and public key combination.
	fn ed25519_key_pair(&self, id: KeyTypeId, pub_key: &ed25519::Public) -> Option<ed25519::Pair>;

	//fn get_aux<T:Public,O> (&self,id: KeyTypeId,public:T) -> Result<O,()>;  - probably cannot be used without serialization

	/// Insert a new key. This doesn't require any known of the crypto; but a public key must be
	/// manually provided.
	///
	/// Places it into the file system store.
	///
	/// `Err` if there's some sort of weird filesystem error, but should generally be `Ok`.
	fn insert_unknown(&mut self, _key_type: KeyTypeId, _suri: &str, _public: &[u8]) -> Result<(), ()>;

	/// Get the password for this store.
	fn password(&self) -> Option<&str>;

    /// Initiates a (key) request with a given id, returns error if request exists or could not be created
	fn initiate_request(&self, request_id: &[u8],key_type: KeyTypeId) -> Result<(), ()>;

	/// gets data of the request details
    fn get_request_data(&self, request_id: &[u8],key_type: KeyTypeId) -> Option<Vec<u8>>;

	/// sets request data (binary blob) 
	fn set_request_data(&self, request_id: &[u8],key_type: KeyTypeId,request_data: &[u8]) ->Result<(),()>;




}

/// A pointer to the key store.
pub type BareCryptoStorePtr = std::sync::Arc<parking_lot::RwLock<dyn BareCryptoStore>>;

externalities::decl_extension! {
	/// The keystore extension to register/retrieve from the externalities.
	pub struct KeystoreExt(BareCryptoStorePtr);
}

/// Code execution engine.
pub trait CodeExecutor: Sized + Send + Sync {
	/// Externalities error type.
	type Error: Display + Debug + Send + 'static;

	/// Call a given method in the runtime. Returns a tuple of the result (either the output data
	/// or an execution error) together with a `bool`, which is true if native execution was used.
	fn call<
		E: Externalities,
		R: codec::Codec + PartialEq,
		NC: FnOnce() -> Result<R, String> + UnwindSafe,
	>(
		&self,
		ext: &mut E,
		method: &str,
		data: &[u8],
		use_native: bool,
		native_call: Option<NC>,
	) -> (Result<crate::NativeOrEncoded<R>, Self::Error>, bool);
}
