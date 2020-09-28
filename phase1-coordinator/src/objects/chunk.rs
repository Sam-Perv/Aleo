use crate::{
    objects::{Contribution, Participant},
    CoordinatorError,
};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_aux::prelude::*;
use serde_diff::SerdeDiff;
use tracing::trace;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, SerdeDiff)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    chunk_id: u64,
    lock_holder: Option<Participant>,
    #[serde_diff(opaque)]
    contributions: Vec<Contribution>,
}

impl Chunk {
    ///
    /// Creates a new instance of `Chunk`.
    ///
    /// Checks that the given participant is a verifier,
    /// as this function is intended for use to initialize
    /// a new round by the coordinator.
    ///
    /// This function creates one contribution with a
    /// contribution ID of `0`.
    ///
    #[inline]
    pub fn new(chunk_id: u64, participant: Participant, verifier_locator: String) -> Result<Self, CoordinatorError> {
        match participant.is_verifier() {
            // Construct the starting contribution template for this chunk.
            true => Ok(Self {
                chunk_id,
                lock_holder: None,
                contributions: vec![Contribution::new_verifier(0, participant, verifier_locator)?],
            }),
            false => Err(CoordinatorError::ExpectedVerifier),
        }
    }

    /// Returns the assigned chunk ID.
    #[inline]
    pub fn chunk_id(&self) -> u64 {
        self.chunk_id
    }

    /// Returns a reference to the current contribution in this chunk,
    /// irrespective of the state of the contribution.
    #[inline]
    pub fn current_contribution(&self) -> Result<&Contribution, CoordinatorError> {
        self.get_contribution(self.current_contribution_id())
    }

    /// Returns a reference to a contribution given a contribution ID.
    #[inline]
    pub fn get_contribution(&self, contribution_id: u64) -> Result<&Contribution, CoordinatorError> {
        match self.contributions.get(contribution_id as usize) {
            Some(contribution) => Ok(contribution),
            _ => Err(CoordinatorError::ContributionMissing),
        }
    }

    /// Returns a mutable reference to a contribution given a contribution ID.
    #[inline]
    pub fn get_contribution_mut(&mut self, contribution_id: u64) -> Result<&mut Contribution, CoordinatorError> {
        match self.contributions.get_mut(contribution_id as usize) {
            Some(contribution) => Ok(contribution),
            _ => Err(CoordinatorError::ContributionMissing),
        }
    }

    /// Returns a reference to a list of contributions in this chunk.
    #[inline]
    pub fn get_contributions(&self) -> &Vec<Contribution> {
        &self.contributions
    }

    ///
    /// Returns the current number of contributions in this chunk.
    ///
    /// This function does NOT consider the state of the current contribution.
    ///
    #[inline]
    pub fn current_contribution_id(&self) -> u64 {
        (self.contributions.len() - 1) as u64
    }

    ///
    /// Returns `true` if the given next contribution ID is valid, based on the
    /// given expected number of contributions as a basis for computing it.
    ///
    /// This function does NOT consider the *verified status* of the current contribution.
    ///
    /// If the contributions are complete, returns `false`.
    ///
    #[inline]
    pub fn is_next_contribution_id(&self, next_contribution_id: u64, expected_contributions: u64) -> bool {
        // Check that the current and next contribution ID differ by 1.
        let current_contribution_id = self.current_contribution_id();
        if current_contribution_id + 1 != next_contribution_id {
            return false;
        }

        // Check if the contributions for this chunk are complete.
        if self.only_contributions_complete(expected_contributions) {
            return false;
        }

        true
    }

    ///
    /// Returns the next number of contributions in this chunk.
    ///
    /// This function uses the given expected number of contributions
    /// as a basis for determining the next contribution ID.
    ///
    /// This function should only be used to get the next contribution ID when
    /// the next contribution ID is ready to be computed. It should not be used
    /// for polling or querying, as it is too restrictive for those purposes.
    ///
    /// If the current contribution is not verified, returns `CoordinatorError`.
    /// If the contributions are complete, returns `CoordinatorError`.
    ///
    #[inline]
    pub fn next_contribution_id(&self, expected_contributions: u64) -> Result<u64, CoordinatorError> {
        // Check if the current contribution is verified.
        if !self.current_contribution()?.is_verified() {
            return Err(CoordinatorError::ContributionMissingVerification);
        }
        // Check if all contributions for this chunk are present.
        match !self.only_contributions_complete(expected_contributions) {
            true => Ok(self.current_contribution_id() + 1),
            false => Err(CoordinatorError::ContributionsComplete),
        }
    }

