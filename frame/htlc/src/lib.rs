#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::type_complexity)]

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use codec::{Decode, Encode};

use rstd::prelude::*;
use runtime_io::hashing::sha2_256;
use sp_runtime::{
	traits::{Member, One, SimpleArithmetic, StaticLookup, Zero},
	RuntimeDebug,
};
use support::{
	decl_event, decl_module, decl_storage, dispatch::{DispatchResult, DispatchError}, ensure, traits::Time, Parameter,
};
use system::ensure_signed;

pub trait Trait: system::Trait {
	type AssetId: Parameter + SimpleArithmetic + Default + Copy;

	type Balance: Member + Parameter + SimpleArithmetic + Default + Copy;

	type Time: Time;

	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
}

// trait alias
pub type AccountIdOf<T> = <T as system::Trait>::AccountId;
pub type MomentOf<T> = <<T as Trait>::Time as Time>::Moment;

// type alias
pub type SecretHash = [u8; 32]; // SHA-256 hash type
pub type Symbol = Vec<u8>;
pub type Secret = Vec<u8>;

#[derive(Encode, Decode)]
pub struct TokenInfo<AssetId, AccountId> {
	pub symbol: Symbol,
	pub asset_id: AssetId,
	pub owner: AccountId,
}

#[derive(Encode, Decode, Clone, PartialEq, RuntimeDebug)]
pub enum HtlcState {
	Created,
	Claimed,
	Canceled,
}

#[derive(Encode, Decode, Clone)]
pub struct HtlcInfo<AssetId, AccountId, Balance, Moment> {
	pub buyer: AccountId,
	pub symbol: Symbol,
	pub asset_id: AssetId,
	pub amount: Balance,
	pub expiration_in_ms: Moment,
	pub secret_hash: SecretHash,
	pub state: HtlcState,
}

decl_storage! {
	trait Store for Module<T: Trait> as Htlc {
		Balances get(fn balance_of):
			map (T::AssetId, T::AccountId) => T::Balance;

		Allowances get(fn allowance):
			map (T::AssetId, T::AccountId, T::AccountId) => T::Balance;

		NextAssetId get(fn next_asset_id):
			T::AssetId;

		TotalSupply get(fn total_supply):
			map T::AssetId => T::Balance;

		Symbols get(fn symbol_of):
			map T::AssetId => Symbol;

		Token get(fn token_of):
			map Symbol => Option<TokenInfo<T::AssetId, T::AccountId>>;

		Htlc get(fn htlc_of):
			map SecretHash => Option<HtlcInfo<T::AssetId, T::AccountId, T::Balance, MomentOf<T>>>;
	}
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {

		fn deposit_event() = default;

		pub fn create_token(origin, symbol: Symbol) -> DispatchResult {
			let who = ensure_signed(origin)?; // root?
			Self::_create_token(who, symbol)?;

			Ok(())
		}

		pub fn create_htlc(
			origin,
			symbol: Symbol,
			buyer: <T::Lookup as StaticLookup>::Source,
			amount: T::Balance,
			secret_hash: SecretHash,
			expiration_in_ms: MomentOf<T>
		) -> DispatchResult {
			let who = ensure_signed(origin)?; // root?
			let buyer = T::Lookup::lookup(buyer)?;

			Self::_create_htlc(
				who, symbol, buyer, amount, secret_hash, expiration_in_ms
			)?;

			Ok(())
		}

		pub fn claim(origin, secret: Secret) -> DispatchResult {
			// mint
			ensure_signed(origin)?; // root?

			Self::_claim(secret)?;
			Ok(())
		}

		pub fn cancel(origin, secret_hash: SecretHash) -> DispatchResult {
			ensure_signed(origin)?; // root?

			Self::_cancel(secret_hash)?;
			Ok(())
		}

		pub fn transfer(
			origin,
			symbol: Symbol, // or asset_id?
			to: <T::Lookup as StaticLookup>::Source,
			amount: T::Balance
		) -> DispatchResult {
			let from = ensure_signed(origin)?;
			let to = T::Lookup::lookup(to)?;
			let asset_id = Self::asset_id_of(&symbol).ok_or("the symbol does not exist")?;

			Self::_transfer(&asset_id, &from, &to, amount)?;
			Ok(())
		}

		pub fn approve(
			origin,
			symbol: Symbol, // or asset id?
			spender: <T::Lookup as StaticLookup>::Source,
			amount: T::Balance
		) -> DispatchResult {
			let owner = ensure_signed(origin)?;
			let spender = T::Lookup::lookup(spender)?;
			let asset_id = Self::asset_id_of(&symbol).ok_or("the symbol does not exist")?;

			Self::_approve(&asset_id, &owner, &spender, amount)?;
			Ok(())
		}

		pub fn transfer_from(
			origin,
			symbol: Symbol, // or asset id?
			from: <T::Lookup as StaticLookup>::Source,
			to: <T::Lookup as StaticLookup>::Source,
			amount: T::Balance
		) -> DispatchResult {
			let spender = ensure_signed(origin)?;
			let from = T::Lookup::lookup(from)?;
			let to = T::Lookup::lookup(to)?;
			let asset_id = Self::asset_id_of(&symbol).ok_or("the symbol does not exist")?;

			let allowance = Self::get_allowance(&asset_id, &from, &spender);
			ensure!(allowance >= amount, "no enough allowance");

			Self::_transfer(&asset_id, &from, &to, amount)?;
			Self::_approve(&asset_id, &from, &spender, allowance - amount)?;
			Ok(())
		}

		pub fn burn(
			origin,
			symbol: Symbol, // or asset id?
			amount: T::Balance
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let asset_id = Self::asset_id_of(&symbol).ok_or("the symbol does not exist")?;
			Self::_burn(&asset_id, &who, amount)?;
			Ok(())
		}

	}
}

