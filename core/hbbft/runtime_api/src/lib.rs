#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;
use client::decl_runtime_apis;
pub mod app
{
  use sr_primitives::app_crypto::{app_crypto, hbbft_thresh, key_types::HB_NODE};
  app_crypto!(hbbft_thresh, HB_NODE);

  impl sr_primitives::BoundToRuntimeAppPublic for Public
  {
    type Public = Self;
  }
}

decl_runtime_apis! {


  pub trait HbbftApi {
    fn do_nothing();
  }
}
