mod aux_schema;

use std::{
	sync::Arc, marker::PhantomData, time::{Duration, Instant}, collections::HashMap,
};
use log::trace;
use codec::Encode;
use parking_lot::Mutex;
use merlin::Transcript;
use sp_core::{Blake2Hasher, H256, crypto::{Pair, Public}};
use sp_blockchain::{Result as ClientResult, ProvideCache, HeaderMetadata};
use sp_inherents::InherentData;
use sp_timestamp::{TimestampInherentData, InherentType as TimestampInherent};
use sp_consensus::{
	Error as ConsensusError, BlockImportParams, BlockOrigin, ForkChoiceStrategy,
	ImportResult, BlockImport,
};
use sp_consensus::import_queue::{Verifier, CacheKeyId, BasicQueue};
use sp_consensus_sassafras::{
	SASSAFRAS_ENGINE_ID, AuthorityPair, AuthorityId, Randomness, VRFProof,
	SassafrasAuthorityWeight,
};
use sp_consensus_sassafras::digest::{
	NextEpochDescriptor, PostBlockDescriptor, PreDigest, CompatibleDigestItem
};
use sp_consensus_sassafras::inherents::SassafrasInherentData;
use sp_runtime::{generic::BlockId, Justification};
use sp_runtime::traits::{Block as BlockT, Header};
use sp_api::ProvideRuntimeApi;
use sp_block_builder::BlockBuilder as BlockBuilderApi;
use sc_client::{Client, CallExecutor};
use sc_client_api::backend::{AuxStore, Backend};
use sc_consensus_epochs::{SlotNumber, Epoch as EpochT};
use sc_consensus_slots::SlotCompatible;
use crate::aux_schema::{load_epoch_changes, write_epoch_changes};

/// Validator set of a particular epoch, can be either publishing or validating.
pub struct ValidatorSet {
	/// Proofs of all VRFs collected.
	pub proofs: Vec<VRFProof>,
	/// The authorities and their weights.
	pub authorities: Vec<(AuthorityId, SassafrasAuthorityWeight)>,
	/// Randomness for this epoch.
	pub randomness: Randomness,
}

/// Epoch data for Sassafras
pub struct Epoch {
	/// Start slot of the epoch.
	pub start_slot: SlotNumber,
	/// Duration of this epoch.
	pub duration: SlotNumber,
	/// Epoch index.
	pub epoch_index: u64,

	/// Publishing validator set. The set will start validating block in the next epoch.
	pub publishing: ValidatorSet,
	/// Validating validator set. The set validates block in the current epoch.
	pub validating: ValidatorSet,
}

impl EpochT for Epoch {
	type NextEpochDescriptor = NextEpochDescriptor;

	fn increment(&self, descriptor: NextEpochDescriptor) -> Epoch {
		Epoch {
			epoch_index: self.epoch_index + 1,
			start_slot: self.start_slot + self.duration,
			duration: self.duration,

			validating: self.publishing.clone(),
			publishing: ValidatorSet {
				proofs: Vec::new(),
				authorities: descriptor.authorities,
				randomness: descriptor.randomness,
			},
		}
	}

	fn start_slot(&self) -> SlotNumber {
		self.start_slot
	}

	fn end_slot(&self) -> SlotNumber {
		self.start_slot + self.duration
	}
}

#[derive(derive_more::Display, Debug)]
enum Error<B: BlockT> {
	#[display(fmt = "Could not extract timestamp and slot: {:?}", _0)]
	Extraction(sp_consensus::Error),
	#[display(fmt = "Header {:?} rejected: too far in the future", _0)]
	TooFarInFuture(B::Hash),
	#[display(fmt = "Parent ({}) of {} unavailable. Cannot import", _0, _1)]
	ParentUnavailable(B::Hash, B::Hash),
	#[display(fmt = "Could not fetch parent header: {:?}", _0)]
	FetchParentHeader(sp_blockchain::Error),
	MultiplePreRuntimeDigest,
	NoPreRuntimeDigest,
	MultipleNextEpochDescriptor,
	MultiplePostBlockDescriptor,
	InvalidTicketVRFIndex,
	InvalidAuthorityId,
	InvalidSeal,
	HeaderUnsealed(B::Hash),
	TicketVRFVerificationFailed,
	PostVRFVerificationFailed,
	SlotInPast,
	SlotInFuture,
	Runtime(sp_inherents::Error),
	Client(sp_blockchain::Error),
}

