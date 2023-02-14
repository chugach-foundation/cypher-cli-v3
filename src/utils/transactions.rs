
use cypher_utils::{transaction_builder::TransactionBuilder, utils::send_transaction};
use log::warn;
use solana_client::{client_error::ClientError, nonblocking::rpc_client::RpcClient};
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    signature::{Keypair, Signature},
};

use crate::common::orders::{CandidateCancel, CandidatePlacement};

#[derive(Debug, Default, Clone)]
pub struct TransactionInfo<T> {
    pub candidates: Vec<T>,
    pub signature: Option<Signature>,
}

#[inline(always)]
pub async fn send_cancels(
    rpc_client: &RpcClient,
    candidates: Vec<CandidateCancel>,
    ixs: Vec<Instruction>,
    signer: &Keypair,
    confirm: bool,
) -> Result<Vec<TransactionInfo<CandidateCancel>>, ClientError> {
    let mut txn_builder = TransactionBuilder::new();
    add_priority_fees(&mut txn_builder);
    let mut submitted: bool = false;
    let mut signatures: Vec<TransactionInfo<CandidateCancel>> = Vec::new();
    let mut prev_tx_idx = 0;

    let blockhash = match rpc_client.get_latest_blockhash().await {
        Ok(h) => h,
        Err(e) => {
            return Err(e);
        }
    };

    for (idx, ix) in ixs.iter().enumerate() {
        if txn_builder.len() != 0 {
            let tx = txn_builder.build(blockhash, signer, None);
            // we do this to attempt to pack as many ixs in a tx as possible
            // there's more efficient ways to do it but we'll do it in the future
            if tx.message_data().len() > 1100 {
                let res = send_transaction(rpc_client, &tx, confirm).await;
                match res {
                    Ok(s) => {
                        let (_, rest) = candidates.split_at(prev_tx_idx);
                        let idx = if idx > rest.len() { rest.len() } else { idx };
                        let (split, _) = rest.split_at(idx);
                        signatures.push(TransactionInfo::<CandidateCancel> {
                            candidates: split.to_vec(),
                            signature: Some(s),
                        });
                        prev_tx_idx = idx;
                        submitted = true;
                        txn_builder.clear();
                        add_priority_fees(&mut txn_builder);
                        txn_builder.add(ix.clone());
                    }
                    Err(e) => {
                        warn!("There was an error submitting transaction: {:?}", e);

                        let (_, rest) = candidates.split_at(prev_tx_idx);
                        let (split, _) = rest.split_at(idx);
                        signatures.push(TransactionInfo::<CandidateCancel> {
                            candidates: split.to_vec(),
                            signature: None,
                        });
                    }
                }
            } else {
                txn_builder.add(ix.clone());
            }
        } else {
            txn_builder.add(ix.clone());
        }
    }

    if !submitted || txn_builder.len() != 0 {
        let tx = txn_builder.build(blockhash, signer, None);
        let res = send_transaction(rpc_client, &tx, confirm).await;
        match res {
            Ok(s) => {
                let (_, rest) = candidates.split_at(prev_tx_idx);
                signatures.push(TransactionInfo::<CandidateCancel> {
                    candidates: rest.to_vec(),
                    signature: Some(s),
                });
            }
            Err(e) => {
                warn!("There was an error submitting transaction: {:?}", e);
                let tx_err = e.get_transaction_error();
                if tx_err.is_some() {
                    let err = tx_err.unwrap();
                    warn!("Error: {}", err.to_string());
                }
                let (_, rest) = candidates.split_at(prev_tx_idx);
                signatures.push(TransactionInfo::<CandidateCancel> {
                    candidates: rest.to_vec(),
                    signature: None,
                });
                return Err(e);
            }
        }
    }

    Ok(signatures)
}

#[inline(always)]
pub async fn send_placements(
    rpc_client: &RpcClient,
    candidates: Vec<CandidatePlacement>,
    ixs: Vec<Instruction>,
    signer: &Keypair,
    confirm: bool,
) -> Result<Vec<TransactionInfo<CandidatePlacement>>, ClientError> {
    let mut txn_builder = TransactionBuilder::new();
    add_priority_fees(&mut txn_builder);
    let mut submitted: bool = false;
    let mut signatures: Vec<TransactionInfo<CandidatePlacement>> = Vec::new();
    let mut prev_tx_idx = 0;

    let blockhash = match rpc_client.get_latest_blockhash().await {
        Ok(h) => h,
        Err(e) => {
            return Err(e);
        }
    };

    for (idx, ix) in ixs.iter().enumerate() {
        if txn_builder.len() != 0 {
            let tx = txn_builder.build(blockhash, signer, None);
            // we do this to attempt to pack as many ixs in a tx as possible
            // there's more efficient ways to do it but we'll do it in the future
            if tx.message_data().len() > 1100 {
                let res = send_transaction(rpc_client, &tx, confirm).await;
                match res {
                    Ok(s) => {
                        let (_, rest) = candidates.split_at(prev_tx_idx);
                        let idx = if idx > rest.len() { rest.len() } else { idx };
                        let (split, _) = rest.split_at(idx);
                        signatures.push(TransactionInfo::<CandidatePlacement> {
                            candidates: split.to_vec(),
                            signature: Some(s),
                        });
                        prev_tx_idx = idx;
                        submitted = true;
                        txn_builder.clear();
                        add_priority_fees(&mut txn_builder);
                        txn_builder.add(ix.clone());
                    }
                    Err(e) => {
                        warn!("There was an error submitting transaction: {:?}", e);
                    }
                }
            } else {
                txn_builder.add(ix.clone());
            }
        } else {
            txn_builder.add(ix.clone());
        }
    }

    if !submitted || txn_builder.len() != 0 {
        let tx = txn_builder.build(blockhash, signer, None);
        let res = send_transaction(rpc_client, &tx, confirm).await;
        match res {
            Ok(s) => {
                let (_, rest) = candidates.split_at(prev_tx_idx);
                signatures.push(TransactionInfo::<CandidatePlacement> {
                    candidates: rest.to_vec(),
                    signature: Some(s),
                });
            }
            Err(e) => {
                warn!("There was an error submitting transaction: {:?}", e);
                let tx_err = e.get_transaction_error();
                if tx_err.is_some() {
                    let err = tx_err.unwrap();
                    warn!("Error: {}", err.to_string());
                }
                let (_, rest) = candidates.split_at(prev_tx_idx);
                signatures.push(TransactionInfo::<CandidatePlacement> {
                    candidates: rest.to_vec(),
                    signature: None,
                });
                return Err(e);
            }
        }
    }

    Ok(signatures)
}

fn add_priority_fees(txn_builder: &mut TransactionBuilder) {
    txn_builder.add(ComputeBudgetInstruction::set_compute_unit_limit(1_400_000));
    txn_builder.add(ComputeBudgetInstruction::set_compute_unit_price(1));
}
