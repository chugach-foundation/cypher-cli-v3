use async_trait::async_trait;
use cypher_client::cache_account;
use cypher_utils::{
    accounts_cache::{AccountState, AccountsCache},
    contexts::{CacheContext, ContextError, UserContext},
};
use log::{info, warn};
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
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

    fn update_account(&mut self, account: &Pubkey, data: &[u8]) -> Result<(), ContextError> {
        match self.user.reload_account_from_account_data(account, data) {
            Ok(()) => (),
            Err(e) => {
                return Err(e);
            }
        }
        Ok(())
    }

    fn update_sub_account(&mut self, account: &Pubkey, data: &[u8]) -> Result<(), ContextError> {
        match self
            .user
            .reload_sub_account_from_account_data(account, data)
        {
            Ok(()) => (),
            Err(e) => {
                return Err(e);
            }
        }
        Ok(())
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
}

impl GlobalContextBuilder {
    /// Creates a new [`GlobalContextBuilder`].
    pub fn new(accounts_cache: Arc<AccountsCache>, account: Pubkey, sub_account: Pubkey) -> Self {
        Self {
            accounts_cache,
            account,
            sub_account,
            state: RwLock::new(GlobalContextState::default()),
            update_sender: Arc::new(channel::<GlobalContext>(1).0),
        }
    }
}

#[async_trait]
impl ContextBuilder for GlobalContextBuilder {
    type Output = GlobalContext;

    async fn start(&self) -> Result<(), ContextBuilderError> {
        let mut sub = self.accounts_cache.subscribe();

        loop {
            tokio::select! {
                account_state_update = sub.recv() => {
                    if account_state_update.is_err() {
                        warn!("[GCTX-BLDR] There was an error receiving account state update.");
                        continue;
                    } else {
                        let account_state = account_state_update.unwrap();
                        self.process_update(&account_state).await;
                        self.send().await;
                    }
                }
            }
        }
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
            info!("[GCTX-BLDR] Sucessfully processed cache account update.");
        }

        // check if this account is the user account
        if account_state.account == self.account {
            let mut state = self.state.write().await;
            match state.update_account(&self.account, &account_state.data) {
                Ok(()) => {
                    info!("[GCTX-BLDR] Sucessfully processed cypher account update.");
                }
                Err(e) => {
                    return Err(ContextBuilderError::ProcessUpdateError(
                        "account".to_string(),
                    ))
                }
            }
        }

        // check if this account is the user account
        if account_state.account == self.sub_account {
            let mut state = self.state.write().await;
            match state.update_sub_account(&self.account, &account_state.data) {
                Ok(()) => {
                    info!("[GCTX-BLDR] Sucessfully processed cypher sub account update.");
                    Ok(())
                }
                Err(e) => {
                    return Err(ContextBuilderError::ProcessUpdateError(
                        "sub account".to_string(),
                    ))
                }
            }
        } else {
            Ok(())
        }
    }

    fn subscribe(&self) -> Receiver<GlobalContext> {
        self.update_sender.subscribe()
    }

    fn symbol(&self) -> String {
        "GLOBAL".to_string()
    }
}