impl<B: BlockT> std::convert::From<Error<B>> for String {
	fn from(error: Error<B>) -> String {
		error.to_string()
	}
}

pub struct SassafrasVerifier<B, E, Block: BlockT, RA, PRA> {
	client: Arc<Client<B, E, Block, RA>>,
	api: Arc<PRA>,
	inherent_data_providers: sp_inherents::InherentDataProviders,
	time_source: TimeSource,
}

impl<B, E, Block, RA, PRA> SassafrasVerifier<B, E, Block, RA, PRA> where
	Block: BlockT<Hash=H256>,
	B: Backend<Block> + 'static,
	E: CallExecutor<Block> + 'static + Clone + Send + Sync,
	RA: Send + Sync,
	PRA: ProvideRuntimeApi<Block> + Send + Sync + AuxStore + ProvideCache<Block>,
	PRA::Api: BlockBuilderApi<Block, Error = sp_blockchain::Error>,
{
	fn verify(
		&mut self,
		origin: BlockOrigin,
		mut header: Block::Header,
		justification: Option<Justification>,
		mut body: Option<Vec<Block::Extrinsic>>,
	) -> Result<(BlockImportParams<Block>, Option<Vec<(CacheKeyId, Vec<u8>)>>), Error<Block>> {
		trace!(
			target: "sassafras",
			"Verifying origin: {:?} header: {:?} justification: {:?} body: {:?}",
			origin,
			header,
			justification,
			body,
		);

		let mut inherent_data = self
			.inherent_data_providers
			.create_inherent_data()
			.map_err(Error::Runtime)?;

		let (_, slot_now, _) = self.time_source.extract_timestamp_and_slot(&inherent_data)
			.map_err(Error::Extraction)?;

		let hash = header.hash();
		let parent_hash = *header.parent_hash();

		let parent_header_metadata = self.client.header_metadata(parent_hash)
			.map_err(Error::FetchParentHeader)?;

		// First, Verify pre-runtime digest.
		let pre_digest = find_pre_digest::<Block>(&header)?;

		if pre_digest.slot > slot_now {
			return Err(Error::SlotInFuture.into())
		}

		// Check the signature.
		let (author, block_weight) = auxiliary.validating.authorities
			.get(pre_digest.authority_index as usize)
			.cloned()
			.ok_or(Error::InvalidAuthorityId)?;
		let seal = header.digest_mut().pop()
			.ok_or(Error::HeaderUnsealed(header.hash()))?;
		let signature = seal.as_sassafras_seal().ok_or(Error::InvalidSeal)?;
		let pre_hash = header.hash();
		if !AuthorityPair::verify(&signature, pre_hash, &author) {
			return Err(Error::InvalidSeal.into())
		}

		// Check that the ticket VRF is of a valid index in auxiliary.validating.
		let ticket_vrf_proof = auxiliary.validating.proofs
			.get(pre_digest.ticket_vrf_index as usize)
			.cloned()
			.ok_or(Error::InvalidTicketVRFIndex)?;

		// Check that the ticket VRF is valid.
		let ticket_transcript = make_ticket_transcript(
			&auxiliary.validating.randomness,
			pre_digest.slot,
			auxiliary.validating.epoch,
		);
		schnorrkel::PublicKey::from_bytes(author.as_slice()).and_then(|p| {
			p.vrf_verify(ticket_transcript, &pre_digest.ticket_vrf_output, &ticket_vrf_proof)
		}).map_err(|_| Error::TicketVRFVerificationFailed)?;

		// Check that the post-block VRF is valid.
		let post_transcript = make_post_transcript(
			&auxiliary.validating.randomness,
			pre_digest.slot,
			auxiliary.validating.epoch,
		);
		schnorrkel::PublicKey::from_bytes(author.as_slice()).and_then(|p| {
			p.vrf_verify(post_transcript, &pre_digest.post_vrf_output, &pre_digest.post_vrf_proof)
		}).map_err(|_| Error::PostVRFVerificationFailed)?;

		// Second, push in any commitments of ticket VRF.
		if let Some(post_block_desc) = find_post_block_descriptor::<Block>(&header)? {
			// TODO: verify that proofs are below threshold.

			auxiliary.publishing.proofs.append(&mut post_block_desc.commitments.clone());
		}

		// Finally, if we are switching epoch, move publishing to validating, and sort the proofs.
		if let Some(next_epoch_desc) = find_next_epoch_descriptor::<Block>(&header)? {
			// TODO: check descriptor validity.

			std::mem::swap(&mut auxiliary.publishing, &mut auxiliary.validating);
			auxiliary.publishing = PoolAuxiliary {
				proofs: Vec::new(),
				authorities: next_epoch_desc.authorities,
				randomness: next_epoch_desc.randomness,
				epoch: auxiliary.validating.epoch + 1,
			};

			// TODO: sort the validating proofs in "outside-in" order.
		}

		let block_import_params = BlockImportParams {
			origin,
			header,
			post_digests: vec![seal],
			body,
			finalized: false,
			justification,
			auxiliary: vec![(AUXILIARY_KEY.to_vec(), Some(auxiliary.encode()))],
			fork_choice: ForkChoiceStrategy::LongestChain,
			allow_missing_state: false,
			import_existing: false,
		};

		Ok((block_import_params, Default::default()))
	}
}

