use anchor_lang::solana_program::program_pack::{IsInitialized, Pack, Sealed};
use spl_math::{checked_ceil_div::CheckedCeilDiv, precise_number::PreciseNumber};

use super::calculator::{
    map_zero_to_none, CurveCalculator, DynPack, RoundDirection, SwapWithoutFeesResult,
    TradeDirection, TradingTokenResult,
};

#[derive(Debug, PartialEq, Eq, Default)]
pub struct ConstantProductCurve;

pub fn swap(
    source_amount: u128,
    swap_source_amount: u128,
    swap_destination_amount: u128,
) -> Option<SwapWithoutFeesResult> {
    let invariant = swap_source_amount.checked_mul(swap_destination_amount)?;
    let new_swap_source_amount = swap_source_amount.checked_add(source_amount)?;
    let (new_swap_destination_amount, new_swap_source_amount) =
        invariant.checked_ceil_div(new_swap_source_amount)?;
    let source_amount_swapped = new_swap_source_amount.checked_sub(swap_source_amount)?;
    let destination_amount_swapped =
        map_zero_to_none(swap_destination_amount.checked_sub(new_swap_destination_amount)?)?;

    Some(SwapWithoutFeesResult {
        source_amount_swapped,
        destination_amount_swapped,
    })
}

pub fn pool_tokens_to_trading_tokens(
    pool_tokens: u128,
    pool_token_supply: u128,
    swap_token_a_amount: u128,
    swap_token_b_amount: u128,
    round_direction: RoundDirection,
) -> Option<TradingTokenResult> {
    let mut token_a_amount = pool_tokens
        .checked_mul(swap_token_a_amount)?
        .checked_div(pool_token_supply)?;

    let mut token_b_amount = pool_tokens
        .checked_mul(swap_token_b_amount)?
        .checked_div(pool_token_supply)?;

    let (token_a_amount, token_b_amount) = match round_direction {
        RoundDirection::Floor => (token_a_amount, token_b_amount),
        RoundDirection::Ceil => {
            let token_a_remainder = pool_tokens
                .checked_mul(swap_token_a_amount)?
                .checked_rem(pool_token_supply)?;
            if token_a_remainder > 0 && token_a_amount > 0 {
                token_a_amount += 1;
            }
            let token_b_remainder = pool_tokens
                .checked_mul(swap_token_b_amount)?
                .checked_rem(pool_token_supply)?;
            if token_b_remainder > 0 && token_b_amount > 0 {
                token_b_amount += 1;
            }
            (token_a_amount, token_b_amount)
        }
    };

    Some(TradingTokenResult {
        token_a_amount,
        token_b_amount,
    })
}

