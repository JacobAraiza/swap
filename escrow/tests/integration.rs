use anchor_lang::AnchorDeserialize;
use rand::Rng;
use solana_program::{instruction::Instruction, program_pack::Pack};
use solana_program_test::{processor, tokio, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account::AccountSharedData, pubkey::Pubkey, signature::Keypair, signer::Signer,
    signers::Signers, transaction::Transaction,
};
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account,
};

type Error = Box<dyn std::error::Error>;

#[tokio::test]
async fn test_program() {
    // Setup testing validator and accounts
    let mut validator = ProgramTest::default();
    validator.add_program("escrow", escrow::ID, processor!(escrow::entry));

    let authority = add_wallet(&mut validator);
    let poster = add_wallet(&mut validator);
    let taker = add_wallet(&mut validator);

    let mut context = validator.start_with_context().await;

    // Create tokens for trade
    let alpha_mint = create_token_mint(&mut context, &authority, 0)
        .await
        .unwrap();
    let beta_mint = create_token_mint(&mut context, &authority, 0)
        .await
        .unwrap();

    // Create test accounts
    let poster_alpha = create_token_account(&mut context, &poster, &alpha_mint)
        .await
        .unwrap();
    let poster_beta = create_token_account(&mut context, &poster, &beta_mint)
        .await
        .unwrap();
    let taker_alpha = create_token_account(&mut context, &taker, &alpha_mint)
        .await
        .unwrap();
    let taker_beta = create_token_account(&mut context, &taker, &beta_mint)
        .await
        .unwrap();

    // Mint tokens
    mint_token(&mut context, &authority, &alpha_mint, &poster_alpha, 10)
        .await
        .unwrap();
    mint_token(&mut context, &authority, &beta_mint, &taker_beta, 7)
        .await
        .unwrap();

    // Check initial balances as expected
    assert_eq!(token_balance(&mut context, poster_alpha).await.unwrap(), 10);
    assert_eq!(token_balance(&mut context, poster_beta).await.unwrap(), 0);
    assert_eq!(token_balance(&mut context, taker_alpha).await.unwrap(), 0);
    assert_eq!(token_balance(&mut context, taker_beta).await.unwrap(), 7);

    // Create escrow and swap info accounts
    let swap_seed: Vec<_> = (0..10).map(|_| rand::thread_rng().gen()).collect();
    let (swap_address, _swap_bump) =
        Pubkey::find_program_address(&[swap_seed.as_ref()], &escrow::ID);
    let (escrow_address, _escrow_bump) =
        Pubkey::find_program_address(&[swap_address.as_ref()], &escrow::ID);

    // Post a swap on the market
    initialise_swap(
        &mut context,
        &poster,
        &poster_alpha,
        &poster_beta,
        &escrow_address,
        &alpha_mint,
        &swap_address,
        10,
        7,
        swap_seed.clone(),
    )
    .await
    .unwrap();

    // Check swap information posted correctly
    let swap_account = context
        .banks_client
        .get_account(swap_address)
        .await
        .unwrap()
        .unwrap();
    // (Skipping the first 8 bytes which are used by anchor to tag the type of account)
    let swap_info = escrow::SwapInfo::deserialize(&mut &swap_account.data[8..]).unwrap();
    assert_eq!(swap_info.poster, poster.pubkey());
    assert_eq!(swap_info.escrow_account, escrow_address);
    assert_eq!(swap_info.poster_sell_amount, 10);
    assert_eq!(swap_info.poster_buy_account, poster_beta);
    assert_eq!(swap_info.poster_buy_amount, 7);

    // Re-using the same swap account for a different trade should fail
    assert!(initialise_swap(
        &mut context,
        &poster,
        &poster_alpha,
        &poster_beta,
        &escrow_address,
        &alpha_mint,
        &swap_address,
        9,
        6,
        swap_seed,
    )
    .await
    .is_err());

    // Take the swap
    take_swap(
        &mut context,
        &taker,
        &taker_beta,
        &taker_alpha,
        &swap_address,
        &escrow_address,
        &poster_beta,
    )
    .await
    .unwrap();

    // Check swap happened as expected (and that swap info account was closed after)
    assert_eq!(token_balance(&mut context, poster_alpha).await.unwrap(), 0);
    assert_eq!(token_balance(&mut context, poster_beta).await.unwrap(), 7);
    assert_eq!(token_balance(&mut context, taker_alpha).await.unwrap(), 10);
    assert_eq!(token_balance(&mut context, taker_beta).await.unwrap(), 0);

    assert!(context
        .banks_client
        .get_account(swap_address)
        .await
        .unwrap()
        .is_none());
}