impl<B, E, Block, RA, PRA> Verifier<Block> for SassafrasVerifier<B, E, Block, RA, PRA> where
	Block: BlockT<Hash=H256>,
	B: Backend<Block> + 'static,
	E: CallExecutor<Block> + 'static + Clone + Send + Sync,
	RA: Send + Sync,
	PRA: ProvideRuntimeApi<Block> + Send + Sync + AuxStore + ProvideCache<Block>,
	PRA::Api: BlockBuilderApi<Block, Error = sp_blockchain::Error>,
{
	fn verify(
		&mut self,
		origin: BlockOrigin,
		mut header: Block::Header,
		justification: Option<Justification>,
		mut body: Option<Vec<Block::Extrinsic>>,
	) -> Result<(BlockImportParams<Block>, Option<Vec<(CacheKeyId, Vec<u8>)>>), String> {
		self.verify(origin, header, justification, body).map_err(Into::into)
	}
}

pub type SassafrasImportQueue<B, Transaction> = BasicQueue<B, Transaction>;

pub struct SassafrasBlockImport<B, E, Block: BlockT, I, RA, PRA> {
	inner: I,
	client: Arc<Client<B, E, Block, RA>>,
	api: Arc<PRA>,
}

impl<B, E, Block: BlockT, I, RA, PRA> BlockImport<Block> for
	SassafrasBlockImport<B, E, Block, I, RA, PRA>
where
	I: BlockImport<Block, Transaction = sp_api::TransactionFor<PRA, Block>> + Send + Sync,
	I::Error: Into<ConsensusError>,
	B: Backend<Block> + 'static,
	E: CallExecutor<Block> + 'static + Clone + Send + Sync,
	Client<B, E, Block, RA>: AuxStore,
	RA: Send + Sync,
	PRA: ProvideRuntimeApi<Block> + ProvideCache<Block>,
	PRA::Api: ApiExt<Block, StateBackend = B::State>,
{
	type Error = ConsensusError;
	type Transaction = sp_api::TransactionFor<PRA, Block>;

	fn import_block(
		&mut self,
		mut block: BlockImportParams<Block, Self::Transaction>,
		new_cache: HashMap<CacheKeyId, Vec<u8>>,
	) -> Result<ImportResult, Self::Error> {
		let hash = block.post_header().hash();
		let parent_hash = *block.header.parent_hash();
		let number = block.header.number().clone();

		let mut auxiliary = aux_schema::load_auxiliary(&parent_hash, self.api.as_ref())
			.map_err(Error::Client)?;

		let pre_digest = find_pre_digest::<Block>(&block.header)?;

		// Verify that the slot is increasing, and not in the future.
		if pre_digest.slot <= auxiliary.slot {
			return Err(Error::SlotInPast.into())
		}
		auxiliary.slot = pre_digest.slot;

		let import_result = self.inner.import_block(block, new_cache);

		import_result.map_err(Into::into)
	}
}