    ///
    /// Returns `true` if the current number of contributions in this chunk
    /// matches the given expected number of contributions. Otherwise,
    /// returns `false`.
    ///
    /// Note that this does NOT mean the contributions in this chunk have
    /// been verified. To account for that, use `Chunk::is_complete`.
    ///
    #[inline]
    pub fn only_contributions_complete(&self, expected_contributions: u64) -> bool {
        (self.contributions.len() as u64) == expected_contributions
    }

    ///
    /// Returns `true` if the current contributions in this chunk have been verified.
    /// Otherwise, returns `false`.
    ///
    /// Note that this does NOT mean all expected contributions in this chunk
    /// are present and have have been verified.  To account for that,
    /// use `Chunk::is_complete`.
    ///
    #[inline]
    pub fn only_current_verifications_complete(&self) -> bool {
        self.get_contributions()
            .par_iter()
            .filter(|contribution| !contribution.is_verified())
            .count() as u64
            == 0
    }

    ///
    /// Returns `true` if the given expected number of contributions for
    /// this chunk is complete and all contributions have been verified.
    /// Otherwise, returns `false`.
    ///
    #[inline]
    pub fn is_complete(&self, expected_contributions: u64) -> bool {
        let contributions_complete = self.only_contributions_complete(expected_contributions);
        let verifications_complete = (self
            .get_contributions()
            .par_iter()
            .filter(|contribution| contribution.is_verified())
            .count() as u64)
            == expected_contributions;

        trace!(
            "Chunk {} contributions complete ({}) and verifications complete ({})",
            self.chunk_id(),
            contributions_complete,
            verifications_complete
        );

        contributions_complete && verifications_complete
    }

    /// Returns `true` if this chunk is locked. Otherwise, returns `false`.
    #[inline]
    pub fn is_locked(&self) -> bool {
        self.lock_holder.is_some()
    }

    /// Returns `true` if this chunk is unlocked. Otherwise, returns `false`.
    #[inline]
    pub fn is_unlocked(&self) -> bool {
        !self.is_locked()
    }

    /// Returns `true` if this chunk is locked by the given participant.
    /// Otherwise, returns `false`.
    #[inline]
    pub fn is_locked_by(&self, participant: &Participant) -> bool {
        // Retrieve the current lock holder, or return `false` if the chunk is unlocked.
        match &self.lock_holder {
            Some(lock_holder) => *lock_holder == *participant,
            None => false,
        }
    }

    ///
    /// Attempts to acquire the lock for the given participant.
    ///
    /// If the chunk is locked already, returns a `CoordinatorError`.
    ///
    /// If the chunk is already complete, returns a `CoordinatorError`.
    ///
    /// If the participant is a contributor, check that they have not
    /// contributed to this chunk before and that the current contribution
    /// is already verified.
    ///
    /// If the participant is a verifier, check that the current contribution
    /// has not been verified yet.
    ///
    #[inline]
    pub(crate) fn acquire_lock(
        &mut self,
        participant: Participant,
        num_contributors: u64,
    ) -> Result<(), CoordinatorError> {
        // Check that this chunk is not locked before attempting to acquire the lock.
        if self.is_locked() {
            return Err(CoordinatorError::ChunkLockAlreadyAcquired);
        }

        // Check that this chunk is still incomplete before attempting to acquire the lock.
        if self.is_complete(num_contributors) {
            trace!("{} {:#?}", num_contributors, self);
            return Err(CoordinatorError::ChunkAlreadyComplete);
        }

        // If the participant is a contributor, check that they have not already contributed to this chunk before.
        if participant.is_contributor() {
            // Fetch all contributions with this contributor ID.
            let matches: Vec<_> = self
                .contributions
                .par_iter()
                .filter(|contribution| *contribution.get_contributor() == Some(participant.clone()))
                .collect();
            if !matches.is_empty() {
                return Err(CoordinatorError::ContributorAlreadyContributed);
            }

            // If the lock is currently held by this participant,
            // the current contributor ID is this contributor,
            // the current contributed location is empty,
            // and the current contribution is not verified,
            // then it could mean this contributor lost their
            // connection and is attempting to reconnect.
            //
            // In this case, no further action needs to be taken,
            // and we may return true.
            let contribution = self.current_contribution()?;
            if self.is_locked_by(&participant)
                && *contribution.get_contributor() == Some(participant.clone())
                && contribution.get_contributed_location().is_none()
                && !contribution.is_verified()
            {
                return Ok(());
            }

            // Check that the current contribution in this chunk has been verified.
            if !self.current_contribution()?.is_verified() {
                return Err(CoordinatorError::ChunkMissingVerification);
            }
        }

        // If the participant is a verifier, check that they have not already contributed to this chunk before.
        if participant.is_verifier() {
            // Check that the current contribution in this chunk has NOT been verified.
            if self.current_contribution()?.is_verified() {
                return Err(CoordinatorError::ChunkAlreadyVerified);
            }
        }

        // Set the lock holder as the participant.
        self.lock_holder = Some(participant);
        Ok(())
    }

