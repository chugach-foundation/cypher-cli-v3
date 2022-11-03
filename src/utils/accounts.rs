use cypher_client::{
    instructions::create_orders_account as create_orders_account_ix, OrdersAccount,
};
use cypher_utils::utils::{get_cypher_zero_copy_account, send_transactions};
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
        println!("Orders Account for market {} already exists.", market);
        Ok(account_res.unwrap())
    } else {
        println!("Could not find Orders Account, attempting to create..");

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
    match send_transactions(&rpc_client, ixs, authority, true).await {
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
