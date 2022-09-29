use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use borsh::{BorshDeserialize, BorshSchema, BorshSerialize};
use serde::{Deserialize, Serialize};
use solana_sdk::{pubkey::Pubkey, system_program, sysvar::rent, instruction::{AccountMeta, Instruction}};
use voter_stake_registry::state::LockupKind;
use clap::{arg, command, value_parser, ArgAction, Command};
use solana_remote_wallet::locator::Locator;
use solana_remote_wallet::remote_keypair::generate_remote_keypair;
use solana_remote_wallet::remote_wallet::maybe_wallet_manager;
use solana_sdk::derivation_path::DerivationPath;
use solana_sdk::{
    self, commitment_config::CommitmentConfig, signature::Keypair, signature::Signer,
    transaction::Transaction as SolanaTransaction, signer::keypair::read_keypair_file
};
use uriparse::URIReference;

use dotenv::dotenv;

use std::{env, fs, str::FromStr, path::PathBuf};

#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize, BorshSchema)]
#[repr(C)]
pub struct LocalInstructionData {
    /// Pubkey of the instruction processor that executes this instruction
    pub program_id: Pubkey,
    /// Metadata for what accounts should be passed to the instruction processor
    pub accounts: Vec<AccountMetaData>,
    /// Opaque data passed to the instruction processor
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize, BorshSchema)]
#[repr(C)]
pub struct AccountMetaData {
    /// An account's public key
    pub pubkey: Pubkey,
    /// True if an Instruction requires a Transaction signature matching `pubkey`.
    pub is_signer: bool,
    /// True if the `pubkey` can be loaded as a read-write account.
    pub is_writable: bool,
}