#[derive(Default, Clone)]
struct TimeSource(Arc<Mutex<(Option<Duration>, Vec<(Instant, u64)>)>>);

impl SlotCompatible for TimeSource {
	fn extract_timestamp_and_slot(
		&self,
		data: &InherentData,
	) -> Result<(TimestampInherent, u64, std::time::Duration), sp_consensus::Error> {
		trace!(target: "babe", "extract timestamp");
		data.timestamp_inherent_data()
			.and_then(|t| data.sassafras_inherent_data().map(|a| (t, a)))
			.map_err(Into::into)
			.map_err(sp_consensus::Error::InherentData)
			.map(|(x, y)| (x, y, self.0.lock().0.take().unwrap_or_default()))
	}
}

fn find_pre_digest<B: BlockT>(
	header: &B::Header,
) -> Result<PreDigest, Error<B>> {
	let mut pre_digest = None;
	for log in header.digest().logs() {
		match (log.as_sassafras_pre_digest(), pre_digest.is_some()) {
			(Some(_), true) => return Err(Error::MultiplePreRuntimeDigest),
			(None, _) => (),
			(s, false) => pre_digest = s,
		}
	}
	pre_digest.ok_or_else(|| Error::NoPreRuntimeDigest)
}

fn find_post_block_descriptor<B: BlockT>(
	header: &B::Header,
) -> Result<Option<PostBlockDescriptor>, Error<B>> {
	let mut desc = None;
	for log in header.digest().logs() {
		match (log.as_sassafras_post_block_descriptor(), desc.is_some()) {
			(Some(_), true) => return Err(Error::MultiplePostBlockDescriptor),
			(None, _) => (),
			(s, false) => desc = s,
		}
	}
	Ok(desc)
}

fn find_next_epoch_descriptor<B: BlockT>(
	header: &B::Header,
) -> Result<Option<NextEpochDescriptor>, Error<B>> {
	let mut desc = None;
	for log in header.digest().logs() {
		match (log.as_sassafras_next_epoch_descriptor(), desc.is_some()) {
			(Some(_), true) => return Err(Error::MultipleNextEpochDescriptor),
			(None, _) => (),
			(s, false) => desc = s,
		}
	}
	Ok(desc)
}

fn make_ticket_transcript(
	randomness: &[u8],
	slot_number: u64,
	epoch: u64,
) -> Transcript {
	let mut transcript = Transcript::new(&SASSAFRAS_ENGINE_ID);
	transcript.commit_bytes(b"type", b"ticket");
	transcript.commit_bytes(b"slot number", &slot_number.to_le_bytes());
	transcript.commit_bytes(b"current epoch", &epoch.to_le_bytes());
	transcript.commit_bytes(b"chain randomness", randomness);
	transcript
}

fn make_post_transcript(
	randomness: &[u8],
	slot_number: u64,
	epoch: u64,
) -> Transcript {
	let mut transcript = Transcript::new(&SASSAFRAS_ENGINE_ID);
	transcript.commit_bytes(b"type", b"post");
	transcript.commit_bytes(b"slot number", &slot_number.to_le_bytes());
	transcript.commit_bytes(b"current epoch", &epoch.to_le_bytes());
	transcript.commit_bytes(b"chain randomness", randomness);
	transcript
}
