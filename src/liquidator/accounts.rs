use anchor_lang::prelude::Pubkey;
use cypher_client::{utils::get_zero_copy_account, CypherAccount, CypherSubAccount};
use cypher_utils::{
    accounts_cache::{AccountState, AccountsCache},
    contexts::{AccountContext, SubAccountContext, UserContext},
    services::StreamingAccountInfoService,
    utils::get_program_accounts_without_data,
};
use dashmap::DashMap;
use log::{info, warn};
use solana_client::{
    client_error::ClientError, nonblocking::rpc_client::RpcClient, rpc_filter::RpcFilterType,
};
use solana_sdk::{account::Account, commitment_config::CommitmentConfig};

use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::broadcast::{channel, Sender};

use super::error::Error;

pub struct CypherAccountsService {
    pub rpc_client: Arc<RpcClient>,
    pub streaming_accounts: Arc<StreamingAccountInfoService>,
    pub accounts_cache: Arc<AccountsCache>,
    pub shutdown_sender: Arc<Sender<bool>>,
    /// map from authority to user context
    pub users_map: DashMap<Pubkey, UserContext>,
    pub all_accounts_map: DashMap<Pubkey, bool>,
    pub all_sub_accounts_map: DashMap<Pubkey, bool>,
    pub subscribed_accounts_map: DashMap<Pubkey, bool>,
    pub subscribed_sub_accounts_map: DashMap<Pubkey, bool>,
    pub update_sender: Arc<Sender<UserContext>>,
}

impl CypherAccountsService {
    pub fn new(
        rpc_client: Arc<RpcClient>,
        streaming_accounts: Arc<StreamingAccountInfoService>,
        accounts_cache: Arc<AccountsCache>,
        shutdown_sender: Arc<Sender<bool>>,
    ) -> Self {
        Self {
            rpc_client,
            streaming_accounts,
            accounts_cache,
            shutdown_sender,
            users_map: DashMap::new(),
            all_accounts_map: DashMap::new(),
            all_sub_accounts_map: DashMap::new(),
            subscribed_accounts_map: DashMap::new(),
            subscribed_sub_accounts_map: DashMap::new(),
            update_sender: Arc::new(channel::<UserContext>(u16::MAX as usize).0),
        }
    }