    ///
    /// Attempts to add a new contribution from a contributor to this chunk.
    /// Upon success, releases the lock on this chunk to allow a verifier to
    /// check the contribution for correctness.
    ///
    /// This function is intended to be used only by an authorized contributor
    /// currently holding the lock on this chunk.
    ///
    /// If the operations succeed, returns `Ok(())`. Otherwise, returns `CoordinatorError`.
    ///
    #[inline]
    pub fn add_contribution(
        &mut self,
        contribution_id: u64,
        participant: Participant,
        contributed_locator: String,
        expected_contributions: u64,
    ) -> Result<(), CoordinatorError> {
        // Check that the participant is a contributor.
        if !participant.is_contributor() {
            return Err(CoordinatorError::ExpectedContributor);
        }

        // Check that this chunk is locked by the contributor before attempting to add the contribution.
        if !self.is_locked_by(&participant) {
            return Err(CoordinatorError::ChunkNotLockedOrByWrongParticipant);
        }

        // Check that the contribution ID is one above the current contribution ID.
        if self.is_next_contribution_id(contribution_id, expected_contributions) {
            return Err(CoordinatorError::ContributionIdMismatch);
        }

        // Add the contribution to this chunk.
        self.contributions
            .push(Contribution::new_contributor(participant, contributed_locator)?);

        // Release the lock on this chunk from the contributor.
        self.lock_holder = None;

        Ok(())
    }

    ///
    /// Updates the contribution corresponding to the given contribution ID as verified.
    ///
    /// This function is intended to be called by an authorized verifier
    /// holding a lock on the chunk.
    ///
    /// The underlying function checks that the contribution has a verifier assigned to it.
    ///
    #[inline]
    pub fn verify_contribution(
        &mut self,
        contribution_id: u64,
        participant: &Participant,
    ) -> Result<(), CoordinatorError> {
        // Check that the participant is a verifier.
        if !participant.is_verifier() {
            return Err(CoordinatorError::ExpectedVerifier);
        }

        // Check that this chunk is locked by the verifier before attempting to verify contribution.
        if !self.is_locked_by(participant) {
            return Err(CoordinatorError::ChunkNotLockedOrByWrongParticipant);
        }

        // Fetch the contribution to be verified from the chunk.
        let mut contribution = self.get_contribution_mut(contribution_id)?;

        // Attempt to verify the contribution.
        match contribution.is_verified() {
            // Case 1 - Check that the contribution is not verified yet.
            true => Err(CoordinatorError::ContributionAlreadyVerified),
            // Case 2 - If the contribution is not verified, attempt to set it to verified.
            false => {
                contribution.try_verify(participant)?;

                // Release the lock on this chunk from the verifier.
                self.lock_holder = None;

                trace!("Verification of contribution {} succeeded", contribution_id);
                Ok(())
            }
        }
    }

    /// Generates an unique number representing the current state of the chunk.
    #[inline]
    pub fn version(&self) -> u32 {
        let matching_contributions: u32 = self
            .contributions
            .par_iter()
            .map(|contribution| contribution.get_contributor().is_some() as u32)
            .sum();
        let matching_verifications: u32 = self
            .contributions
            .par_iter()
            .map(|contribution| contribution.get_verifier().is_some() as u32)
            .sum();
        return matching_contributions + matching_verifications;
    }
}

#[cfg(test)]
mod tests {
    // use crate::testing::prelude::*;

    // #[test]
    // fn test_update_chunk() {
    //     let mut expected = test_round_1_json().unwrap().chunks[0].clone();
    //     expected.acquire_lock("test_updated_contributor");
    //
    //     let candidate = test_round_1().unwrap().get_chunk_mut(0).unwrap();
    //     assert!(candidate.update_chunk(0, &expected));
    //     assert_eq!(expected, *candidate);
    // }
}
