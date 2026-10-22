use soroban_sdk::contracterror;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum VoteEscrowError {
    NotInitialized = 1,
    LockNotFound = 2,
    LockNotMatured = 3,
    LockAlreadyWithdrawn = 4,
    InvalidDuration = 5,
    InvalidAmount = 6,
    InvalidEndLedger = 7,
    ArithmeticOverflow = 8,
    Unauthorized = 9,
}