impl From<Instruction> for LocalInstructionData {
    fn from(instruction: Instruction) -> Self {
        LocalInstructionData {
            program_id: instruction.program_id,
            accounts: instruction
                .accounts
                .iter()
                .map(|a| AccountMetaData {
                    pubkey: a.pubkey,
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
                .collect(),
            data: instruction.data,
        }
    }
}

impl From<&LocalInstructionData> for Instruction {
    fn from(instruction: &LocalInstructionData) -> Self {
        Instruction {
            program_id: instruction.program_id,
            accounts: instruction
                .accounts
                .iter()
                .map(|a| AccountMeta {
                    pubkey: a.pubkey,
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
                .collect(),
            data: instruction.data.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum GrantType {
    None,
    Daily,
    Monthly,
    Cliff,
    Constant,
}

impl From<&GrantType> for LockupKind {
    fn from(lockup: &GrantType) -> Self {
        match lockup {
            GrantType::None => LockupKind::None,
            GrantType::Daily => LockupKind::Daily,
            GrantType::Monthly => LockupKind::Monthly,
            GrantType::Cliff => LockupKind::Cliff,
            GrantType::Constant => LockupKind::Constant,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Grant {
    pub wallet: String,
    pub grant_type: GrantType,
    pub start: Option<u64>,
    pub periods: u32,
    pub allow_clawback: bool,
    pub amount: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProposalData {
    pub name: String,
    pub description: String,
    pub grants: Vec<Grant>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Transaction {
    pub wallet: String,
    pub grant_type: GrantType,
    pub start: Option<u64>,
    pub periods: u32,
    pub allow_clawback: bool,
    pub amount: u64,
    pub tx: String,
}

pub type Transactions = Vec<Transaction>;

fn main() {
    dotenv().ok();

    // let grants_data = fs::read_to_string("./grants.json").unwrap();

    // let grants: Grants = serde_json::from_str(&grants_data).unwrap();

    // let transactions = grant_transactions(grants);

    // let j = serde_json::to_string(&transactions).unwrap();

    // fs::write("./transactions.json", j).unwrap();

    let matches = command!()
        .arg(
            arg!(
                -w --wallet <FILE> "Fee payer wallet"
            )
            .value_parser(value_parser!(PathBuf)),
        )
        .subcommand(
            Command::new("grant")
                .about("creates new DAO proposal and attaches bunch of Grant transactions to it")
                .arg(arg!(-g --grants "lists of grants to be created").required(true).action(ArgAction::Set))
        )
        .get_matches();

    let wallet_path = matches.get_one::<PathBuf>("wallet").unwrap();

    if let Some(matches) = matches.subcommand_matches("grant") {
        let grants_file = matches.get_one::<String>("grants").unwrap();

        let grants_data = fs::read_to_string(grants_file).unwrap();

        let grants: ProposalData = serde_json::from_str(&grants_data).unwrap();
    }
}

fn create_proposal(name: String, description: String, grants: Vec<Instruction>) {
    // to get proposal_index call RPC to get governance_data.proposals_count
    // proposal_owner_record - Account PDA seeds: ['governance', realm, token_mint, token_owner ]
    // governance_auth will be the wallet

}

fn keypair_or_ledger_of(path: &PathBuf) -> Option<Box<dyn Signer>> {
    return if path.starts_with("usb://") {
        let uri_invalid_msg =
            "Failed to parse usb:// keypair path. It must be of the form 'usb://ledger?key=0'.";
        let uri_ref = URIReference::try_from(path.to_str().unwrap()).expect(uri_invalid_msg);
        let derivation_path = DerivationPath::from_uri_key_query(&uri_ref)
            .expect(uri_invalid_msg)
            .unwrap_or_default();
        let locator = Locator::new_from_uri(&uri_ref).expect(uri_invalid_msg);

        let hw_wallet = maybe_wallet_manager()
            .expect("Remote wallet found, but failed to establish protocol. Maybe the Solana app is not open.")
            .expect("Failed to find a remote wallet, maybe Ledger is not connected or locked.");

        // When using a Ledger hardware wallet, confirm the public key of the
        // key to sign with on its display, so users can be sure that they
        // selected the right key.
        let confirm_public_key = true;

        Some(Box::new(
            generate_remote_keypair(
                locator,
                derivation_path,
                &hw_wallet,
                confirm_public_key,
                "council", /* When multiple wal
                            lets are connected, used to display a hint */
            )
            .expect("Failed to contact remote wallet"),
        ))
    } else {
        Some(Box::new(read_keypair_file(path.to_str().unwrap()).unwrap()))
    };
}

pub fn grant_transactions(grants: Vec<Grant>) -> Vec<Instruction> {
    let voter_stake_program = Pubkey::from_str(&env::var("VOTER_STAKE_PROGRAM").unwrap()).unwrap();

    let mint = Pubkey::from_str(&env::var("MINT").unwrap()).unwrap();

    let registrar = Pubkey::from_str(&env::var("REGISTRAR").unwrap()).unwrap();

    let deposit_token = Pubkey::from_str(&env::var("DEPOSIT_TOKEN").unwrap()).unwrap();

    let deposit_token_auth = Pubkey::from_str(&env::var("DEPOSIT_TOKEN_AUTH").unwrap()).unwrap();

    let realm_auth = Pubkey::from_str(&env::var("REALM_AUTH").unwrap()).unwrap();

    let payer = Pubkey::from_str(&env::var("PAYER").unwrap()).unwrap();

    let mut instructions = Vec::new();

    for grant in grants.iter() {
        // wallet
        let voter_authority = Pubkey::from_str(&grant.wallet).unwrap();

        let (voter, voter_bump) = Pubkey::find_program_address(
            &[
                registrar.as_ref(),
                "voter".as_bytes(),
                voter_authority.as_ref(),
            ],
            &voter_stake_program,
        );

        let (voter_weight_record, voter_weight_record_bump) = Pubkey::find_program_address(
            &[
                registrar.as_ref(),
                "voter-weight-record".as_bytes(),
                voter_authority.as_ref(),
            ],
            &voter_stake_program,
        );

        let vault = spl_associated_token_account::get_associated_token_address(&voter, &mint);

        let accounts = voter_stake_registry::accounts::Grant {
            registrar,
            voter,
            voter_authority,
            voter_weight_record,
            vault,
            deposit_token,
            token_authority: deposit_token_auth,
            grant_authority: realm_auth,
            payer,
            deposit_mint: mint,
            system_program: system_program::id(),
            token_program: spl_token::id(),
            associated_token_program: spl_associated_token_account::id(),
            rent: rent::id(),
        }
        .to_account_metas(None);

        let data = voter_stake_registry::instruction::Grant {
            voter_bump,
            voter_weight_record_bump,
            kind: (&grant.grant_type).into(),
            start_ts: grant.start,
            periods: grant.periods,
            allow_clawback: grant.allow_clawback,
            amount: grant.amount,
        }
        .data();

        let instruction = Instruction {
            program_id: voter_stake_program,
            data,
            accounts,
        };

        instructions.push(instruction);
    }

    instructions
}
