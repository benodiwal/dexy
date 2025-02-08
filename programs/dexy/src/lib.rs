mod curve;

use anchor_lang::{prelude::*, solana_program::program_option::COption};
use anchor_spl::token::{self, Mint, MintTo, TokenAccount};
use curve::fees::CurveFees;

declare_id!("HRPryQD82JQcHALokdMpAYL83hUvSaSZGLKoHoFADvV");

#[program]
pub mod dexy {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let (swap_authority, bump_seed) = Pubkey::find_program_address(
            &[&ctx.accounts.amm.to_account_info().key.to_bytes()],
            ctx.program_id,
        );
        let _ = &ctx.accounts.validate_input_accounts(swap_authority)?;
        let _ = &mut ctx.accounts.mint_create_state_account(bump_seed)?;

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

impl<'info> Initialize<'info> {
    #[allow(unused)]
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

    #[allow(unused)]
    fn mint_create_state_account(&mut self, bump_seed: u8) -> Result<()> {
        let seeds = &[
            &self.amm.to_account_info().key().to_bytes(),
            &[bump_seed][..],
        ];

        let initial_ammount = 1 as u128;

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
        );

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

        Ok(())
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

pub fn build_curve(curve_input: &CurveInput) -> Result<()> {
    Ok(())
}