fn add_wallet(validator: &mut ProgramTest) -> Keypair {
    let keypair = Keypair::new();
    let account = AccountSharedData::new(1_000_000_000_000, 0, &solana_sdk::system_program::id());
    validator.add_account(keypair.pubkey(), account.into());
    keypair
}

async fn create_token_mint(
    context: &mut ProgramTestContext,
    authority: &Keypair,
    decimals: u8,
) -> Result<Pubkey, Error> {
    let mint = Keypair::new();
    let space = spl_token::state::Mint::LEN;
    let lamports = context
        .banks_client
        .get_rent()
        .await?
        .minimum_balance(space);
    let create = solana_sdk::system_instruction::create_account(
        &authority.pubkey(),
        &mint.pubkey(),
        lamports,
        space as u64,
        &spl_token::ID,
    );
    let initialize = spl_token::instruction::initialize_mint(
        &spl_token::ID,
        &mint.pubkey(),
        &authority.pubkey(),
        None,
        decimals,
    )?;
    execute(
        context,
        authority,
        &[create, initialize],
        &[authority, &mint],
    )
    .await?;
    Ok(mint.pubkey())
}

async fn create_token_account(
    context: &mut ProgramTestContext,
    owner: &Keypair,
    mint: &Pubkey,
) -> Result<Pubkey, Error> {
    let address = get_associated_token_address(&owner.pubkey(), mint);
    let instruction = create_associated_token_account(&owner.pubkey(), &owner.pubkey(), mint);
    execute(context, owner, &[instruction], &[owner]).await?;
    Ok(address)
}

async fn mint_token(
    context: &mut ProgramTestContext,
    authority: &Keypair,
    mint: &Pubkey,
    account: &Pubkey,
    amount: u64,
) -> Result<(), Error> {
    let instruction = spl_token::instruction::mint_to(
        &spl_token::ID,
        mint,
        account,
        &authority.pubkey(),
        &[&authority.pubkey()],
        amount,
    )?;
    execute(context, authority, &[instruction], &[authority]).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn initialise_swap(
    context: &mut ProgramTestContext,
    poster: &Keypair,
    sell_from: &Pubkey,
    buy_to: &Pubkey,
    escrow: &Pubkey,
    mint: &Pubkey,
    swap_info: &Pubkey,
    sell_amount: u64,
    buy_amount: u64,
    swap_seed: Vec<u8>,
) -> Result<(), Error> {
    let instruction = escrow::initialize_swap(
        poster.pubkey(),
        *sell_from,
        *buy_to,
        *swap_info,
        swap_seed,
        *escrow,
        *mint,
        sell_amount,
        buy_amount,
    )?;
    execute(context, poster, &[instruction], &[poster]).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn take_swap(
    context: &mut ProgramTestContext,
    taker: &Keypair,
    taker_sell_from: &Pubkey,
    taker_buy_to: &Pubkey,
    swap_info: &Pubkey,
    escrow: &Pubkey,
    poster_buy_to: &Pubkey,
) -> Result<(), Error> {
    let instruction = escrow::take_swap(
        taker.pubkey(),
        *taker_sell_from,
        *taker_buy_to,
        *swap_info,
        *escrow,
        *poster_buy_to,
    );
    execute(context, taker, &[instruction], &[taker]).await?;
    Ok(())
}

async fn execute<T: Signers>(
    context: &mut ProgramTestContext,
    payer: &Keypair,
    instructions: &[Instruction],
    signers: &T,
) -> Result<(), Error> {
    let transaction = Transaction::new_signed_with_payer(
        instructions,
        Some(&payer.pubkey()),
        signers,
        context.banks_client.get_latest_blockhash().await?,
    );
    context
        .banks_client
        .process_transaction(transaction)
        .await?;
    Ok(())
}

async fn token_balance(context: &mut ProgramTestContext, address: Pubkey) -> Result<u64, Error> {
    let account = context
        .banks_client
        .get_account(address)
        .await?
        .ok_or_else(|| "Account not found".to_string())?;
    let info = spl_token::state::Account::unpack(&account.data)?;
    Ok(info.amount)
}
