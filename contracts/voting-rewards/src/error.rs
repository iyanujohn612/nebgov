use soroban_sdk::contracterror;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum VotingRewardsError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAuthorized = 3,
    InvalidEpochDuration = 4,
    EpochNotFound = 5,
    EpochNotEnded = 6,
    EpochAlreadyFinalized = 7,
    EpochNotFinalized = 8,
    InsufficientPool = 9,
    InvalidAmount = 10,
    AlreadyClaimed = 11,
    InvalidProof = 12,
    EpochOverclaimed = 13,
    EpochOverflow = 14,
    EpochAlreadySwept = 15,
}
