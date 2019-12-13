use jsonrpc_core::{Error, ErrorCode, Result};
use jsonrpc_derive::rpc;
use badger_primitives::{SignedAccountBinding,AuthorityPair,AccountBinding};
use keystore::KeyStorePtr;
use std::sync::Arc;

use substrate_primitives::{
	Bytes
};
use parity_codec::{Codec,Encode};
use runtime_primitives::traits::{
   Block as BlockT,  ProvideRuntimeApi, //BlakeTwo256,Header,NumberFor
};
use client::blockchain::HeaderBackend;
//use client::{
	//backend::AuxStore,};
use substrate_primitives::crypto::Pair;
/// A struct that encodes RPC parameters required for a call to a smart-contract.
/*#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CallRequest<AccountId, Balance> {
	origin: AccountId,
	dest: AccountId,
	value: Balance,
	gas_limit: number::NumberOrHex<u64>,
	input_data: Bytes,
}*/

/// Contracts RPC methods.
#[rpc]
pub trait BadgerRpcApi<AccountId> {
	/// Executes a call to a contract.
	///
	/// This call is performed locally without submitting any transactions. Thus executing this
	/// won't change any state. Nonetheless, the calling state-changing contracts is still possible.
	///
	/// This method is useful for calling getter-like methods on contracts.
	#[rpc(name = "badgerrpc_bindAccount")]
	fn bind_account(
        &self,
        account:AccountId,
	) -> Result<Bytes>;


}


pub struct BadgerRpcCaller<C,B>
{
    client: Arc<C>,
    keystore:KeyStorePtr,
	_marker: std::marker::PhantomData<B>,

}

impl<C,B> BadgerRpcCaller<C,B> {
	/// Create new `Contracts` with the given reference to the client.
	pub fn new(client: Arc<C>,keystore:KeyStorePtr) -> Self {
		BadgerRpcCaller {
            client,
            keystore,
			_marker: Default::default(),
		}
	}
}
use crate::aux_store::GenesisAuthoritySetProvider;
impl<C,Block,AccountId > BadgerRpcApi<AccountId,>
	for BadgerRpcCaller<C,Block>
where
	Block: BlockT,
	C: Send + Sync + 'static+sc_api::AuxStore+GenesisAuthoritySetProvider<Block>,
	C: ProvideRuntimeApi,
	C: HeaderBackend<Block>,
	AccountId: Codec+core::fmt::Debug,
	//Balance: Codec,
{
    fn bind_account(
        &self,
        account:AccountId,
    ) -> Result<Bytes>
    {
        let genesis_authorities_provider= &*self.client.clone();
        let persistent_data = match crate::aux_store::load_persistent_badger(
            &*self.client,
            || {
                let authorities = genesis_authorities_provider.get()?;
                Ok(authorities)
            },self.keystore.clone() )
            {
                Ok(dat) =>dat,
                Err(_)=> return Err(Error {
                    code: ErrorCode::InternalError,
                    message: "Client error".to_string(),
                    data: None
                })
            };
        let pair:AuthorityPair=
        match self.keystore.read().key_pair_by_type::<AuthorityPair>(&persistent_data.authority_set.inner.read().self_id, app_crypto::key_types::HB_NODE)
        {
            Ok(dat) => dat,
            Err(_) => return Err(Error {
                code: ErrorCode::InternalError,
                message: "Keypair error".to_string(),
                data: None
            })
        };
        let bind=AccountBinding
        {
            self_pub_key: persistent_data.authority_set.inner.read().self_id.clone(),
            bound_account:account,
        };
        let sig=pair.sign(&bind.encode());
        let sgn=SignedAccountBinding
        {
      data:bind,
      sig:sig,
        };
        Ok(sgn.encode().into())

    }
}




