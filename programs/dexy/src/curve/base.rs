use super::{
    calculator::{CurveCalculator, RoundDirection, SwapWithoutFeesResult, TradeDirection},
    constant_product::ConstantProductCurve,
    fees::CurveFees,
};
use anchor_lang::{
    prelude::ProgramError,
    solana_program::program_pack::{Pack, Sealed},
};
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};

/// # Curve Types
///
/// ## ConstantProduct (x * y = k)
/// The traditional Uniswap-style constant product curve.
/// ```text
/// y │
///   │    ╭────────────────
///   │    │
///   │    │
///   │    │      k = x * y
///   │    │
///   │    │
///   │    │
///   └────┴───────────────── x
/// ```
///
/// ## ConstantPrice (y = mx)
/// Linear price curve where price remains constant.
/// ```text
/// y │           ╱
///   │         ╱
///   │       ╱
///   │     ╱     y = mx
///   │   ╱       (m = price)
///   │ ╱
///   │╱
///   └────────────────────── x
/// ```
///
/// ## ConstantProductWithOffset (k = (x + offset_x)(y + offset_y))
/// Modified constant product curve with price concentration.
/// ```text
/// y │
///   │    ╭────────────────
///   │    │
///   │    │offset_y
///   │    ├──┐   k = (x + offset_x)(y + offset_y)
///   │    │  │
///   │    │  │
///   │    │  │
///   └────┴──┴─────────────── x
///        offset_x
/// ```
///
/// # Implementation Notes
///
/// - ConstantProduct: Best for most general trading pairs
/// - ConstantPrice: Useful for stable pairs (e.g. USDC/USDT)
/// - ConstantProductWithOffset: Helps concentrate liquidity in a specific price range
///
/// # Mathematical Formulas
///
/// - ConstantProduct: k = x * y
/// - ConstantPrice: y = m * x
/// - ConstantProductWithOffset: k = (x + a)(y + b) where a,b are offsets
///
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurveType {
    /// Standard constant product curve (Uniswap V2 style)
    /// Formula: x * y = k
    ConstantProduct,

    // TODO: Implement the following curve types
    /// Constant price curve (stable swap)
    /// Formula: y = mx where m is the price
    ConstantPrice,

    // TODO: Implement the following curve types
    /// Constant product curve with offset for concentrated liquidity
    /// Formula: (x + offset_x)(y + offset_y) = k
    ConstantProductWithOffset,
}

impl Default for CurveType {
    fn default() -> Self {
        Self::ConstantProduct
    }
}

impl TryFrom<u8> for CurveType {
    type Error = ProgramError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::ConstantProduct),
            1 => Ok(Self::ConstantPrice),
            2 => Ok(Self::ConstantProductWithOffset),
            _ => Err(ProgramError::InvalidArgument),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct SwapResult {
    /// New amount of source token
    pub new_swap_source_amount: u128,
    /// New amount of destination token
    pub new_swap_destination_amount: u128,
    /// Amount of source token swapped (includes fees)
    pub source_amount_swapped: u128,
    /// Amount of destination token swapped
    pub destination_amount_swapped: u128,
    /// Amount of source tokens going to pool holders
    pub trade_fee: u128,
    /// Amount of source tokens going to owner
    pub owner_fee: u128,
}

#[repr(C)]
#[derive(Debug)]
pub struct SwapCurve {
    pub curve_type: CurveType,
    pub calculator: Box<dyn CurveCalculator>,
}

impl SwapCurve {
    /// Calculate the amount of destination tokens for a given amount of source amount after fee subtraction.
    pub fn swap(
        &self,
        source_amount: u128,
        swap_source_amount: u128,
        swap_destination_amount: u128,
        trade_direction: TradeDirection,
        fees: &CurveFees,
    ) -> Option<SwapResult> {
        // Calcuating Trade Fee
        let trade_fee = fees.trading_fee(source_amount)?;
        // Calculating Owner Fee
        let owner_fee = fees.owner_trading_fee(source_amount)?;
        // Calculating Total Fee
        let total_fee = trade_fee.checked_add(owner_fee)?;

        // Calculating Source Amount Without Fee
        let source_amount_minus_fee = source_amount.checked_sub(total_fee)?;

        let SwapWithoutFeesResult {
            source_amount_swapped,
            destination_amount_swapped,
        } = self.calculator.swap_without_token_fees(
            source_amount_minus_fee,
            swap_source_amount,
            swap_destination_amount,
            trade_direction,
        )?;

        // Total Source Amount Swapped including fees
        let source_amount_swapped = source_amount_swapped.checked_add(total_fee)?;
        Some(SwapResult {
            new_swap_source_amount: swap_source_amount.checked_add(source_amount_swapped)?,
            new_swap_destination_amount: swap_destination_amount
                .checked_sub(destination_amount_swapped)?,
            source_amount_swapped,
            destination_amount_swapped,
            trade_fee,
            owner_fee,
        })
    }

