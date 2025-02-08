use spl_math::precise_number::PreciseNumber;
use std::fmt::Debug;

use crate::SwapError;

/// Initial amount of pool tokens for swap contract, hard-coded to something
/// "sensible" given a maximum of u128.
/// Note that on Ethereum, Uniswap uses the geometric mean of all provided
/// input amounts, and Balancer uses 100 * 10 ^ 18.
pub const INITIAL_SWAP_POOL_AMOUNT: u128 = 1_000_000_000;

/// Hardcode the number of token types in a pool, used to calculate the
/// equivalent pool tokens for the owner trading fee.
pub const TOKENS_IN_POOL: u128 = 2;

pub fn map_zero_to_none(x: u128) -> Option<u128> {
    if x == 0 {
        None
    } else {
        Some(x)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TradeDirection {
    AtoB,
    BtoA,
}

impl TradeDirection {
    fn oppsoite(&self) -> Self {
        match self {
            Self::AtoB => Self::BtoA,
            Self::BtoA => Self::AtoB,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RoundDirection {
    Ceil,
    Floor,
}

#[derive(Debug, PartialEq)]
pub struct SwapWithoutFeesResult {
    /// Amount of source token swapped
    pub source_amount_swapped: u128,
    /// Amount of destination token swapped
    pub destination_amount_swapped: u128,
}

#[derive(Debug, PartialEq)]
pub struct TradingTokenResult {
    pub token_a_amount: u128,
    pub token_b_amount: u128,
}

pub trait DynPack {
    fn pack_into_slice(&self, dst: &mut [u8]);
}

pub trait CurveCalculator: Debug + DynPack {
    fn swap_without_token_fees(
        &self,
        source_amount: u128,
        swap_source_amount: u128,
        swap_destination_amount: u128,
        trade_direction: TradeDirection,
    ) -> Option<SwapWithoutFeesResult>;

    fn new_supply_pool(&self) -> u128 {
        INITIAL_SWAP_POOL_AMOUNT
    }

    fn pool_tokens_to_trading_tokens(
        &self,
        pool_tokens: u128,
        pool_token_supply: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        round_direction: RoundDirection,
    ) -> Option<TradingTokenResult>;

    fn deposit_single_token_type(
        &self,
        source_amount: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        pool_supply: u128,
        trade_direction: TradeDirection,
        round_direction: RoundDirection,
    ) -> Option<u128>;

    fn withdraw_single_token_type_exact_out(
        &self,
        source_amount: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        pool_supply: u128,
        trade_direction: TradeDirection,
        round_direction: RoundDirection,
    ) -> Option<u128>;

    fn validate(&self) -> Result<(), SwapError>;

    fn validate_supply(&self, token_a_amount: u64, token_b_amount: u64) -> Result<(), SwapError> {
        if token_a_amount == 0 {
            return Err(SwapError::EmptySupply.into());
        }
        if token_b_amount == 0 {
            return Err(SwapError::EmptySupply.into());
        }
        Ok(())
    }

    fn allow_deposits(&self) -> bool {
        true
    }

    fn normalized_value(
        &self,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
    ) -> Option<PreciseNumber>;
}
