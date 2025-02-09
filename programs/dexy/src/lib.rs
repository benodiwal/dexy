mod curve;

use anchor_lang::{prelude::*, solana_program::program_option::COption};
use anchor_spl::token::{self, Burn, Mint, MintTo, TokenAccount, Transfer};
use curve::{
    base::{CurveType, SwapCurve},
    calculator::{CurveCalculator, RoundDirection},
    constant_product::ConstantProductCurve,
    fees::CurveFees,
};

declare_id!("HRPryQD82JQcHALokdMpAYL83hUvSaSZGLKoHoFADvV");

#[program]
pub mod dexy {
    use crate::curve::calculator::TradeDirection;

    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        fees_input: FeeInput,
        curve_input: CurveInput,
    ) -> Result<()> {
        let (swap_authority, bump_seed) = Pubkey::find_program_address(
            &[&ctx.accounts.amm.to_account_info().key.to_bytes()],
            ctx.program_id,
        );
        let curve = ctx
            .accounts
            .validate_amm_fees_and_curve(&fees_input, &curve_input)?;
        let _ = &ctx.accounts.validate_input_accounts(swap_authority)?;
        let _ = &mut ctx.accounts.mint_create_state_account(
            bump_seed,
            curve_input,
            fees_input,
            &curve,
        )?;

        Ok(())
    }

    pub fn swap(ctx: Context<Swap>, amount_in: u64, minimum_amount_out: u64) -> Result<()> {
        let amm = &mut ctx.accounts.amm;
        if amm.to_account_info().owner != ctx.program_id {
            return Err(SwapError::InvalidOwner.into());
        }

        if *ctx.accounts.authority.key
            != authority_key(ctx.program_id, amm.to_account_info().key(), amm.bump_seed)?
        {
            return Err(SwapError::InvalidProgramAddress.into());
        }

        if !(*ctx.accounts.swap_source.to_account_info().key == amm.token_a_account
            || *ctx.accounts.swap_source.to_account_info().key == amm.token_b_account)
        {
            return Err(SwapError::IncorrectSwapAccount.into());
        }

        if !(*ctx.accounts.swap_destination.to_account_info().key == amm.token_a_account
            || *ctx.accounts.swap_destination.to_account_info().key == amm.token_b_account)
        {
            return Err(SwapError::IncorrectSwapAccount.into());
        }

        if *ctx.accounts.swap_source.to_account_info().key
            == *ctx.accounts.swap_destination.to_account_info().key
        {
            return Err(SwapError::InvalidInput.into());
        }

        if ctx.accounts.swap_source.to_account_info().key != ctx.accounts.source_info.key {
            return Err(SwapError::InvalidInput.into());
        }

        if ctx.accounts.swap_destination.to_account_info().key != ctx.accounts.destination_info.key
        {
            return Err(SwapError::InvalidInput.into());
        }

        if *ctx.accounts.pool_mint.to_account_info().key != amm.pool_mint {
            return Err(SwapError::IncorrectPoolMint.into());
        }

        if *ctx.accounts.host_fee_account.to_account_info().key != amm.pool_fee_account {
            return Err(SwapError::IncorrectFeeAccount.into());
        }

        if *ctx.accounts.token_program.key != amm.token_program_id {
            return Err(SwapError::IncorrectTokenProgramId.into());
        }

        let trade_direction =
            if *ctx.accounts.swap_source.to_account_info().key == amm.token_a_account {
                TradeDirection::AtoB
            } else {
                TradeDirection::BtoA
            };

        let curve = build_curve(&amm.curve).unwrap();
        let fees = build_fees(&amm.fees).unwrap();

        let result = curve
            .calculator
            .swap_without_token_fees(
                u128::from(amount_in),
                u128::from(ctx.accounts.swap_source.amount),
                u128::from(ctx.accounts.swap_destination.amount),
                trade_direction,
            )
            .ok_or(SwapError::ZeroTradingTokens)?;

        let mut trade_fee = fees
            .trading_fee(result.destination_amount_swapped)
            .ok_or(SwapError::FeeCalculationFailure)?;

        let mut owner_fee = fees
            .owner_trading_fee(result.destination_amount_swapped)
            .ok_or(SwapError::FeeCalculationFailure)?;

        let host_fee = if owner_fee > 0 {
            fees.host_fee(owner_fee)
                .ok_or(SwapError::FeeCalculationFailure)?
        } else {
            0
        };

        if host_fee > 0 {
            owner_fee = owner_fee
                .checked_sub(host_fee)
                .ok_or(SwapError::FeeCalculationFailure)?;
        }

        let total_fees = trade_fee
            .checked_add(owner_fee)
            .ok_or(SwapError::FeeCalculationFailure)?;

        let destination_amount_swapped = result
            .destination_amount_swapped
            .checked_sub(total_fees)
            .ok_or(SwapError::FeeCalculationFailure)?;

        let output_amount = u64::try_from(destination_amount_swapped)
            .map_err(|_| SwapError::ConversionFailure)?;

        if output_amount < minimum_amount_out {
            return Err(SwapError::ExceededSlippage.into());
        }

        let seeds = &[
            &amm.to_account_info().key().to_bytes(),
            &[amm.bump_seed][..],
        ];

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.clone(),
                token::Transfer {
                    from: ctx.accounts.source_info.clone(),
                    to: ctx.accounts.swap_source.to_account_info().clone(),
                    authority: ctx.accounts.user_transfer_authority.clone(),
                },
            ),
            amount_in,
        )?;

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.clone(),
                token::Transfer {
                    from: ctx.accounts.swap_destination.to_account_info().clone(),
                    to: ctx.accounts.destination_info.clone(),
                    authority: ctx.accounts.authority.clone(),
                },
                &[&seeds[..]],
            ),
            output_amount,
        )?;

        if owner_fee > 0 {
            let pool_mint_amount = curve
                .calculator
                .deposit_single_token_type(
                    owner_fee,
                    u128::from(ctx.accounts.swap_source.amount),
                    u128::from(ctx.accounts.swap_destination.amount),
                    u128::from(ctx.accounts.pool_mint.supply),
                    trade_direction,
                    RoundDirection::Floor,
                )
                .ok_or(SwapError::ZeroOwnerTradeFee)?;

            token::mint_to(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.clone(),
                    MintTo {
                        mint: ctx.accounts.pool_mint.to_account_info().clone(),
                        to: ctx.accounts.pool_account.to_account_info().clone(),
                        authority: ctx.accounts.authority.clone(),
                    },
                    &[&seeds[..]],
                ),
                u64::try_from(pool_mint_amount).map_err(|_| SwapError::ConversionFailure)?,
            )?;
        }

        if host_fee > 0 {
            let host_fee_mint_amount = curve
                .calculator
                .deposit_single_token_type(
                    host_fee,
                    u128::from(ctx.accounts.swap_source.amount),
                    u128::from(ctx.accounts.swap_destination.amount),
                    u128::from(ctx.accounts.pool_mint.supply),
                    trade_direction,
                    RoundDirection::Floor,
                )
                .ok_or(SwapError::ZeroHostFee)?;

            token::mint_to(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.clone(),
                    MintTo {
                        mint: ctx.accounts.pool_mint.to_account_info().clone(),
                        to: ctx.accounts.host_fee_account.clone(),
                        authority: ctx.accounts.authority.clone(),
                    },
                    &[&seeds[..]],
                ),
                u64::try_from(host_fee_mint_amount).map_err(|_| SwapError::ConversionFailure)?,
            )?;
        }

        Ok(())
    }

    pub fn deposit_liquidity(
        ctx: Context<DepositLiquidity>,
        pool_token_amount: u64,
        maximum_token_a_amount: u64,
        maximum_token_b_amount: u64,
    ) -> Result<()> {
        let amm = &mut ctx.accounts.amm;

        if !amm.is_initialized {
            return Err(SwapError::NotInitialized.into());
        }

        let curve = build_curve(&amm.curve)?;

        let current_pool_mint_supply = u128::from(ctx.accounts.pool_mint.supply);
        let (token_a_amount, token_b_amount) = if current_pool_mint_supply > 0 {
            let tokens = curve
                .calculator
                .pool_tokens_to_trading_tokens(
                    u128::from(pool_token_amount),
                    u128::from(ctx.accounts.pool_mint.supply),
                    u128::from(ctx.accounts.token_a.amount),
                    u128::from(ctx.accounts.token_b.amount),
                    RoundDirection::Ceil,
                )
                .ok_or(SwapError::ZeroTradingTokens)?;

            let token_a_amount = u64::try_from(tokens.token_a_amount)
                .map_err(|_| SwapError::ConversionFailure)?;
            let token_b_amount = u64::try_from(tokens.token_b_amount)
                .map_err(|_| SwapError::ConversionFailure)?;

            if token_a_amount > maximum_token_a_amount {
                return Err(SwapError::ExceededSlippage.into());
            }
            if token_b_amount > maximum_token_b_amount {
                return Err(SwapError::ExceededSlippage.into());
            }

            (token_a_amount, token_b_amount)
        } else {
            (maximum_token_a_amount, maximum_token_b_amount)
        };

        let seeds = &[
            &amm.to_account_info().key().to_bytes(),
            &[amm.bump_seed][..],
        ];

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.clone(),
                Transfer {
                    from: ctx.accounts.user_token_a.to_account_info().clone(),
                    to: ctx.accounts.token_a.to_account_info().clone(),
                    authority: ctx.accounts.user_transfer_authority.clone(),
                },
            ),
            token_a_amount,
        )?;

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.clone(),
                Transfer {
                    from: ctx.accounts.user_token_b.to_account_info().clone(),
                    to: ctx.accounts.token_b.to_account_info().clone(),
                    authority: ctx.accounts.user_transfer_authority.clone(),
                },
            ),
            token_b_amount,
        )?;

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.clone(),
                MintTo {
                    mint: ctx.accounts.pool_mint.to_account_info().clone(),
                    to: ctx.accounts.user_pool_token.to_account_info().clone(),
                    authority: ctx.accounts.authority.clone(),
                },
                &[&seeds[..]],
            ),
            pool_token_amount,
        )?;

        Ok(())
    }

    pub fn withdraw_liquidity(
        ctx: Context<WithdrawLiquidity>,
        pool_token_amount: u64,
        minimum_token_a_amount: u64,
        minimum_token_b_amount: u64,
    ) -> Result<()> {
        let amm = &mut ctx.accounts.amm;

        if !amm.is_initialized {
            return Err(SwapError::NotInitialized.into());
        }

        let curve = build_curve(&amm.curve)?;
        let fees = build_fees(&amm.fees)?;

        let withdraw_fee = fees
            .owner_withdraw_fee(u128::from(pool_token_amount))
            .ok_or(SwapError::FeeCalculationFailure)?;

        let pool_token_amount_after_fee = u128::from(pool_token_amount)
            .checked_sub(withdraw_fee)
            .ok_or(SwapError::FeeCalculationFailure)?;

        let tokens = curve
            .calculator
            .pool_tokens_to_trading_tokens(
                pool_token_amount_after_fee,
                u128::from(ctx.accounts.pool_mint.supply),
                u128::from(ctx.accounts.token_a.amount),
                u128::from(ctx.accounts.token_b.amount),
                RoundDirection::Floor,
            )
            .ok_or(SwapError::ZeroTradingTokens)?;

        let token_a_amount = u64::try_from(tokens.token_a_amount)
            .map_err(|_| SwapError::ConversionFailure)?;
        let token_b_amount = u64::try_from(tokens.token_b_amount)
            .map_err(|_| SwapError::ConversionFailure)?;

        if token_a_amount < minimum_token_a_amount {
            return Err(SwapError::ExceededSlippage.into());
        }
        if token_b_amount < minimum_token_b_amount {
            return Err(SwapError::ExceededSlippage.into());
        }

        let seeds = &[
            &amm.to_account_info().key().to_bytes(),
            &[amm.bump_seed][..],
        ];

        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.clone(),
                token::Burn {
                    mint: ctx.accounts.pool_mint.to_account_info().clone(),
                    from: ctx.accounts.source_pool_account.to_account_info().clone(),
                    authority: ctx.accounts.user_transfer_authority.clone(),
                },
            ),
            pool_token_amount,
        )?;

        if withdraw_fee > 0 {
            token::mint_to(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.clone(),
                    MintTo {
                        mint: ctx.accounts.pool_mint.to_account_info().clone(),
                        to: ctx.accounts.fee_account.to_account_info().clone(),
                        authority: ctx.accounts.authority.clone(),
                    },
                    &[&seeds[..]],
                ),
                u64::try_from(withdraw_fee).map_err(|_| SwapError::ConversionFailure)?,
            )?;
        }

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.clone(),
                Transfer {
                    from: ctx.accounts.token_a.to_account_info().clone(),
                    to: ctx.accounts.user_token_a.to_account_info().clone(),
                    authority: ctx.accounts.authority.clone(),
                },
                &[&seeds[..]],
            ),
            token_a_amount,
        )?;

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.clone(),
                Transfer {
                    from: ctx.accounts.token_b.to_account_info().clone(),
                    to: ctx.accounts.user_token_b.to_account_info().clone(),
                    authority: ctx.accounts.authority.clone(),
                },
                &[&seeds[..]],
            ),
            token_b_amount,
        )?;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    /// CHECK: This is the authority for the swap. The validation is handled in the instruction logic.
    #[account(mut)]
    pub authority: AccountInfo<'info>,
    /// CHECK: This is the initializer of the swap. The validation is handled in the instruction logic.
    #[account(mut, signer)]
    pub initializer: AccountInfo<'info>,
    #[account(init, payer=initializer, space=999)]
    pub amm: Box<Account<'info, Amm>>,
    #[account(mut)]
    pub pool_mint: Box<Account<'info, Mint>>,
    #[account(mut)]
    pub token_a: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_b: Account<'info, TokenAccount>,
    #[account(mut)]
    pub fee_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub destination: Account<'info, TokenAccount>,
    /// CHECK: This is the Solana token program, which is a known, trusted program
    pub token_program: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Swap<'info> {
    /// CHECK: This is the authority for the swap. The validation is handled in the instruction logic.
    pub authority: AccountInfo<'info>,
    pub amm: Box<Account<'info, Amm>>,
    /// CHECK: This is the user transfer authority. The validation is handled in the instruction logic.
    #[account(signer)]
    pub user_transfer_authority: AccountInfo<'info>,
    /// CHECK: This is the source token account. The validation is handled in the instruction logic.
    #[account(mut)]
    pub source_info: AccountInfo<'info>,
    /// CHECK: This is the destination token account. The validation is handled in the instruction logic.
    #[account(mut)]
    pub destination_info: AccountInfo<'info>,
    #[account(mut)]
    pub swap_source: Account<'info, TokenAccount>,
    #[account(mut)]
    pub swap_destination: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_mint: Account<'info, Mint>,
    #[account(mut)]
    pub pool_account: Account<'info, TokenAccount>,
    /// CHECK: This is the Solana token program, which is a known, trusted program
    pub token_program: AccountInfo<'info>,
    /// CHECK: This is the system program, which is a known, trusted program
    pub host_fee_account: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct DepositLiquidity<'info> {
    /// CHECK: This is the authority for the swap. The validation is handled in the instruction logic.
    pub authority: AccountInfo<'info>,
    pub amm: Box<Account<'info, Amm>>,
    /// CHECK: This is the user transfer authority. The validation is handled in the instruction logic.
    #[account(signer)]
    pub user_transfer_authority: AccountInfo<'info>,
    #[account(mut)]
    pub user_token_a: Account<'info, TokenAccount>,
    #[account(mut)]
    pub user_token_b: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_a: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_b: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_mint: Account<'info, Mint>,
    #[account(mut)]
    pub user_pool_token: Account<'info, TokenAccount>,
    /// CHECK: This is the Solana token program, which is a known, trusted program
    pub token_program: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct WithdrawLiquidity<'info> {
    /// CHECK: This is the authority for the swap. The validation is handled in the instruction logic.
    pub authority: AccountInfo<'info>,
    pub amm: Box<Account<'info, Amm>>,
    /// CHECK: This is the user transfer authority. The validation is handled in the instruction logic.
    #[account(signer)]
    pub user_transfer_authority: AccountInfo<'info>,
    #[account(mut)]
    pub source_pool_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_a: Account<'info, TokenAccount>,
    #[account(mut)]
    pub token_b: Account<'info, TokenAccount>,
    #[account(mut)]
    pub user_token_a: Account<'info, TokenAccount>,
    #[account(mut)]
    pub user_token_b: Account<'info, TokenAccount>,
    #[account(mut)]
    pub pool_mint: Account<'info, Mint>,
    #[account(mut)]
    pub fee_account: Account<'info, TokenAccount>,
    /// CHECK: This is the Solana token program, which is a known, trusted program
    pub token_program: AccountInfo<'info>,
}

