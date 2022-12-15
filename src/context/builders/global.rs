use async_trait::async_trait;
use cypher_client::cache_account;
use cypher_utils::{
    accounts_cache::{AccountState, AccountsCache},
    contexts::{CacheContext, ContextError, UserContext},
};
use log::{info, warn};
use solana_sdk::pubkey::Pubkey;
use std::{any::type_name, sync::Arc};
use tokio::sync::{
    broadcast::{channel, Receiver, Sender},
    RwLock,
};

use crate::common::context::{
    builder::{ContextBuilder, ContextBuilderError},
    GlobalContext,
};

/// The global context state.
#[derive(Default, Clone)]
pub struct GlobalContextState {
    /// The cache context.
    pub cache: CacheContext,
    /// The user context.
    pub user: UserContext,
}

impl GlobalContextState {
    fn update_cache(&mut self, data: &[u8]) {
        self.cache.reload_from_account_data(data);
    }

    fn update_account(&mut self, account: &Pubkey, data: &[u8]) {
        self.user.reload_account_from_account_data(account, data);
    }

    fn update_sub_account(&mut self, account: &Pubkey, data: &[u8]) {
        self.user
            .reload_sub_account_from_account_data(account, data);
    }
}

/// Builds and maintains an updated operation context for the market maker at all times.
///
/// Receives updates from the [AccountsCache] about the user accounts.
pub struct GlobalContextBuilder {
    accounts_cache: Arc<AccountsCache>,
    account: Pubkey,
    sub_account: Pubkey,
    state: RwLock<GlobalContextState>,
    update_sender: Arc<Sender<GlobalContext>>,
    shutdown_sender: Arc<Sender<bool>>,
}

impl GlobalContextBuilder {
    /// Creates a new [`GlobalContextBuilder`].
    pub fn new(
        accounts_cache: Arc<AccountsCache>,
        shutdown_sender: Arc<Sender<bool>>,
        account: Pubkey,
        sub_account: Pubkey,
    ) -> Self {
        Self {
            accounts_cache,
            shutdown_sender,
            account,
            sub_account,
            state: RwLock::new(GlobalContextState::default()),
            update_sender: Arc::new(channel::<GlobalContext>(50).0),
        }
    }
}

#[async_trait]
impl ContextBuilder for GlobalContextBuilder {
    type Output = GlobalContext;

    async fn cache_receiver(&self) -> Receiver<AccountState> {
        let accounts = vec![self.account, self.sub_account, cache_account::id()];
        self.accounts_cache.subscribe(&accounts).await
    }

    fn shutdown_receiver(&self) -> Receiver<bool> {
        self.shutdown_sender.subscribe()
    }

    async fn send(&self) -> Result<(), ContextBuilderError> {
        let ctx = self.state.read().await;
        match self.update_sender.send(GlobalContext {
            cache: ctx.cache.clone(),
            user: ctx.user.clone(),
        }) {
            Ok(_) => Ok(()),
            Err(e) => Err(ContextBuilderError::GlobalContextSendError(e)),
        }
    }

    /// Returns the process update of this [`GlobalContextBuilder`].
    async fn process_update(
        &self,
        account_state: &AccountState,
    ) -> Result<(), ContextBuilderError> {
        // check if this account is the cache account
        if account_state.account == cache_account::id() {
            let mut state = self.state.write().await;
            state.update_cache(&account_state.data);
            info!(
                "{} - Sucessfully processed cache account update.",
                type_name::<Self>(),
            );
            return Ok(());
        }

        // check if this account is the user account
        if account_state.account == self.account {
            let mut state = self.state.write().await;
            state.update_account(&self.account, &account_state.data);
            info!(
                "{} - Sucessfully processed cypher account update.",
                type_name::<Self>(),
            );
            return Ok(());
        }

        // check if this account is the user account
        if account_state.account == self.sub_account {
            let mut state = self.state.write().await;
            state.update_sub_account(&self.sub_account, &account_state.data);
            info!(
                "{} - Sucessfully processed cypher sub account update.",
                type_name::<Self>(),
            );
            return Ok(());
        } else {
            Err(ContextBuilderError::UnrecognizedAccount(
                account_state.account,
            ))
        }
    }

    fn sender(&self) -> Arc<Sender<GlobalContext>> {
        self.update_sender.clone()
    }

    fn subscribe(&self) -> Receiver<GlobalContext> {
        self.update_sender.subscribe()
    }

    fn symbol(&self) -> &str {
        "GlobalContext"
    }
}
