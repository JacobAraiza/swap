use anchor_lang::{prelude::*, InstructionData};
use anchor_spl::token::{transfer, Transfer, Token, TokenAccount, Mint};
use solana_program::{instruction::Instruction};

declare_id!("2Ls5MquEmp42AXBxKXX3a9Gu54aPYYVC19tV7RCMKsTp");

#[account]
pub struct SwapInfo {
    pub is_initialized: bool,
    pub poster: Pubkey,
    pub escrow_account: Pubkey,
    pub poster_buy_account: Pubkey,
    pub poster_sell_amount: u64,
    pub poster_buy_amount: u64,
}

pub const SWAP_INFO_BYTES: usize = 1 + 32 + 32 + 32 + 8 + 8;

#[derive(Accounts)]
#[instruction(swap_seed: Vec<u8>)]
pub struct PostSwap<'info> {
    #[account(mut)]
    pub poster: Signer<'info>,
    #[account(
        mut, 
        constraint = sell_from.owner == poster.key(),
    )]
    pub sell_from: Account<'info, TokenAccount>,
    pub buy_to: Account<'info, TokenAccount>,
    #[account(
        init_if_needed,
        space = 8 + SWAP_INFO_BYTES,
        payer=poster,
        owner=*program_id,
        seeds=[&swap_seed],
        bump
    )]
    pub swap_info: Account<'info, SwapInfo>,
    #[account(
        init,
        payer = poster,
        token::mint = mint,
        token::authority = escrow,
        seeds=[swap_info.key().as_ref()],
        bump,
        owner=token_program.key()
    )]
    pub escrow: Account<'info, TokenAccount>,
    #[account(
        constraint = sell_from.mint == mint.key()
    )]
    pub mint: Account<'info, Mint>,
    pub escrow_program: Program<'info, program::Delegate>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: UncheckedAccount<'info>,
}

#[allow(clippy::too_many_arguments)]
pub fn initialize_swap(
    poster: Pubkey, 
    sell_from: Pubkey, 
    buy_to: Pubkey, 
    swap_info: Pubkey, 
    swap_seed: Vec<u8>,
    escrow_address: Pubkey,
    escrow_mint: Pubkey,
    sell_amount: u64,
    buy_amount: u64, 
) -> Result<Instruction> {
    let instruction = instruction::InitializeSwap {
        swap_seed,
        sell_amount,
        buy_amount,
    };
    Ok(Instruction::new_with_bytes(
        ID,
        &instruction.data(),
        vec![
            AccountMeta::new(poster, true),
            AccountMeta::new(sell_from, false),
            AccountMeta::new_readonly(buy_to, false),
            AccountMeta::new(swap_info, false),
            AccountMeta::new(escrow_address, false),
            AccountMeta::new_readonly(escrow_mint, false),
            AccountMeta::new_readonly(ID, false),
            AccountMeta::new_readonly(anchor_spl::token::ID, false),
            AccountMeta::new_readonly(solana_program::system_program::ID, false),
            AccountMeta::new_readonly(solana_program::sysvar::rent::ID, false),
        ],
    ))
}

#[derive(Accounts)]
pub struct TakeSwap<'info> {
    #[account(mut)]
    pub taker: Signer<'info>,
    #[account(
        mut, 
        constraint = taker_sell_from.owner == taker.key())
    ]
    pub taker_sell_from: Account<'info, TokenAccount>,
    #[account(mut)]
    pub taker_buy_to: Account<'info, TokenAccount>,
    #[account(mut, close = taker)]
    pub swap_info: Account<'info, SwapInfo>,
    #[account(
        mut, 
        address = swap_info.escrow_account,
    )]
    pub escrow: Account<'info, TokenAccount>,
    #[account(mut, address = swap_info.poster_buy_account)]
    pub poster_buy_to: Account<'info, TokenAccount>,
    pub escrow_program: Program<'info, program::Delegate>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn take_swap(
    taker: Pubkey, 
    taker_sell_from: Pubkey, 
    taker_buy_to: Pubkey, 
    swap_info: Pubkey,
    escrow: Pubkey, 
    poster_buy_to: Pubkey
) -> Instruction {
    let instruction = instruction::TakeSwap {};
    Instruction::new_with_bytes(
        ID,
        &instruction.data(),
        vec![
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_sell_from, false),
            AccountMeta::new(taker_buy_to, false),
            AccountMeta::new(swap_info, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(poster_buy_to, false),
            AccountMeta::new_readonly(ID, false),
            AccountMeta::new_readonly(anchor_spl::token::ID, false),
            AccountMeta::new_readonly(solana_program::system_program::ID, false),
        ],
    )
}

#[program]
pub mod delegate {

    use super::*;

    pub fn initialize_swap(context: Context<PostSwap>, swap_seed: Vec<u8>, sell_amount: u64, buy_amount: u64) -> Result<()> {
        if context.accounts.swap_info.is_initialized {
            return err!(EscrowError::SwapInfoAlreadyInitialised);
        }

        // Intialize swap information account
        context.accounts.swap_info.is_initialized = true;
        context.accounts.swap_info.poster = context.accounts.poster.key();
        context.accounts.swap_info.escrow_account = context.accounts.escrow.key();
        context.accounts.swap_info.poster_buy_account = context.accounts.buy_to.key();
        context.accounts.swap_info.poster_sell_amount = sell_amount;
        context.accounts.swap_info.poster_buy_amount = buy_amount;

        // Transfer to escrow
        let token_program = context.accounts.token_program.to_account_info();
        let token_accounts = Transfer {
            from: context.accounts.sell_from.to_account_info(),
            to: context.accounts.escrow.to_account_info(),
            authority: context.accounts.poster.to_account_info() 
        };
        let token_context = CpiContext::new(token_program, token_accounts);
        transfer(token_context, sell_amount)?;

        Ok(())
    }

    pub fn take_swap(context: Context<TakeSwap>) -> Result<()> {   
        // Calculate escrow seed for signing transfer
        let seed = context.accounts.swap_info.key();
        let (_address, bump) = Pubkey::find_program_address(&[seed.as_ref()], &ID);
        let full_seed = &[&[seed.as_ref(), std::slice::from_ref(&bump)][..]];

        // Moving tokens from escrow to taker
        let token_program = context.accounts.token_program.to_account_info();
        let token_accounts = Transfer {
            from: context.accounts.escrow.to_account_info(),
            to: context.accounts.taker_buy_to.to_account_info(),
            authority: context.accounts.escrow.to_account_info(),
        };
        let token_ctx = CpiContext::new_with_signer(token_program, token_accounts, full_seed);
        transfer(token_ctx, context.accounts.swap_info.poster_sell_amount)?;

        // Moving tokens from taker to poster
        let token_program = context.accounts.token_program.to_account_info();
        let token_accounts = Transfer {
            from: context.accounts.taker_sell_from.to_account_info(),
            to: context.accounts.poster_buy_to.to_account_info(),
            authority: context.accounts.taker.to_account_info(),
        };
        let token_ctx = CpiContext::new(token_program, token_accounts);
        transfer(token_ctx, context.accounts.swap_info.poster_buy_amount)?;

        Ok(())
    }
}


#[error_code]
pub enum EscrowError {
    #[msg("Swap information account is already initialised")]
    SwapInfoAlreadyInitialised
}