impl<'info> Initialize<'info> {
    fn validate_input_accounts(&self, swap_authority: Pubkey) -> Result<()> {
        if self.amm.is_initialized {
            return Err(SwapError::AlreadyInUse.into());
        }

        if *self.authority.key != swap_authority {
            return Err(SwapError::InvalidProgramAddress.into());
        }
        if *self.authority.key != self.token_a.owner || *self.authority.key != self.token_b.owner {
            return Err(SwapError::InvalidOwner.into());
        }

        if *self.authority.key == self.fee_account.owner
            && *self.authority.key == self.destination.owner
        {
            return Err(SwapError::InvalidOuputOwner.into());
        }
        if COption::Some(*self.authority.key) != self.pool_mint.mint_authority {
            return Err(SwapError::InvalidOwner.into());
        }
        if self.token_a.mint == self.token_b.mint {
            return Err(SwapError::RepeatedMint.into());
        }

        if self.token_a.delegate.is_some() || self.token_b.delegate.is_some() {
            return Err(SwapError::InvalidDelegate.into());
        }

        if self.token_a.close_authority.is_some() || self.token_b.close_authority.is_some() {
            return Err(SwapError::InvalidCloseAuthority.into());
        }

        if self.pool_mint.supply != 0 {
            return Err(SwapError::InvalidSupply.into());
        }

        if self.pool_mint.freeze_authority.is_some() {
            return Err(SwapError::InvalidFreezeAuthority.into());
        }

        if *self.pool_mint.to_account_info().key != self.fee_account.mint {
            return Err(SwapError::IncorrectPoolMint.into());
        }

        Ok(())
    }

