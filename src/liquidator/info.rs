use anchor_lang::prelude::Pubkey;
use cypher_client::{CypherAccount, CypherSubAccount, SubAccountMargining};

#[derive(Default, Clone)]
pub struct UserAccountsInfo {
    pub account: CypherAccount,
    pub account_address: Pubkey,
    pub sub_accounts: Vec<CypherSubAccount>,
    pub sub_account_addresses: Vec<Pubkey>,
}

impl UserAccountsInfo {
    pub fn get_isolated(&self) -> (Vec<Pubkey>, Vec<&CypherSubAccount>) {
        let sub_accounts: Vec<_> = self
            .sub_accounts
            .iter()
            .enumerate()
            .filter(|(_, s)| s.margining_type == SubAccountMargining::Isolated)
            .collect();

        let account_keys: Vec<_> = sub_accounts
            .iter()
            .map(|(idx, sa)| self.sub_account_addresses[*idx])
            .collect();
        (
            account_keys.to_vec(),
            sub_accounts.iter().map(|(_, s)| s.clone()).collect(),
        )
    }

    pub fn get_cross(&self) -> (Vec<Pubkey>, Vec<&CypherSubAccount>) {
        let sub_accounts: Vec<_> = self
            .sub_accounts
            .iter()
            .enumerate()
            .filter(|(_, s)| s.margining_type == SubAccountMargining::Cross)
            .collect();

        let account_keys: Vec<_> = sub_accounts
            .iter()
            .map(|(idx, sa)| self.sub_account_addresses[*idx])
            .collect();
        (
            account_keys.to_vec(),
            sub_accounts.iter().map(|(_, s)| s.clone()).collect(),
        )
    }
}
