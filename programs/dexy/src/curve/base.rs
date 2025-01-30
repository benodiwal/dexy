use super::{calculator::CurveCalculator, constant_product::ConstantProductCurve};
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

    /// Constant price curve (stable swap)
    /// Formula: y = mx where m is the price
    ConstantPrice,

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