    fn mint_create_state_account(
        &mut self,
        bump_seed: u8,
        curve_input: CurveInput,
        fee_input: FeeInput,
        curve: &SwapCurve,
    ) -> Result<()> {
        let seeds = &[
            &self.amm.to_account_info().key().to_bytes(),
            &[bump_seed][..],
        ];

        let initial_ammount = curve.calculator.new_supply_pool();

        let mint_initial_amt_cpi_ctx = CpiContext::new(
            self.token_program.clone(),
            MintTo {
                mint: self.pool_mint.to_account_info().clone(),
                to: self.destination.to_account_info().clone(),
                authority: self.authority.clone(),
            },
        );

        token::mint_to(
            mint_initial_amt_cpi_ctx.with_signer(&[&seeds[..]]),
            u64::try_from(initial_ammount).unwrap(),
        )?;

        let amm = &mut self.amm;
        amm.is_initialized = true;
        amm.bump_seed = bump_seed;
        amm.token_program_id = *self.token_program.key;
        amm.token_a_account = *self.token_a.to_account_info().key;
        amm.token_b_account = *self.token_b.to_account_info().key;
        amm.pool_mint = *self.pool_mint.to_account_info().key;
        amm.token_a_mint = self.token_a.mint;
        amm.token_b_mint = self.token_b.mint;
        amm.pool_fee_account = *self.fee_account.to_account_info().key;
        amm.fees = fee_input;
        amm.curve = curve_input;

        Ok(())
    }