    /// Computes the amount of liquidity pool (LP) tokens to be minted when a user deposits a single token (either A or B) into the pool.
    ///
    /// # Parameters:
    /// - `source_amount`: The amount of the single token being deposited into the liquidity pool.
    /// - `swap_token_a_amount`: The current reserve of token A in the pool.
    /// - `swap_token_b_amount`: The current reserve of token B in the pool.
    /// - `pool_supply`: The total supply of LP tokens before the deposit.
    /// - `trade_direction`: Specifies whether the deposited token is A or B.
    /// - `fees`: A reference to the `CurveFees` structure, which defines the trading fees applied to the deposit.
    ///
    /// # Returns:
    /// - `Some(u128)`: The number of LP tokens the user will receive for the deposit.
    /// - `None`: If an overflow or underflow occurs in the calculations.
    ///
    /// # Process:
    /// 1. **Edge Case Handling:**
    ///    - If `source_amount == 0`, the function returns `Some(0)`, meaning no LP tokens are minted for a zero deposit.
    /// 2. **Estimate Trading Fee:**
    ///    - Since the pool requires a balanced deposit of both token A and B, depositing a single token implicitly assumes half of it is swapped internally.
    ///    - The function estimates the trading fee on this hypothetical swap by taking **half** of `source_amount` (`source_amount / 2`).
    ///    - Ensures `half_source_amount` is at least 1 to avoid division by zero issues.
    /// 3. **Fee Deduction:**
    ///    - Calls `fees.trading_fee(half_source_amount)` to determine the fee.
    ///    - Deducts this fee from the original deposit amount.
    /// 4. **Final Deposit Calculation:**
    ///    - Calls `self.calculator.deposit_single_token_type(...)`, which applies the AMM’s internal logic to compute the LP token minting.
    ///
    /// # Why Half of `source_amount` is Used for Fees:
    /// - The pool assumes half of the deposit will be virtually swapped to balance the reserves.
    /// - This method prevents overcharging fees while maintaining accuracy in estimating the actual contribution to the pool.
    /// - Inspired by Balancer’s liquidity pool fee model.
    ///
    /// # Example Usage:
    /// ```rust
    /// let lp_tokens = pool.deposit_single_token_type(
    ///     1000,  // Depositing 1000 units of token A
    ///     50000, // Current pool balance of token A
    ///     60000, // Current pool balance of token B
    ///     10000, // Total LP token supply
    ///     TradeDirection::AtoB,
    ///     &fees,
    /// );
    /// assert!(lp_tokens.is_some());
    /// ```
    pub fn deposit_single_token_type(
        &self,
        source_amount: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        pool_supply: u128,
        trade_direction: TradeDirection,
        round_direction: RoundDirection,
        fees: &CurveFees,
    ) -> Option<u128> {
        if source_amount == 0 {
            return Some(0);
        }

        let half_source_amount = std::cmp::max(1, source_amount.checked_div(2)?);
        let trade_fee = fees.trading_fee(half_source_amount)?;
        let source_amount = source_amount.checked_sub(trade_fee)?;

        self.calculator.deposit_single_token_type(
            source_amount,
            swap_token_a_amount,
            swap_token_b_amount,
            pool_supply,
            trade_direction,
            round_direction,
        )
    }

