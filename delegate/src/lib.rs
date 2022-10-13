use anchor_lang::{prelude::*, InstructionData};
use anchor_spl::token::{transfer, Transfer, Token, TokenAccount, Approve, approve};
use solana_program::{instruction::Instruction, program_option::COption};

// TODO update with correct ID
declare_id!("2Ls5MquEmp42AXBxKXX3a9Gu54aPYYVC19tV7RCMKsTt");

#[account]
pub struct SwapInfo {
    pub is_initialized: bool,
    pub poster: Pubkey,
    pub poster_sell_account: Pubkey,
    pub poster_buy_account: Pubkey,
    pub poster_sell_amount: u64,
    pub poster_buy_amount: u64,
}

pub const SWAP_INFO_BYTES: usize = 1 + 32 + 32 + 32 + 8 + 8;

#[derive(Accounts)]
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
        seeds=[sell_from.key().as_ref()],
        bump
    )]
    pub swap_info: Account<'info, SwapInfo>,
    pub delegate_program: Program<'info, program::Delegate>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn initialize_swap(
    poster: Pubkey, 
    sell_from: Pubkey, 
    buy_to: Pubkey, 
    swap_info: Pubkey, 
    sell_amount: u64,
    buy_amount: u64, 
) -> Instruction {
    let instruction = instruction::InitializeSwap {
        sell_amount,
        buy_amount,
    };
    Instruction::new_with_bytes(
        ID,
        &instruction.data(),
        vec![
            AccountMeta::new(poster, true),
            AccountMeta::new(sell_from, false),
            AccountMeta::new_readonly(buy_to, false),
            AccountMeta::new(swap_info, false),
            AccountMeta::new_readonly(ID, false),
            AccountMeta::new_readonly(anchor_spl::token::ID, false),
            AccountMeta::new_readonly(solana_program::system_program::ID, false),
        ],
    )
}

#[derive(Accounts)]
pub struct TakeSwap<'info> {
    #[account(mut)]
    pub taker: Signer<'info>,
    #[account(mut, constraint = taker_sell_from.owner == taker.key())]
    pub taker_sell_from: Account<'info, TokenAccount>,
    #[account(mut)]
    pub taker_buy_to: Account<'info, TokenAccount>,
    #[account(
        mut,
        seeds=[swap_info.poster_sell_account.as_ref()],
        bump,
        close = taker
    )]
    pub swap_info: Account<'info, SwapInfo>,
    #[account(
        mut, 
        address = swap_info.poster_sell_account,
        constraint = poster_sell_from.owner == swap_info.poster,
        constraint = poster_sell_from.delegate == COption::Some(swap_info.key())
    )]
    pub poster_sell_from: Account<'info, TokenAccount>,
    #[account(mut, address = swap_info.poster_buy_account)]
    pub poster_buy_to: Account<'info, TokenAccount>,
    pub delegate_program: Program<'info, program::Delegate>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn take_swap(
    taker: Pubkey, 
    taker_sell_from: Pubkey, 
    taker_buy_to: Pubkey, 
    swap_info: Pubkey, 
    swap_info_bump: u8,
    poster_sell_from: Pubkey, 
    poster_buy_to: Pubkey
) -> Instruction {
    let instruction = instruction::TakeSwap { swap_info_bump };
    Instruction::new_with_bytes(
        ID,
        &instruction.data(),
        vec![
            AccountMeta::new(taker, true),
            AccountMeta::new(taker_sell_from, false),
            AccountMeta::new(taker_buy_to, false),
            AccountMeta::new(swap_info, false),
            AccountMeta::new(poster_sell_from, false),
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

    pub fn initialize_swap(context: Context<PostSwap>, sell_amount: u64, buy_amount: u64) -> Result<()> {
        if context.accounts.swap_info.is_initialized {
            return err!(DelegateError::SwapInfoAlreadyInitialised);
        }

        // Intialize swap info information
        context.accounts.swap_info.is_initialized = true;
        context.accounts.swap_info.poster = context.accounts.poster.key();
        context.accounts.swap_info.poster_sell_account = context.accounts.sell_from.key();
        context.accounts.swap_info.poster_buy_account = context.accounts.buy_to.key();
        context.accounts.swap_info.poster_sell_amount = sell_amount;
        context.accounts.swap_info.poster_buy_amount = buy_amount;

        // Delegate to program
        let token_program = context.accounts.token_program.to_account_info();
        let token_accounts = Approve {
            to: context.accounts.sell_from.to_account_info(),
            delegate: context.accounts.swap_info.to_account_info(),
            authority: context.accounts.poster.to_account_info() 
        };
        let token_context = CpiContext::new(token_program, token_accounts);
        approve(token_context, sell_amount)?;

        Ok(())
    }

    pub fn take_swap(context: Context<TakeSwap>, swap_info_bump: u8) -> Result<()> {   
        // Moving tokens from poster to taker
        let token_program = context.accounts.token_program.to_account_info();
        let token_accounts = Transfer {
            from: context.accounts.poster_sell_from.to_account_info(),
            to: context.accounts.taker_buy_to.to_account_info(),
            authority: context.accounts.swap_info.to_account_info(),
        };
        let seeds = &[&[context.accounts.swap_info.poster_sell_account.as_ref(), std::slice::from_ref(&swap_info_bump)][..]];
        let token_ctx = CpiContext::new_with_signer(token_program, token_accounts, seeds);
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
pub enum DelegateError {
    #[msg("Swap information account is already initialised")]
    SwapInfoAlreadyInitialised,
}