    pub async fn start(self: &Arc<Self>) -> Result<(), Error> {
        let aself = Arc::clone(self);

        let aself_clone = aself.clone();
        let updates_handler = tokio::spawn(async move {
            aself_clone.process_updates().await;
        });

        let mut shutdown_receiver = aself.shutdown_sender.subscribe();
        // let's start off fetching all accounts every minute
        let mut interval = tokio::time::interval(Duration::from_secs(60));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let start = SystemTime::now();
                    match aself.get_all_accounts().await {
                        Ok(()) => info!("Successfully fetched all accounts."),
                        Err(e) => {
                            warn!("There was an error attempting to fetch all accounts. Error: {:?}", e);
                        }
                    };
                    let delta = start.elapsed().unwrap();
                    info!("Time elapsed fetching all accounts: {:?}", delta);

                },
                _ = shutdown_receiver.recv() => {
                    info!("Received shutdown signal, stopping.");
                    break;
                }
            }
        }

        updates_handler.await.unwrap();
        Ok(())
    }

    pub async fn process_updates(self: &Arc<Self>) {
        let mut shutdown_receiver = self.shutdown_sender.subscribe();
        let mut update_receiver = self.accounts_cache.subscribe_all();

        loop {
            tokio::select! {
                account_update = update_receiver.recv() => {
                    match account_update {
                        Ok(account_state) => {
                            match self.process_update(&account_state) {
                                Ok(user_ctx) => {
                                    match self.update_sender.send(user_ctx) {
                                        Ok(r) => info!("Successfully sent user context update to {} receiver(s).", r),
                                        Err(e) => warn!("There was an error sending user context update. Error {:?}", e)
                                    }
                                },
                                Err(e) => {
                                    warn!("There was an error processing account update. Error: {:?}", e);
                                }
                            }
                        },
                        Err(e) => {
                            warn!("There was an error receiving account update. Error: {:?}", e);
                        }
                    }
                }
                _ = shutdown_receiver.recv() => {
                    info!("Received shutdown signal, stopping.");
                    break;
                }
            }
        }
    }

    pub fn process_update(
        self: &Arc<Self>,
        account_state: &AccountState,
    ) -> Result<UserContext, Error> {
        if self.all_accounts_map.contains_key(&account_state.account) {
            let account = get_zero_copy_account::<CypherAccount>(&account_state.data);

            let user_ctx = match self.users_map.get_mut(&account.authority) {
                Some(mut user_ctx) => {
                    user_ctx.reload_account_from_account_data(
                        &account_state.account,
                        &account_state.data,
                    );
                    user_ctx.clone()
                }
                None => {
                    info!("User {} not found, adding to cache.", account.authority);
                    let mut user_ctx = UserContext {
                        authority: account.authority,
                        ..Default::default()
                    };
                    user_ctx.account_ctx = AccountContext::new(account_state.account, account);
                    let clone = user_ctx.clone();
                    self.users_map.insert(user_ctx.authority, user_ctx);
                    clone
                }
            };
            return Ok(user_ctx);
        }

        if self
            .all_sub_accounts_map
            .contains_key(&account_state.account)
        {
            let sub_account = get_zero_copy_account::<CypherSubAccount>(&account_state.data);

            let user_ctx = match self.users_map.get_mut(&sub_account.authority) {
                Some(mut user_ctx) => {
                    user_ctx.reload_sub_account_from_account_data(
                        &account_state.account,
                        &account_state.data,
                    );
                    user_ctx.clone()
                }
                None => {
                    info!("User {} not found, adding to cache.", sub_account.authority);
                    let mut user_ctx = UserContext {
                        authority: sub_account.authority,
                        ..Default::default()
                    };
                    user_ctx
                        .sub_account_ctxs
                        .push(SubAccountContext::new(account_state.account, sub_account));
                    let clone = user_ctx.clone();
                    self.users_map.insert(user_ctx.authority, user_ctx);
                    clone
                }
            };
            return Ok(user_ctx);
        }

        Err(Error::UnrecognizedAccount)
    }

    pub async fn get_all_accounts(self: &Arc<Self>) -> Result<(), ClientError> {
        let accounts_filters = vec![RpcFilterType::DataSize(
            std::mem::size_of::<CypherAccount>() as u64 + 8,
        )];

        info!("Fetching all user accounts..");

        let accounts = match get_program_accounts_without_data(
            &self.rpc_client,
            accounts_filters,
            &cypher_client::id(),
        )
        .await
        {
            Ok(a) => a,
            Err(e) => {
                return Err(e);
            }
        };

        let mut account_keys = Vec::new();

        for (account_key, _) in accounts.iter() {
            if !self.all_accounts_map.contains_key(account_key) {
                info!("Adding subscription for account: {}", account_key);
                account_keys.push(*account_key);
                self.all_accounts_map.insert(*account_key, true);
            }
        }

        info!("Adding subscription for {} accounts.", account_keys.len());

        let sub_accounts_filters = vec![RpcFilterType::DataSize(
            std::mem::size_of::<CypherSubAccount>() as u64 + 8,
        )];

        info!("Fetching all user sub accounts..");

        let sub_accounts = match get_program_accounts_without_data(
            &self.rpc_client,
            sub_accounts_filters,
            &cypher_client::id(),
        )
        .await
        {
            Ok(a) => a,
            Err(e) => {
                return Err(e);
            }
        };

        let mut sub_account_keys = Vec::new();

        for (account_key, _) in sub_accounts.iter() {
            if !self.all_sub_accounts_map.contains_key(account_key) {
                info!("Adding subscription for account: {}", account_key);
                sub_account_keys.push(*account_key);
                self.all_sub_accounts_map.insert(*account_key, true);
            }
        }

        info!(
            "Adding subscription for {} sub accounts.",
            sub_account_keys.len()
        );

        let user_ctxs = match self
            .get_multiple_accounts_repeat(&account_keys, &sub_account_keys)
            .await
        {
            Ok(a) => a,
            Err(e) => {
                return Err(e);
            }
        };

        self.streaming_accounts
            .add_subscriptions(&[account_keys, sub_account_keys].concat())
            .await;

        Ok(())
    }

    async fn get_multiple_accounts_repeat(
        self: &Arc<Self>,
        account_keys: &[Pubkey],
        sub_account_keys: &[Pubkey],
    ) -> Result<Vec<UserContext>, ClientError> {
        let mut sub_account_ctxs = Vec::new();

        for i in (0..sub_account_keys.len()).step_by(100) {
            let mut pubkeys: Vec<Pubkey> = Vec::new();
            pubkeys.extend(sub_account_keys[i..sub_account_keys.len().min(i + 100)].iter());

            let accounts_res = self.get_multiple_accounts(&pubkeys).await;
            let accounts = match accounts_res {
                Ok(a) => a,
                Err(e) => {
                    warn!("Could not fetch cypher sub accounts: {}", e);
                    return Err(e);
                }
            };

            info!("Fetched {} cypher accounts.", accounts.len());

            for (idx, maybe_account) in accounts.iter().enumerate() {
                if let Some(account) = maybe_account {
                    let sub_account_state =
                        get_zero_copy_account::<CypherSubAccount>(&account.data);
                    let sub_account_ctx =
                        SubAccountContext::new(sub_account_keys[idx], sub_account_state);
                    sub_account_ctxs.push(sub_account_ctx);
                }
            }
        }

        let mut user_ctxs = Vec::new();

        for i in (0..account_keys.len()).step_by(100) {
            let mut pubkeys: Vec<Pubkey> = Vec::new();
            pubkeys.extend(account_keys[i..account_keys.len().min(i + 100)].iter());

            let accounts_res = self.get_multiple_accounts(&pubkeys).await;
            let accounts = match accounts_res {
                Ok(a) => a,
                Err(e) => {
                    warn!("Could not fetch cypher accounts: {}", e.to_string());
                    return Err(e);
                }
            };

            info!("Fetched {} cypher accounts.", accounts.len());

            for (idx, maybe_account) in accounts.iter().enumerate() {
                if let Some(account) = maybe_account {
                    let account_state = get_zero_copy_account::<CypherAccount>(&account.data);
                    let sub_account_pubkeys = account_state
                        .sub_account_caches
                        .iter()
                        .filter(|sa| sa.sub_account != Pubkey::default())
                        .map(|sac| sac.sub_account)
                        .collect::<Vec<_>>();
                    let authority = account_state.authority.clone();
                    let account_ctx = AccountContext::new(account_keys[idx], account_state);
                    let filtered_sub_accounts_ctxs = sub_account_ctxs
                        .iter()
                        .filter(|sa| sub_account_pubkeys.contains(&sa.address))
                        .map(|sa| sa.clone())
                        .collect::<Vec<_>>();
                    let user_ctx =
                        UserContext::new(authority, account_ctx, filtered_sub_accounts_ctxs);
                    user_ctxs.push(user_ctx);
                }
            }
        }
        Ok(user_ctxs)
    }

    async fn get_multiple_accounts(
        self: &Arc<Self>,
        pubkeys: &[Pubkey],
    ) -> Result<Vec<Option<Account>>, ClientError> {
        let accounts_res = self
            .rpc_client
            .get_multiple_accounts_with_commitment(pubkeys, CommitmentConfig::confirmed())
            .await;

        let accounts = match accounts_res {
            Ok(a) => a.value,
            Err(e) => {
                warn!("Could not fetch account: {:?}", e);
                return Err(e);
            }
        };
        Ok(accounts)
    }
}
