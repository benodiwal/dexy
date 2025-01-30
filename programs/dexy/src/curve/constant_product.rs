use anchor_lang::solana_program::program_pack::{IsInitialized, Pack, Sealed};
use spl_math::checked_ceil_div::CheckedCeilDiv;

use super::calculator::{map_zero_to_none, CurveCalculator, DynPack, SwapWithoutFeesResult};

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

impl CurveCalculator for ConstantProductCurve {
    fn swap_without_token_fees(
        &self,
        source_amount: u128,
        swap_source_amount: u128,
        swap_destination_amount: u128,
        _: super::calculator::TradeDirection,
    ) -> Option<super::calculator::SwapWithoutFeesResult> {
        swap(source_amount, swap_source_amount, swap_destination_amount)
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