    fn validate_amm_fees_and_curve(
        &self,
        fees_input: &FeeInput,
        curve_input: &CurveInput,
    ) -> Result<SwapCurve> {
        let curve = build_curve(curve_input).unwrap();
        curve
            .calculator
            .validate_supply(self.token_a.amount, self.token_b.amount)?;

        let fees = build_fees(fees_input)?;
        fees.validate()?;
        curve.calculator.validate()?;
        Ok(curve)
    }
}

#[account]
pub struct Amm {
    pub is_initialized: bool,
    pub bump_seed: u8,
    pub token_program_id: Pubkey,
    // Token A liquidity Account
    pub token_a_account: Pubkey,
    // Token B liquidity Account
    pub token_b_account: Pubkey,
    pub pool_mint: Pubkey,
    // Token A mint Account
    pub token_a_mint: Pubkey,
    // Token B mint Account
    pub token_b_mint: Pubkey,
    // Pool fee account
    pub pool_fee_account: Pubkey,
    // Fees associated with swap
    pub fees: FeeInput,
    // Curve associated with swap
    pub curve: CurveInput,
}

#[error_code]
pub enum SwapError {
    #[msg("Swap account already in use")]
    AlreadyInUse,
    #[msg("Invalid program address generated from bump seed and key")]
    InvalidProgramAddress,
    #[msg("Input account owner is not the program address")]
    InvalidOwner,
    #[msg("Output pool account owner cannot be the program address")]
    InvalidOuputOwner,
    #[msg("Swap input token accounts have the same mint")]
    RepeatedMint,
    #[msg("Token account has a delegate")]
    InvalidDelegate,
    #[msg("Token account has a close authority")]
    InvalidCloseAuthority,
    #[msg("Pool token supply is non-zero")]
    InvalidSupply,
    #[msg("Pool token has a freeze authority")]
    InvalidFreezeAuthority,
    #[msg("Address of the provided pool token mint is incorrect")]
    IncorrectPoolMint,
    #[msg("Empty supply")]
    EmptySupply,
    #[msg("Invalid fees")]
    InvalidFees,
    #[msg("Incorrect swap account")]
    IncorrectSwapAccount,
    #[msg("Invald Input Token for Swap")]
    InvalidInput,
    #[msg("Incorrect Fee Account")]
    IncorrectFeeAccount,
    #[msg("Incorrect Token Program Id")]
    IncorrectTokenProgramId,
    #[msg("Given pool token amount results in zero trading tokens")]
    ZeroTradingTokens,
    #[msg("Fee calculation failed")]
    FeeCalculationFailure,
    #[msg("Conversion overflow")]
    ConversionFailure,
    #[msg("The provided slippage limit has been exceeded")]
    ExceededSlippage,
    #[msg("Zero owner trading fee")]
    ZeroOwnerTradeFee,
    #[msg("Zero host fee")]
    ZeroHostFee,
    #[msg("Invalid fee percentage")]
    InvalidPercentage,
    #[msg("AMM not initialized")]
    NotInitialized,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct FeeInput {
    pub trade_fee_numerator: u64,
    pub trade_fee_denominator: u64,
    pub owner_trade_fee_numerator: u64,
    pub owner_trade_fee_denominator: u64,
    pub owner_withdraw_fee_numerator: u64,
    pub owner_withdraw_fee_denominator: u64,
    pub host_fee_numerator: u64,
    pub host_fee_denominator: u64,
}

pub fn build_fees(fee_input: &FeeInput) -> Result<CurveFees> {
    let fees = CurveFees {
        trade_fee_numerator: fee_input.trade_fee_numerator,
        trade_fee_denominator: fee_input.trade_fee_denominator,
        owner_trade_fee_numerator: fee_input.owner_trade_fee_numerator,
        owner_trade_fee_denominator: fee_input.owner_trade_fee_denominator,
        owner_withdraw_fee_numerator: fee_input.owner_withdraw_fee_numerator,
        owner_withdraw_fee_denominator: fee_input.owner_withdraw_fee_denominator,
        host_fee_numerator: fee_input.host_fee_numerator,
        host_fee_denominator: fee_input.host_fee_denominator,
    };
    Ok(fees)
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct CurveInput {
    pub curve_type: u8,
    pub curve_params: u64,
}

pub fn build_curve(curve_input: &CurveInput) -> Result<SwapCurve> {
    let curve_type = CurveType::try_from(curve_input.curve_type).unwrap();
    let calculator: Box<dyn CurveCalculator> = match curve_type {
        CurveType::ConstantProduct => Box::new(ConstantProductCurve {}),
        CurveType::ConstantPrice => unimplemented!(),
        CurveType::ConstantProductWithOffset => unimplemented!(),
    };
    let curve = SwapCurve {
        curve_type,
        calculator,
    };
    Ok(curve)
}

pub fn authority_key(program_id: &Pubkey, info: Pubkey, bump_seed: u8) -> Result<Pubkey> {
    Pubkey::create_program_address(&[&info.to_bytes()[..32], &[bump_seed]], program_id)
        .or(Err(SwapError::InvalidProgramAddress.into()))
}
