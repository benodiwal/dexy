use anchor_lang::prelude::*;

declare_id!("HRPryQD82JQcHALokdMpAYL83hUvSaSZGLKoHoFADvV");

#[program]
pub mod dexy {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
