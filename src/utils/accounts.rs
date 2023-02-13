use anchor_spl::dex::serum_dex::state::OpenOrders;
use cypher_client::{
    constants::SUB_ACCOUNT_ALIAS_LEN,
    instructions::{
        create_account as create_account_ix, create_orders_account as create_orders_account_ix,
        create_sub_account as create_sub_account_ix, init_spot_open_orders,
    },
    utils::{derive_account_address, derive_public_clearing_address, derive_sub_account_address},
    CypherAccount, CypherSubAccount, OrdersAccount,
};
use cypher_utils::utils::{get_cypher_zero_copy_account, get_dex_account, send_transactions};
use log::info;
use solana_client::{client_error::ClientError, nonblocking::rpc_client::RpcClient};
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer};
use std::sync::Arc;

pub async fn get_or_create_orders_account(
    rpc_client: &Arc<RpcClient>,
    authority: &Keypair,
    master_account: &Pubkey,
    market: &Pubkey,
    orders_account: &Pubkey,
) -> Result<Box<OrdersAccount>, ClientError> {
    let account_res = get_orders_account(rpc_client, orders_account).await;

    if account_res.is_ok() {
        info!("Orders Account for market {} already exists.", market);
        Ok(account_res.unwrap())
    } else {
        info!("Could not find Orders Account, attempting to create..");

        let res = create_orders_account(
            rpc_client,
            authority,
            master_account,
            market,
            orders_account,
        )
        .await;

        match res {
            Ok(()) => (),
            Err(e) => {
                return Err(e);
            }
        }

        match get_orders_account(rpc_client, orders_account).await {
            Ok(a) => Ok(a),
            Err(e) => Err(e),
        }
    }
}

async fn create_orders_account(
    rpc_client: &Arc<RpcClient>,
    authority: &Keypair,
    master_account: &Pubkey,
    market: &Pubkey,
    orders_account: &Pubkey,
) -> Result<(), ClientError> {
    let ixs = vec![create_orders_account_ix(
        master_account,
        market,
        orders_account,
        &authority.pubkey(),
        &authority.pubkey(),
    )];
    match send_transactions(
        &rpc_client,
        ixs,
        authority,
        true,
        Some((1_400_000, 1)),
        None,
    )
    .await
    {
        Ok(s) => Ok(()),
        Err(e) => Err(e),
    }
}

async fn get_orders_account(
    rpc_client: &Arc<RpcClient>,
    orders_account: &Pubkey,
) -> Result<Box<OrdersAccount>, ClientError> {
    match get_cypher_zero_copy_account::<OrdersAccount>(&rpc_client, orders_account).await {
        Ok(a) => Ok(a),
        Err(e) => Err(e),
    }
}

pub async fn get_or_create_spot_orders_account(
    rpc_client: &Arc<RpcClient>,
    authority: &Keypair,
    master_account: &Pubkey,
    sub_account: &Pubkey,
    market: &Pubkey,
    pool: &Pubkey,
    token_mint: &Pubkey,
    orders_account: &Pubkey,
) -> Result<OpenOrders, ClientError> {
    let account_res = get_spot_orders_account(rpc_client, orders_account).await;

    if account_res.is_ok() {
        info!("Orders Account for market {} already exists.", market);
        Ok(account_res.unwrap())
    } else {
        info!("Could not find Orders Account, attempting to create..");

        let res = create_spot_orders_account(
            rpc_client,
            authority,
            master_account,
            sub_account,
            market,
            pool,
            token_mint,
            orders_account,
        )
        .await;

        match res {
            Ok(()) => (),
            Err(e) => {
                return Err(e);
            }
        }

        match get_spot_orders_account(rpc_client, orders_account).await {
            Ok(a) => Ok(a),
            Err(e) => Err(e),
        }
    }
}

async fn create_spot_orders_account(
    rpc_client: &Arc<RpcClient>,
    authority: &Keypair,
    master_account: &Pubkey,
    sub_account: &Pubkey,
    market: &Pubkey,
    pool: &Pubkey,
    token_mint: &Pubkey,
    orders_account: &Pubkey,
) -> Result<(), ClientError> {
    let ixs = vec![init_spot_open_orders(
        master_account,
        sub_account,
        pool,
        token_mint,
        market,
        orders_account,
        &authority.pubkey(),
        &authority.pubkey(),
    )];
    match send_transactions(
        &rpc_client,
        ixs,
        authority,
        true,
        Some((1_400_000, 1)),
        None,
    )
    .await
    {
        Ok(s) => Ok(()),
        Err(e) => Err(e),
    }
}

