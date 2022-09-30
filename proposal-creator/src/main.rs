use borsh::BorshDeserialize;
use clap::{arg, command, value_parser, ArgAction, Command};
use dotenv::dotenv;
use serde::{Deserialize, Serialize};
use solana_client::rpc_client::RpcClient;
use solana_remote_wallet::locator::Locator;
use solana_remote_wallet::remote_keypair::generate_remote_keypair;
use solana_remote_wallet::remote_wallet::maybe_wallet_manager;
use solana_sdk::derivation_path::DerivationPath;
use solana_sdk::{
    self, instruction::Instruction, pubkey::Pubkey, signature::Signer,
    signer::keypair::read_keypair_file, transaction::Transaction,
};
use spl_governance::{
    instruction::{
        add_signatory, create_proposal as create_proposal_instruction, insert_transaction,
        sign_off_proposal,
    },
    state::{
        governance::GovernanceV2,
        proposal::{get_proposal_address, VoteType},
        proposal_transaction::InstructionData,
    },
};
use uriparse::URIReference;

use std::{env, fs, path::PathBuf, str::FromStr};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum GrantType {
    None,
    Daily,
    Monthly,
    Cliff,
    Constant,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProposalData {
    pub name: String,
    pub description: String,
    pub grants: Vec<GrantInstruction>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GrantInstruction {
    pub wallet: String,
    pub grant_type: GrantType,
    pub start: Option<u64>,
    pub periods: u32,
    pub allow_clawback: bool,
    pub amount: u64,
    pub instruction: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransactionsToExecute {
    pub governance: String,
    pub proposal: String,
    pub transactions: Vec<ProposalTransaction>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProposalTransaction {
    pub address: String,
    pub transaction_program_id: String,
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
        .arg(
            arg!(
                -n --node <URL> "Solana RPC node URL"
            )
            .default_value("https://api.mainnet-beta.solana.com/"),
        )
        .subcommand(
            Command::new("create-proposal")
                .about("creates new DAO proposal and attaches bunch of Grant transactions to it")
                .arg(
                    arg!(-i --instructions "lists of grants to be created")
                        .required(true)
                        .action(ArgAction::Set),
                ),
        )
        .get_matches();

    let wallet_path = matches.get_one::<PathBuf>("wallet").unwrap();

    let signer = keypair_or_ledger_of(wallet_path);

    let node_url = matches.get_one::<String>("node").unwrap();

    let client = RpcClient::new(node_url);

    if let Some(matches) = matches.subcommand_matches("create-proposal") {
        let instructions_file = matches.get_one::<String>("instructions").unwrap();

        let grants_data = fs::read_to_string(instructions_file).unwrap();

        let grants: ProposalData = serde_json::from_str(&grants_data).unwrap();

        create_proposal(&client, signer, &grants);
    }
}

fn create_proposal(client: &RpcClient, signer: Box<dyn Signer>, data: &ProposalData) {
    let governance_program = Pubkey::from_str(&env::var("GOVERNANCE_PROGRAM").unwrap()).unwrap();
    let governance_key = Pubkey::from_str(&env::var("GOVERNANCE").unwrap()).unwrap();
    let council_mint = Pubkey::from_str(&env::var("COUNCIL_MINT").unwrap()).unwrap();

    let governance_bytes = client.get_account_data(&governance_key).unwrap();
    let governance_data = GovernanceV2::deserialize(&mut governance_bytes.as_ref()).unwrap();

    // proposal_owner_record - Account PDA seeds: ['governance', realm, token_mint, token_owner ]
    let proposal_owner_record = Pubkey::find_program_address(
        &[
            "governance".as_ref(),
            governance_data.realm.as_ref(),
            council_mint.as_ref(),
            signer.try_pubkey().unwrap().as_ref(),
        ],
        &governance_program,
    )
    .0;

    let proposal_instruction = create_proposal_instruction(
        &governance_program,
        &governance_key,
        &proposal_owner_record,
        &signer.try_pubkey().unwrap(),
        &signer.try_pubkey().unwrap(),
        None,
        &governance_data.realm,
        data.name.clone(),
        data.description.clone(),
        &council_mint,
        VoteType::SingleChoice,
        vec!["Approve".to_string()],
        true,
        governance_data.proposals_count,
    );

    let proposal_address = get_proposal_address(
        &governance_program,
        &governance_key,
        &council_mint,
        &governance_data.proposals_count.to_le_bytes(),
    );

    let add_signatory_instruction = add_signatory(
        &governance_program,
        &proposal_address,
        &proposal_owner_record,
        &signer.try_pubkey().unwrap(),
        &signer.try_pubkey().unwrap(),
        &signer.try_pubkey().unwrap(),
    );

    let blockhash = client.get_latest_blockhash().unwrap();

    let tx = Transaction::new_signed_with_payer(
        &[proposal_instruction, add_signatory_instruction],
        Some(&signer.try_pubkey().unwrap()),
        &[&*signer],
        blockhash,
    );
    let signature = client.send_and_confirm_transaction(&tx).unwrap();

    println!("Proposal was created: {:?}", signature);

    let mut proposal_tx_index: u16 = 0;

    // TODO: add error flag

    for grant in data.grants.iter() {
        let instruction: Instruction = bincode::deserialize(&grant.instruction).unwrap();
        let instruction_data = InstructionData::from(instruction);

        let insert_instruction = insert_transaction(
            &governance_program,
            &governance_key,
            &proposal_address,
            &proposal_owner_record,
            &signer.try_pubkey().unwrap(),
            &signer.try_pubkey().unwrap(),
            0,
            proposal_tx_index,
            0,
            vec![instruction_data],
        );

        // TODO: add error processing and retry
        let blockhash = client.get_latest_blockhash().unwrap();

        let tx = Transaction::new_signed_with_payer(
            &[insert_instruction],
            Some(&signer.try_pubkey().unwrap()),
            &[&*signer],
            blockhash,
        );
        let signature = client.send_and_confirm_transaction(&tx).unwrap();

        println!("New transaction was added to the proposal: {:?}", signature);

        proposal_tx_index += 1;
    }

    let sign_off_proposal = sign_off_proposal(
        &governance_program,
        &governance_data.realm,
        &governance_key,
        &proposal_address,
        &signer.try_pubkey().unwrap(),
        None,
    );

    let blockhash = client.get_latest_blockhash().unwrap();

    let tx = Transaction::new_signed_with_payer(
        &[sign_off_proposal],
        Some(&signer.try_pubkey().unwrap()),
        &[&*signer],
        blockhash,
    );
    let signature = client.send_and_confirm_transaction(&tx).unwrap();

    println!("Proposal was signed off: {:?}", signature);
}

fn keypair_or_ledger_of(path: &PathBuf) -> Box<dyn Signer> {
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
