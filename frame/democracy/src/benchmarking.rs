// Copyright 2020 Parity Technologies (UK) Ltd.
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

//! Democracy pallet benchmarking.

use super::*;

use frame_benchmarking::{benchmarks, account};
use frame_support::traits::{Currency, Get};
use frame_system::{RawOrigin, Module as System, self};
use pallet_collective::{RawOrigin as CollectiveOrigin, Instance1, Instance2};
use sp_runtime::traits::Bounded;

use crate::Module as Democracy;

const SEED: u32 = 0;
const MAX_PROPOSALS: u32 = 100;
const MAX_REFERENDUMS: u32 = 100;
const MAX_USERS: u32 = 100;

fn add_proposals<T: Trait>(number: u32) -> Result<(), &'static str> {
	for p in 0 .. number {
		let other: T::AccountId = account("other", p, SEED);
		let _ = T::Currency::make_free_balance_be(&other, BalanceOf::<T>::max_value());
		let value = T::MinimumDeposit::get();

		let proposal_hash: T::Hash = Default::default();

		Democracy::<T>::propose(RawOrigin::Signed(other).into(), proposal_hash, value.into())?;
	}
	Ok(())
}

fn add_referendums<T: Trait>(number: u32) -> Result<(), &'static str> {
	for _ in 0 .. number {
		add_proposals::<T>(1)?;

		let vote_threshold = VoteThreshold::SimpleMajority;
		Democracy::<T>::inject_referendum(
			0.into(),
			Default::default(),
			vote_threshold,
			0.into(),
		);
	}
	Ok(())
}

