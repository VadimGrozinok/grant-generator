use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use borsh::{BorshDeserialize, BorshSchema, BorshSerialize};
use clap::{arg, command, value_parser, ArgAction, Command};
use serde::{Deserialize, Serialize};
use solana_remote_wallet::locator::Locator;
use solana_remote_wallet::remote_keypair::generate_remote_keypair;
use solana_remote_wallet::remote_wallet::maybe_wallet_manager;
use solana_sdk::{
    derivation_path::DerivationPath,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Signer,
    signer::keypair::read_keypair_file,
    system_program,
    sysvar::rent,
};
use uriparse::URIReference;
use voter_stake_registry::state::LockupKind;

use dotenv::dotenv;

use std::{
    env, fs,
    path::{Path, PathBuf},
    str::FromStr,
};

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
pub struct ProposalData<T: Serialize> {
    pub name: String,
    pub description: String,
    pub grants: Vec<T>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GrantInstruction {
    pub wallet: String,
    pub grant_type: GrantType,
    pub start: Option<u64>,
    pub periods: u32,
    pub allow_clawback: bool,
    pub amount: u64,
    pub instruction: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WithdrawInstruction {
    pub wallet: String,
    pub instruction: Vec<u8>,
}

fn main() {
    dotenv().ok();

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
                .arg(
                    arg!(-g --grants "lists of grants to be created")
                        .required(true)
                        .action(ArgAction::Set),
                ),
        )
        .subcommand(
            Command::new("withdraw")
                .about("withdraw locked tokens")
                .arg(
                    arg!(-d --deposit "deposit index")
                        .required(true)
                        .value_parser(clap::value_parser!(u8))
                        .action(ArgAction::Set),
                )
                .arg(
                    arg!(-a --amount "amount to withdraw")
                        .required(true)
                        .value_parser(clap::value_parser!(u64))
                        .action(ArgAction::Set),
                ),
        )
        .get_matches();

    let wallet_path = matches.get_one::<PathBuf>("wallet").unwrap();

    let signer = keypair_or_ledger_of(wallet_path);

    if let Some(matches) = matches.subcommand_matches("grant") {
        let grants_file = matches.get_one::<String>("grants").unwrap();

        let grants_data = fs::read_to_string(grants_file).unwrap();

        let grants: ProposalData<Grant> = serde_json::from_str(&grants_data).unwrap();

        let instructions = grant_instructions(&grants.grants);

        let proposal_data = ProposalData {
            name: grants.name,
            description: grants.description,
            grants: instructions,
        };

        let j = serde_json::to_string(&proposal_data).unwrap();

        fs::write("../instructions.json", j).unwrap();
    }

    if let Some(matches) = matches.subcommand_matches("withdraw") {
        let deposit = matches.get_one::<u8>("deposit").unwrap();
        let amount = matches.get_one::<u64>("amount").unwrap();

        let withdraw_instruction = withdraw_instruction(signer.pubkey(), *deposit, *amount);

        let j = serde_json::to_string(&withdraw_instruction).unwrap();

        fs::write("../withdraw.json", j).unwrap();
    }
}

pub fn grant_instructions(grants: &[Grant]) -> Vec<GrantInstruction> {
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

        let instruction_bytes = bincode::serialize(&instruction).unwrap();

        instructions.push(GrantInstruction {
            wallet: grant.wallet.clone(),
            grant_type: grant.grant_type.clone(),
            start: grant.start,
            periods: grant.periods,
            allow_clawback: grant.allow_clawback,
            amount: grant.amount,
            instruction: instruction_bytes,
        });
    }

    instructions
}

pub fn withdraw_instruction(
    wallet: Pubkey,
    deposit_entry_index: u8,
    amount: u64,
) -> WithdrawInstruction {
    let voter_stake_program = Pubkey::from_str(&env::var("VOTER_STAKE_PROGRAM").unwrap()).unwrap();

    let registrar = Pubkey::from_str(&env::var("REGISTRAR").unwrap()).unwrap();

    let mint = Pubkey::from_str(&env::var("MINT").unwrap()).unwrap();

    let (voter, _voter_bump) = Pubkey::find_program_address(
        &[registrar.as_ref(), "voter".as_bytes(), wallet.as_ref()],
        &voter_stake_program,
    );

    let (voter_weight_record, _voter_weight_record_bump) = Pubkey::find_program_address(
        &[
            registrar.as_ref(),
            "voter-weight-record".as_bytes(),
            wallet.as_ref(),
        ],
        &voter_stake_program,
    );

    let vault = spl_associated_token_account::get_associated_token_address(&voter, &mint);

    let destination_token_acc =
        spl_associated_token_account::get_associated_token_address(&wallet, &mint);

    let token_owner_record = Pubkey::from_str(&env::var("REALM_AUTH").unwrap()).unwrap();

    let accounts = voter_stake_registry::accounts::Withdraw {
        registrar,
        voter,
        voter_authority: wallet,
        token_owner_record,
        voter_weight_record,
        vault,
        destination: destination_token_acc,
        token_program: spl_token::id(),
    }
    .to_account_metas(None);

    let data = voter_stake_registry::instruction::Withdraw {
        deposit_entry_index,
        amount,
    }
    .data();

    let instruction = Instruction {
        program_id: voter_stake_program,
        data,
        accounts,
    };

    let instruction_bytes = bincode::serialize(&instruction).unwrap();

    WithdrawInstruction {
        wallet: wallet.to_string(),
        instruction: instruction_bytes,
    }
}

fn keypair_or_ledger_of(path: &Path) -> Box<dyn Signer> {
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

        Box::new(
            generate_remote_keypair(
                locator,
                derivation_path,
                &hw_wallet,
                confirm_public_key,
                "council", /* When multiple wal
                           lets are connected, used to display a hint */
            )
            .expect("Failed to contact remote wallet"),
        )
    } else {
        Box::new(read_keypair_file(path.to_str().unwrap()).unwrap())
    };
}