async fn get_spot_orders_account(
    rpc_client: &Arc<RpcClient>,
    orders_account: &Pubkey,
) -> Result<OpenOrders, ClientError> {
    match get_dex_account::<OpenOrders>(&rpc_client, orders_account).await {
        Ok(a) => Ok(a),
        Err(e) => Err(e),
    }
}

pub async fn get_or_create_account(
    rpc_client: &Arc<RpcClient>,
    authority: &Keypair,
    account_number: u8,
) -> Result<(Box<CypherAccount>, Pubkey), ClientError> {
    let (account, account_bump) = derive_account_address(&authority.pubkey(), account_number);

    let account_res = get_account(rpc_client, &account).await;

    if account_res.is_ok() {
        info!("Account number {} already exists.", account_number);
        Ok((account_res.unwrap(), account))
    } else {
        info!(
            "Could not find Account number {}, attempting to create..",
            account_number
        );

        let res = create_account(
            rpc_client,
            authority,
            &account,
            account_bump,
            account_number,
        )
        .await;

        match res {
            Ok(()) => (),
            Err(e) => {
                return Err(e);
            }
        }

        match get_account(rpc_client, &account).await {
            Ok(a) => Ok((a, account)),
            Err(e) => Err(e),
        }
    }
}

async fn create_account(
    rpc_client: &Arc<RpcClient>,
    authority: &Keypair,
    master_account: &Pubkey,
    account_bump: u8,
    account_number: u8,
) -> Result<(), ClientError> {
    let (public_clearing_address, _public_clearing_bump) = derive_public_clearing_address();
    let ixs = vec![create_account_ix(
        &public_clearing_address,
        &authority.pubkey(),
        &authority.pubkey(),
        master_account,
        account_bump,
        account_number,
    )];
    match send_transactions(
        &rpc_client,
        ixs,
        authority,
        true,
        Some((1_400_000, 1)),
        None,
    )
    .await
    {
        Ok(s) => Ok(()),
        Err(e) => Err(e),
    }
}

async fn get_account(
    rpc_client: &Arc<RpcClient>,
    account: &Pubkey,
) -> Result<Box<CypherAccount>, ClientError> {
    match get_cypher_zero_copy_account::<CypherAccount>(&rpc_client, account).await {
        Ok(a) => Ok(a),
        Err(e) => Err(e),
    }
}

pub async fn get_or_create_sub_account(
    rpc_client: &Arc<RpcClient>,
    authority: &Keypair,
    master_account: &Pubkey,
    sub_account_number: u8,
) -> Result<(Box<CypherSubAccount>, Pubkey), ClientError> {
    let (sub_account, sub_account_bump) =
        derive_sub_account_address(master_account, sub_account_number);

    let account_res = get_sub_account(rpc_client, &sub_account).await;

    if account_res.is_ok() {
        info!("Sub Account number {} already exists.", sub_account_number);
        Ok((account_res.unwrap(), sub_account))
    } else {
        info!(
            "Could not find Sub Account number {}, attempting to create..",
            sub_account_number
        );

        let res = create_sub_account(
            rpc_client,
            authority,
            master_account,
            &sub_account,
            sub_account_bump,
            sub_account_number,
            [0; SUB_ACCOUNT_ALIAS_LEN],
        )
        .await;

        match res {
            Ok(()) => (),
            Err(e) => {
                return Err(e);
            }
        }

        match get_sub_account(rpc_client, &sub_account).await {
            Ok(a) => Ok((a, sub_account)),
            Err(e) => Err(e),
        }
    }
}

async fn create_sub_account(
    rpc_client: &Arc<RpcClient>,
    authority: &Keypair,
    master_account: &Pubkey,
    sub_account: &Pubkey,
    sub_account_bump: u8,
    sub_account_number: u8,
    sub_account_alias: [u8; SUB_ACCOUNT_ALIAS_LEN],
) -> Result<(), ClientError> {
    let ixs = vec![create_sub_account_ix(
        &authority.pubkey(),
        &authority.pubkey(),
        master_account,
        sub_account,
        sub_account_bump,
        sub_account_number,
        sub_account_alias,
    )];
    match send_transactions(
        &rpc_client,
        ixs,
        authority,
        true,
        Some((1_400_000, 1)),
        None,
    )
    .await
    {
        Ok(s) => Ok(()),
        Err(e) => Err(e),
    }
}

async fn get_sub_account(
    rpc_client: &Arc<RpcClient>,
    sub_account: &Pubkey,
) -> Result<Box<CypherSubAccount>, ClientError> {
    match get_cypher_zero_copy_account::<CypherSubAccount>(&rpc_client, sub_account).await {
        Ok(a) => Ok(a),
        Err(e) => Err(e),
    }
}