decl_event!(
	pub enum Event<T>
	where
		AccountId = AccountIdOf<T>,
		AssetId = <T as Trait>::AssetId,
		Amount = <T as Trait>::Balance,
	{
		// symbol, asset id
		TokenCreated(Symbol, AssetId),

		// symbol, amount, buyer, secret_hash
		HtlcCreated(Symbol, Amount, AccountId, SecretHash),
		HtlcClaimed(Symbol, Amount, AccountId, SecretHash),
		HtlcCanceled(Symbol, Amount, AccountId, SecretHash),

		// asset id, from, to, amount
		Transfer(AssetId, Option<AccountId>, Option<AccountId>, Amount),
		// asset id, owner, spender, amount
		Approval(AssetId, AccountId, AccountId, Amount),
	}
);

impl<T: Trait> Module<T> {
	// public
	pub fn get_balance_of(asset_id: &T::AssetId, who: &T::AccountId) -> T::Balance {
		Self::balance_of((asset_id, who))
	}

	pub fn get_allowance(
		asset_id: &T::AssetId,
		owner: &T::AccountId,
		spender: &T::AccountId,
	) -> T::Balance {
		Self::allowance((asset_id, owner, spender))
	}

	pub fn asset_id_of(symbol: &[u8]) -> Option<T::AssetId> {
		let token = Self::token_of(symbol);
		token.map(|t| t.asset_id)
	}

	pub fn owner_of(symbol: &[u8]) -> Option<T::AccountId> {
		let token = Self::token_of(symbol);
		token.map(|t| t.owner)
	}

	pub fn is_owner_of(who: &T::AccountId, symbol: &[u8]) -> bool {
		Self::owner_of(symbol) == Some(who.clone())
	}

	// private immutables
	fn hash_of(secret: &[u8]) -> SecretHash {
		sha2_256(secret)
	}

	fn token_exists(symbol: &[u8]) -> bool {
		<Token<T>>::exists(symbol)
	}

	fn htlc_exists(secret_hash: &SecretHash) -> bool {
		<Htlc<T>>::exists(secret_hash)
	}

	// private mutables
	fn set_balance(asset_id: &T::AssetId, who: &T::AccountId, amount: T::Balance) {
		<Balances<T>>::insert((asset_id, who), amount);
	}

	fn set_allowance(
		asset_id: &T::AssetId,
		owner: &T::AccountId,
		spender: &T::AccountId,
		amount: T::Balance,
	) {
		<Allowances<T>>::insert((asset_id, owner, spender), amount);
	}

	// fn withdraw(asset_id: &T::AssetId) {}

	fn _transfer(
		asset_id: &T::AssetId,
		from: &T::AccountId,
		to: &T::AccountId,
		amount: T::Balance,
	) -> DispatchResult {
		let from_balance = Self::get_balance_of(asset_id, from);
		ensure!(from_balance >= amount, "no enough balance");
		Self::set_balance(asset_id, from, from_balance - amount);

		let to_balance = Self::get_balance_of(asset_id, to);
		Self::set_balance(asset_id, to, to_balance + amount);

		Self::deposit_event(RawEvent::Transfer(
			*asset_id,
			Some(from.clone()),
			Some(to.clone()),
			amount,
		));
		Ok(())
	}

	fn _approve(
		asset_id: &T::AssetId,
		owner: &T::AccountId,
		spender: &T::AccountId,
		amount: T::Balance,
	) -> DispatchResult {
		Self::set_allowance(asset_id, owner, spender, amount);

		Self::deposit_event(RawEvent::Approval(
			*asset_id,
			owner.clone(),
			spender.clone(),
			amount,
		));
		Ok(())
	}

