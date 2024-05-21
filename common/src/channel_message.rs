use solana_sdk::{
    account::Account, clock::Slot, commitment_config::CommitmentLevel, pubkey::Pubkey,
};

use crate::types::{block_meta::BlockMeta, transaction::Transaction};

#[derive(Debug, PartialEq, Eq)]
pub struct AccountData {
    pub pubkey: Pubkey,
    pub account: Account,
    pub write_version: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChannelMessage {
    Account(AccountData, Slot, bool),
    Slot(u64, u64, CommitmentLevel),
    BlockMeta(BlockMeta),
    Transaction(Box<Transaction>),
}