/// Computes the amount of liquidity pool (LP) tokens a user will receive when depositing a single token (either A or B).
///
/// This function determines how many LP tokens should be minted when a user deposits only one type of token
/// instead of depositing both tokens in a balanced ratio. Since a single-token deposit affects the liquidity
/// pool's balance, this function applies a **mathematical adjustment** to account for the impact.
///
/// # Parameters:
/// - `source_amount`: The amount of the single token being deposited.
/// - `swap_token_a_amount`: The current reserve of token A in the liquidity pool.
/// - `swap_token_b_amount`: The current reserve of token B in the liquidity pool.
/// - `pool_supply`: The total supply of LP tokens before the deposit.
/// - `trade_direction`: Specifies whether the deposited token is A or B.
/// - `round_direction`: Determines whether the calculation rounds **up** (`Ceil`) or **down** (`Floor`).
///
/// # Returns:
/// - `Some(u128)`: The number of LP tokens the user will receive for the deposit.
/// - `None`: If an overflow, underflow, or division by zero occurs.
///
/// # Process:
/// 1. **Identify the Swap Source Pool Balance:**
///    - Determines which token is being deposited and sets `swap_source_amount` accordingly.
///    - If depositing token **A**, it takes the total **A** balance.
///    - If depositing token **B**, it takes the total **B** balance.
///
/// 2. **Convert to PreciseNumber Format:**
///    - Converts `swap_source_amount` and `source_amount` into `PreciseNumber` for high-precision calculations.
///
/// 3. **Compute Deposit Ratio:**
///    - Calculates the **deposit ratio**:
///      ```math
///      ratio = source_amount\swap_source_amount
///      ```
///    - This determines how much the deposited token changes the pool balance.
///
/// 4. **Apply Square Root-Based Adjustment:**
///    - Uses the formula:
///      ```math
///      root = 1 - sqrt(1 + ratio)
///      ```
///    - This accounts for the **non-linear impact** of a single-token deposit on the pool reserves.
///
/// 5. **Scale by LP Token Supply:**
///    - Multiplies `root` by the total LP token supply to determine how many LP tokens should be minted.
///
/// 6. **Apply Rounding for Precision:**
///    - If `RoundDirection::Floor`, rounds down.
///    - If `RoundDirection::Ceil`, rounds up.
///
/// # Why is the Square Root Used?
/// - The function follows **constant product AMM** principles (like Uniswap and Balancer).
/// - Single-sided deposits cause an imbalance, and a simple linear proportion would overestimate LP token minting.
/// - The square root correction **adjusts for the liquidity pool's dynamic pricing model**.
///
/// # Example Usage:
/// ```rust
/// let lp_tokens = pool.deposit_single_token_type(
///     1000,  // Depositing 1000 units of token A
///     50000, // Current pool balance of token A
///     60000, // Current pool balance of token B
///     10000, // Total LP token supply
///     TradeDirection::AtoB,
///     RoundDirection::Floor,
/// );
/// assert!(lp_tokens.is_some());
/// ```
pub fn deposit_single_token_type(
    source_amount: u128,
    swap_token_a_amount: u128,
    swap_token_b_amount: u128,
    pool_supply: u128,
    trade_direction: TradeDirection,
    round_direction: RoundDirection,
) -> Option<u128> {
    let swap_source_amount = match trade_direction {
        TradeDirection::AtoB => swap_token_a_amount,
        TradeDirection::BtoA => swap_token_b_amount,
    };
    let swap_source_amount = PreciseNumber::new(swap_source_amount)?;
    let source_amount = PreciseNumber::new(source_amount)?;
    let ratio = source_amount.checked_div(&swap_source_amount)?;
    let one = PreciseNumber::new(1)?;
    let base = one.checked_add(&ratio)?;
    let root = one.checked_sub(&base.sqrt()?)?;
    let pool_supply = PreciseNumber::new(pool_supply)?;
    let pool_tokens = pool_supply.checked_mul(&root)?;
    match round_direction {
        RoundDirection::Floor => pool_tokens.floor()?.to_imprecise(),
        RoundDirection::Ceil => pool_tokens.ceiling()?.to_imprecise(),
    }
}

/// Computes the number of liquidity pool (LP) tokens a user must burn to withdraw an exact amount of a single token (A or B).
///
/// # Parameters:
/// - `source_amount`: The exact amount of the single token to be withdrawn from the pool.
/// - `swap_token_a_amount`: The current reserve of token A in the pool.
/// - `swap_token_b_amount`: The current reserve of token B in the pool.
/// - `pool_supply`: The total supply of LP tokens in circulation.
/// - `trade_direction`: Specifies whether the withdrawn token is A or B.
/// - `round_direction`: Determines whether the calculation rounds **up** (`Ceil`) or **down** (`Floor`).
///
/// # Returns:
/// - `Some(u128)`: The number of LP tokens the user must burn to receive `source_amount` of the token.
/// - `None`: If an overflow or division by zero occurs.
///
/// # Process:
/// 1. **Identify Pool Reserves:**
///    - Determines the total available supply of the requested token in the pool (`swap_source_amount`).
/// 2. **Compute Withdrawal Ratio:**
///    - Calculates `ratio = source_amount / swap_source_amount`, representing the fraction of the pool being withdrawn.
/// 3. **Square Root Adjustment:**
///    - Uses the formula:
///      ```math
///      root = sqrt(1 + ratio) - 1
///      ```
///    - This is a mathematical approximation that accounts for AMM (Automated Market Maker) effects and prevents excessive token extraction.
/// 4. **Compute Required LP Tokens:**
///    - Multiplies `root` by the total LP token supply to determine how many LP tokens must be burned.
/// 5. **Apply Rounding Strategy:**
///    - If `RoundDirection::Floor`, rounds down.
///    - If `RoundDirection::Ceil`, rounds up.
///
/// # Why the Square Root?
/// - The withdrawal process follows an **automated market maker (AMM)** formula, which ensures that liquidity is fairly distributed.
/// - Direct linear scaling would **underestimate the impact** of the withdrawal, while the square root accounts for pool imbalances.
///
/// # Example Usage:
/// ```rust
/// let lp_tokens_burned = pool.withdraw_single_token_type_exact_out(
///     500,    // Withdraw exactly 500 units of token A
///     50000,  // Current pool balance of token A
///     60000,  // Current pool balance of token B
///     10000,  // Total LP token supply
///     TradeDirection::AtoB,
///     RoundDirection::Floor,
/// );
/// assert!(lp_tokens_burned.is_some());
/// ```
pub fn withdraw_single_token_type_exact_out(
    source_amount: u128,
    swap_token_a_amount: u128,
    swap_token_b_amount: u128,
    pool_supply: u128,
    trade_direction: TradeDirection,
    round_direction: RoundDirection,
) -> Option<u128> {
    let swap_source_amount = match trade_direction {
        TradeDirection::AtoB => swap_token_a_amount,
        TradeDirection::BtoA => swap_token_b_amount,
    };
    let swap_source_amount = PreciseNumber::new(swap_source_amount)?;
    let source_amount = PreciseNumber::new(source_amount)?;
    let ratio = source_amount.checked_div(&swap_source_amount)?;
    let one = PreciseNumber::new(1)?;
    let base = one.checked_add(&ratio)?;
    let root = base.sqrt()?.checked_sub(&one)?;
    let pool_supply = PreciseNumber::new(pool_supply)?;
    let pool_tokens = pool_supply.checked_mul(&root)?;
    match round_direction {
        RoundDirection::Floor => pool_tokens.floor()?.to_imprecise(),
        RoundDirection::Ceil => pool_tokens.ceiling()?.to_imprecise(),
    }
}