	fn _create_token(who: T::AccountId, symbol: Symbol) -> DispatchResult {
		ensure!(!Self::token_exists(&symbol), "token already exists");

		let id = Self::next_asset_id();
		<NextAssetId<T>>::mutate(|id| *id += One::one());

		<Balances<T>>::insert((id, &who), T::Balance::zero());
		<TotalSupply<T>>::insert(id, T::Balance::zero());
		<Symbols<T>>::insert(id, symbol.clone());

		let token = TokenInfo {
			symbol: symbol.clone(),
			asset_id: id,
			owner: who,
		};
		<Token<T>>::insert(&symbol, token);

		Self::deposit_event(RawEvent::TokenCreated(symbol, id));
		Ok(())
	}

	fn _create_htlc(
		owner: T::AccountId,
		symbol: Symbol,
		buyer: T::AccountId,
		amount: T::Balance,
		secret_hash: SecretHash,
		expiration_in_ms: MomentOf<T>,
	) -> DispatchResult {
		ensure!(!Self::htlc_exists(&secret_hash), "htlc already exists");

		if let Some(token) = <Token<T>>::get(&symbol) {
			let asset_id = token.asset_id;
			ensure!(Self::is_owner_of(&owner, &symbol), "not owner");
			ensure!(expiration_in_ms > T::Time::now(), "invalid expiration");

			let htlc = HtlcInfo {
				asset_id,
				amount,
				expiration_in_ms,
				secret_hash,
				symbol: symbol.clone(),
				buyer: buyer.clone(),
				state: HtlcState::Created,
			};

			<Htlc<T>>::insert(&secret_hash, htlc);

			Self::deposit_event(RawEvent::HtlcCreated(symbol, amount, buyer, secret_hash));

			Ok(())
		} else {
			Err(DispatchError::Other("token does not exist"))
		}
	}

	fn _claim(secret: Secret) -> DispatchResult {
		let secret_hash = Self::hash_of(&secret);
		if let Some(htlc) = <Htlc<T>>::get(&secret_hash) {
			ensure!(T::Time::now() <= htlc.expiration_in_ms, "htlc expired");
			ensure!(htlc.state == HtlcState::Created, "invalid htlc state");

			Self::_mint(&htlc.asset_id, &htlc.buyer, htlc.amount)?;

			<Htlc<T>>::mutate(&secret_hash, |old| {
				let new = HtlcInfo {
					state: HtlcState::Claimed,
					..htlc.clone()
				};
				old.replace(new);
			});

			Self::deposit_event(RawEvent::HtlcClaimed(
				htlc.symbol,
				htlc.amount,
				htlc.buyer,
				secret_hash,
			));

			Ok(())
		} else {
			Err(DispatchError::Other("no htlc bound with this secret hash"))
		}
	}

	fn _cancel(secret_hash: SecretHash) -> DispatchResult {
		if let Some(htlc) = <Htlc<T>>::get(&secret_hash) {
			ensure!(
				T::Time::now() > htlc.expiration_in_ms,
				"htlc is not expired yet"
			);
			ensure!(htlc.state == HtlcState::Created, "invalid htlc state");

			<Htlc<T>>::mutate(&secret_hash, |old| {
				let new = HtlcInfo {
					state: HtlcState::Canceled,
					..htlc.clone()
				};
				old.replace(new);
			});

			Self::deposit_event(RawEvent::HtlcCanceled(
				htlc.symbol,
				htlc.amount,
				htlc.buyer,
				secret_hash,
			));
			Ok(())
		} else {
			Err(DispatchError::Other("no htlc bound with this secret hash"))
		}
	}

	fn _mint(asset_id: &T::AssetId, who: &T::AccountId, amount: T::Balance) -> DispatchResult {
		let current_balance = <Balances<T>>::get((asset_id, who));
		Self::set_balance(asset_id, who, current_balance + amount); // checked add?

		let current_supply = <TotalSupply<T>>::get(asset_id);
		<TotalSupply<T>>::insert(asset_id, current_supply + amount);

		Self::deposit_event(RawEvent::Transfer(
			*asset_id,
			None,
			Some(who.clone()),
			amount,
		));

		Ok(())
	}

	fn _burn(asset_id: &T::AssetId, who: &T::AccountId, amount: T::Balance) -> DispatchResult {
		let current_balance = <Balances<T>>::get((asset_id, who));
		if current_balance >= amount {
			Self::set_balance(asset_id, who, current_balance - amount); // checked sub?

			let current_supply = <TotalSupply<T>>::get(asset_id);
			<TotalSupply<T>>::insert(asset_id, current_supply - amount);

			Self::deposit_event(RawEvent::Transfer(
				*asset_id,
				Some(who.clone()),
				None,
				amount,
			));

			Ok(())
		} else {
			Err(DispatchError::Other("No enough balance!"))
		}
	}
}