    /// Computes the number of liquidity pool (LP) tokens that must be burned to withdraw an exact amount of a single token (either A or B),
    /// while accounting for trading fees.
    ///
    /// This function ensures that liquidity remains balanced by applying a **trading fee adjustment** to the requested withdrawal amount.
    /// It calculates how many LP tokens need to be burned to receive `source_amount` of the requested token.
    ///
    /// # Parameters:
    /// - `source_amount`: The exact amount of the single token (A or B) that the user wants to withdraw.
    /// - `swap_token_a_amount`: The current reserve of token A in the liquidity pool.
    /// - `swap_token_b_amount`: The current reserve of token B in the liquidity pool.
    /// - `pool_supply`: The total supply of LP tokens before the withdrawal.
    /// - `trade_direction`: Specifies whether the withdrawn token is A or B.
    /// - `round_direction`: Determines whether the calculation rounds **up** (`Ceil`) or **down** (`Floor`).
    /// - `fees`: A reference to the `CurveFees` structure, which defines the trading fees applied to the withdrawal.
    ///
    /// # Returns:
    /// - `Some(u128)`: The number of LP tokens that must be burned to withdraw `source_amount` of the specified token.
    /// - `None`: If an overflow, underflow, or division by zero occurs.
    ///
    /// # Process:
    /// 1. **Handle Edge Case:**
    ///    - If `source_amount == 0`, the function immediately returns `Some(0)`, meaning no LP tokens need to be burned.
    ///
    /// 2. **Estimate Trading Fee:**
    ///    - The pool assumes that withdrawing a single token disrupts the balance, similar to swapping half the amount.
    ///    - The function estimates the fee using **half** of `source_amount`:
    ///      ```rust
    ///      let half_source_amount = std::cmp::max(1, source_amount.checked_div(2)?);
    ///      ```
    ///    - The max function ensures that at least 1 token is considered for fee calculation to avoid zero division.
    ///
    /// 3. **Deduct Trading Fee from Requested Amount:**
    ///    - Calls `fees.trading_fee(half_source_amount)` to determine the fee.
    ///    - Deducts this fee from the original `source_amount`, ensuring that the user receives the correct post-fee amount.
    ///
    /// 4. **Calculate Required LP Tokens to Burn:**
    ///    - Calls `self.calculator.deposit_single_token_type(...)`,
    ///      which uses AMM logic to determine how many LP tokens must be burned to withdraw the adjusted `source_amount`.
    ///
    /// # Why is Half of `source_amount` Used for Fees?
    /// - Since the user is withdrawing **only one token**, the pool treats it as if half of the withdrawal is being virtually swapped.
    /// - This approach follows **Balancer's liquidity pool logic**, ensuring the correct amount of LP tokens are burned.
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
    ///     &fees,
    /// );
    /// assert!(lp_tokens_burned.is_some());
    /// ```
    pub fn withdraw_single_token_type_exact_out(
        &self,
        source_amount: u128,
        swap_token_a_amount: u128,
        swap_token_b_amount: u128,
        pool_supply: u128,
        trade_direction: TradeDirection,
        round_direction: RoundDirection,
        fees: &CurveFees,
    ) -> Option<u128> {
        if source_amount == 0 {
            return Some(0);
        }

        let half_source_amount = std::cmp::max(1, source_amount.checked_div(2)?);
        let trade_fee = fees.trading_fee(half_source_amount)?;
        let source_amount = source_amount.checked_sub(trade_fee)?;

        self.calculator.deposit_single_token_type(
            source_amount,
            swap_token_a_amount,
            swap_token_b_amount,
            pool_supply,
            trade_direction,
            round_direction,
        )
    }
}

impl Default for SwapCurve {
    fn default() -> Self {
        let curve_type = CurveType::default();
        let calculator: ConstantProductCurve = Default::default();
        Self {
            curve_type,
            calculator: Box::new(calculator),
        }
    }
}

impl PartialEq for SwapCurve {
    fn eq(&self, other: &Self) -> bool {
        let mut packed_self = [0u8; Self::LEN];
        self.pack_into_slice(&mut packed_self);
        let mut other_self = [0u8; Self::LEN];
        other.pack_into_slice(&mut other_self);
        packed_self[..] == other_self[..]
    }
}

impl Sealed for SwapCurve {}
impl Pack for SwapCurve {
    const LEN: usize = 33;

    fn pack_into_slice(&self, dst: &mut [u8]) {
        let output = array_mut_ref![dst, 0, 33];
        let (curve_type, calculator) = mut_array_refs![output, 1, 32];
        curve_type[0] = self.curve_type as u8;
        self.calculator.pack_into_slice(calculator);
    }

    fn unpack_from_slice(src: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![src, 0, 33];
        let (curve_type, calculator) = array_refs![input, 1, 32];
        let curve_type = CurveType::try_from(curve_type[0])?;
        let calculator = match curve_type {
            CurveType::ConstantProduct => {
                let calculator = ConstantProductCurve::unpack_from_slice(calculator)?;
                Box::new(calculator)
            }
            CurveType::ConstantPrice => todo!(),
            CurveType::ConstantProductWithOffset => todo!(),
        };
        Ok(Self {
            curve_type,
            calculator,
        })
    }
}

impl Clone for SwapCurve {
    fn clone(&self) -> Self {
        let mut packed_self = [0u8; Self::LEN];
        self.pack_into_slice(&mut packed_self);
        Self::unpack_from_slice(&packed_self).unwrap()
    }
}