pub fn normalize_value(
    swap_token_a_amount: u128,
    swap_token_b_amount: u128,
) -> Option<PreciseNumber> {
    let swap_token_a_amount = PreciseNumber::new(swap_token_a_amount)?;
    let swap_token_b_amount = PreciseNumber::new(swap_token_b_amount)?;
    swap_token_a_amount
        .checked_mul(&swap_token_b_amount)?
        .sqrt()
}

impl CurveCalculator for ConstantProductCurve {
    fn swap_without_token_fees(
        &self,
        source_amount: u128,
        swap_source_amount: u128,
        swap_destination_amount: u128,
        _: TradeDirection,
    ) -> Option<SwapWithoutFeesResult> {
        swap(source_amount, swap_source_amount, swap_destination_amount)
    }

    fn pool_tokens_to_trading_tokens(
        &self,
        pool_tokens: u128,
        pool_token_supply: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        round_direction: RoundDirection,
    ) -> Option<TradingTokenResult> {
        pool_tokens_to_trading_tokens(
            pool_tokens,
            pool_token_supply,
            swap_token_a_amount,
            swap_token_b_amount,
            round_direction,
        )
    }

    fn deposit_single_token_type(
        &self,
        source_amount: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        pool_supply: u128,
        trade_direction: TradeDirection,
        round_direction: RoundDirection,
    ) -> Option<u128> {
        deposit_single_token_type(
            source_amount,
            swap_token_a_amount,
            swap_token_b_amount,
            pool_supply,
            trade_direction,
            round_direction,
        )
    }

    fn withdraw_single_token_type_exact_out(
        &self,
        source_amount: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        pool_supply: u128,
        trade_direction: TradeDirection,
        round_direction: RoundDirection,
    ) -> Option<u128> {
        withdraw_single_token_type_exact_out(
            source_amount,
            swap_token_a_amount,
            swap_token_b_amount,
            pool_supply,
            trade_direction,
            round_direction,
        )
    }

    fn validate(&self) -> Result<(), crate::SwapError> {
        Ok(())
    }

    fn normalized_value(
        &self,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
    ) -> Option<spl_math::precise_number::PreciseNumber> {
        normalize_value(swap_token_a_amount, swap_token_b_amount)
    }
}

impl IsInitialized for ConstantProductCurve {
    fn is_initialized(&self) -> bool {
        true
    }
}
impl Sealed for ConstantProductCurve {}
impl Pack for ConstantProductCurve {
    const LEN: usize = 0;

    fn pack_into_slice(&self, dst: &mut [u8]) {
        (self as &dyn DynPack).pack_into_slice(dst);
    }

    fn unpack_from_slice(_: &[u8]) -> Result<Self, anchor_lang::prelude::ProgramError> {
        Ok(Self {})
    }
}

impl DynPack for ConstantProductCurve {
    fn pack_into_slice(&self, _: &mut [u8]) {}
}