benchmarks! {
	_ {
		let p in 1 .. MAX_PROPOSALS => add_proposals::<T>(p)?;
		let r in 1 .. MAX_REFERENDUMS => add_referendums::<T>(r)?;
		let u in 1 .. MAX_USERS => ();
	}

	propose {
		// The execution time doesn't seems to change depending on the number of proposals.
		let p in ...;

		let caller: T::AccountId = account("caller", 0, SEED);
		let proposal_hash: T::Hash = Default::default();
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());
		let value = T::MinimumDeposit::get();

	}: _(RawOrigin::Signed(caller), proposal_hash, value.into())

	second {
		// The execution time doesn't seems to change depending on the number of proposals.
		let p in ...;

		let caller: T::AccountId = account("caller", 0, SEED);
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());

		// The index of the proposal p is (p - 1);
	}: _(RawOrigin::Signed(caller), (p - 1).into())

	vote {
		// The execution time doesn't seems to change depending on inputs.
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());

		// Add a proposal.
		add_proposals::<T>(1)?;

		// Inject referendum.
		let proposal_hash: T::Hash = Default::default();
		let vote_threshold = VoteThreshold::SimpleMajority;
		Democracy::<T>::inject_referendum(
			0.into(),
			proposal_hash,
			vote_threshold,
			0.into(),
		);

		// Vote.
		let v = Vote {
			aye: true,
			conviction: Conviction::Locked1x,
		};

	}: _(RawOrigin::Signed(caller), 0u32.into(), v)

	proxy_vote {
		// The execution time doesn't seems to change depending on inputs.
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());

		let proxy: T::AccountId = account("proxy", u + MAX_USERS, SEED);
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());

		Democracy::<T>::open_proxy(RawOrigin::Signed(proxy.clone()).into(), caller.clone())?;
		Democracy::<T>::activate_proxy(RawOrigin::Signed(caller).into(), proxy.clone())?;

		add_proposals::<T>(1)?;

		let proposal_hash: T::Hash = Default::default();
		let vote_threshold = VoteThreshold::SimpleMajority;
		Democracy::<T>::inject_referendum(
			0.into(),
			proposal_hash,
			vote_threshold,
			0.into(),
		);

		let v = Vote {
			aye: true,
			conviction: Conviction::Locked1x,
		};

	}: _(RawOrigin::Signed(proxy), 0u32.into(), v)

	emergency_cancel {
		let u in ...;

		add_referendums::<T>(1)?;

		let origin = CollectiveOrigin::<T::AccountId, Instance1>::Members(0u32, 0u32).into();
		let referendum_index = 0u32;
		let call = Call::<T>::emergency_cancel(referendum_index.into());
	}: {
		let _ = call.dispatch(origin)?;
	}

	external_propose {
		let u in ...;

		let origin = CollectiveOrigin::<T::AccountId, Instance1>::Members(0u32, 0u32).into();
		let proposal_hash = Default::default();
		let call = Call::<T>::external_propose(proposal_hash);

	}: {
		let _ = call.dispatch(origin)?;
	}

	external_propose_majority {
		let u in ...;

		let origin = CollectiveOrigin::<T::AccountId, Instance1>::Members(0u32, 0u32).into();
		let proposal_hash = Default::default();
		let call = Call::<T>::external_propose_majority(proposal_hash);

	}: {
		let _ = call.dispatch(origin)?;
	}

	external_propose_default {
		let u in ...;

		let origin = CollectiveOrigin::<T::AccountId, Instance1>::Members(0u32, 0u32).into();
		let proposal_hash = Default::default();
		let call = Call::<T>::external_propose_default(proposal_hash);

	}: {
		let _ = call.dispatch(origin)?;
	}

	fast_track {
		let u in ...;

		let origin = CollectiveOrigin::<T::AccountId, Instance2>::Members(0u32, 0u32).into();
		let proposal_hash: T::Hash = Default::default();

		let origin_propose = CollectiveOrigin::<T::AccountId, Instance1>::Members(0u32, 0u32).into();
		Democracy::<T>::external_propose_default(origin_propose, proposal_hash.clone())?;

		let voting_period = 0;
		let delay = 0;
		let call = Call::<T>::fast_track(proposal_hash, voting_period.into(), delay.into());

	}: {
		let _ = call.dispatch(origin)?;
	}

	veto_external {
		let u in ...;

		let proposal_hash: T::Hash = Default::default();

		let origin_propose = CollectiveOrigin::<T::AccountId, Instance1>::Members(0u32, 0u32).into();
		Democracy::<T>::external_propose_default(origin_propose, proposal_hash.clone())?;

		let call = Call::<T>::veto_external(proposal_hash);
		let origin = CollectiveOrigin::<T::AccountId, Instance2>::Member(Default::default()).into();

	}: {
		let _ = call.dispatch(origin)?;
	}

	cancel_referendum {
		let u in ...;

		add_referendums::<T>(1)?;

	}: _(RawOrigin::Root, 0u32.into())

	cancel_queued {
		let u in ...;

		// TODO: we could add more items to the DispatchQueue to bench.
		add_referendums::<T>(1)?;
		let block_number: T::BlockNumber = 0.into();
		let hash: T::Hash = Default::default();
		let referendum_index: ReferendumIndex = 0u32.into(); 
		<DispatchQueue<T>>::put(vec![(block_number, hash, referendum_index)]);

	}: _(RawOrigin::Root, 0u32.into())

	open_proxy {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		let proxy: T::AccountId = account("proxy", u + 1, SEED);

	}: _(RawOrigin::Signed(proxy), caller)

	activate_proxy {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		let proxy: T::AccountId = account("proxy", u + 1, SEED);

		Democracy::<T>::open_proxy(RawOrigin::Signed(proxy.clone()).into(), caller.clone())?;

	}: _(RawOrigin::Signed(caller), proxy)

	close_proxy {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		let proxy: T::AccountId = account("proxy", u + 1, SEED);

		Democracy::<T>::open_proxy(RawOrigin::Signed(proxy.clone()).into(), caller.clone())?;
		Democracy::<T>::activate_proxy(RawOrigin::Signed(caller).into(), proxy.clone())?;

	}: _(RawOrigin::Signed(proxy))

	deactivate_proxy {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		let proxy: T::AccountId = account("proxy", u + 1, SEED);

		Democracy::<T>::open_proxy(RawOrigin::Signed(proxy.clone()).into(), caller.clone())?;
		Democracy::<T>::activate_proxy(RawOrigin::Signed(caller.clone()).into(), proxy.clone())?;

	}: _(RawOrigin::Signed(caller), proxy)

	delegate {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());
		let d: T::AccountId = account("delegate", u, SEED);

	}: _(RawOrigin::Signed(caller), d.into(), Conviction::Locked1x)

	undelegate {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());

		for i in 0 .. u {
			let d: T::AccountId = account("delegator", u + i + 1, SEED);
			let conviction = Conviction::Locked1x;
			Democracy::<T>::delegate(RawOrigin::Signed(d.clone()).into(), caller.clone().into(), conviction)?;
		}

		let d: T::AccountId = account("delegator", u + 1, SEED);

	}: _(RawOrigin::Signed(d))

	clear_public_proposals {
		let u in ...;
		// TODO: maybe add some proposals to kill.
	}: _(RawOrigin::Root)

	note_preimage {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());

		let mut encoded_proposal = vec![];
		for i in 0 .. u {
			encoded_proposal.push(Default::default());
		}

	}: _(RawOrigin::Signed(caller), encoded_proposal)

	note_imminent_preimage {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());

		let mut encoded_proposal = vec![];
		for i in 0 .. u {
			encoded_proposal.push(Default::default());
		}

		let proposal_hash = T::Hashing::hash(&encoded_proposal[..]);
		let block_number: T::BlockNumber = 0.into();
		let referendum_index: ReferendumIndex = 0u32.into(); 
		<DispatchQueue<T>>::put(vec![(block_number, proposal_hash, referendum_index)]);

	}: _(RawOrigin::Signed(caller), encoded_proposal)

	reap_preimage {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		T::Currency::make_free_balance_be(&caller, BalanceOf::<T>::max_value());

		let mut encoded_proposal = vec![];
		for i in 0 .. u {
			encoded_proposal.push(Default::default());
		}

		Democracy::<T>::note_preimage(RawOrigin::Signed(caller.clone()).into(), encoded_proposal.clone())?;

		// We need to set this otherwise we get `Early` error.
		let block_number = T::VotingPeriod::get();
		System::<T>::set_block_number(block_number.into());

		let proposal_hash = T::Hashing::hash(&encoded_proposal[..]);

	}: _(RawOrigin::Signed(caller), proposal_hash)

	unlock {
		let u in ...;

		let caller: T::AccountId = account("caller", u, SEED);
		let locked_until: T::BlockNumber = 0u32.into();
		Locks::<T>::insert(&caller, locked_until);
		T::Currency::extend_lock(
			DEMOCRACY_ID,
			&caller,
			Bounded::max_value(),
			WithdrawReason::Transfer.into()
		);

		let other = caller.clone();

	}: _(RawOrigin::Signed(caller), other)
}