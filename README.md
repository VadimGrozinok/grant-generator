# Voter-stake-registry grant generator

If you not familiar with Solana governance program or voter-stake-registry adding please take a look at [this](https://github.com/solana-labs/solana-program-library/tree/master/governance) and [this](https://github.com/blockworks-foundation/voter-stake-registry) docs.

The goal of this tool is to provide fast and convenient way to create a DAO proposal with bunch of grants. For example we have 10 investors/advisors and everyone should receive couple of different grants. It means a lot of routine work and high mistake probability. To avoid it we can fill `grants.json` file with all the data and generate all the transactions.

Next, once transactions are generated, all we need to do is create proposal with transactions from the file.

Also one important moment is that grantee should not execute proposal with his grant, council can do it instead.

So let's move on, before filling in `grants.json` we have to work with `.env` file.

## ENV accounts

`VOTER_STAKE_PROGRAM` - voter-stake-registry program DAO is using

`MINT` - token that we are going to grant

`REGISTRAR` - voter-stake-registry registrar account

`DEPOSIT_TOKEN` - DAO's token account we will send grant from

`DEPOSIT_TOKEN_AUTH` - token account authority, DAO controlled account(PDA)

`REALM_AUTH` - DAO authority

`PAYER` - wallet who will pay transaction fee, may be one of councils

`GOVERNANCE_PROGRAM` - governance program id

`GOVERNANCE` - governance address we create proposal on

`COUNCIL_MINT` - DAO council mint

## Filling in grants file

Data in `grants.json` should look like this:

``` json
{
    "name": "",
    "description": "",
    "grants": [
        {
            "wallet": "address",
            "grant_type": "Daily",
            "start": null,
            "periods": 1,
            "allow_clawback": true,
            "amount": 1000000
        }
    ]
}
```

`name` - proposal name

`description` - proposal description

`wallet` - wallet address

`grant_type` - can be one of `["Daily", "Monthly", "Cliff", "Constant"]`

`start` - moment to start could periods from, if `null` - means current timestamp

`periods` - how long to lock up, depending on `grant_type`

`allow_clawback` - when enabled, the the realm_authority is allowed to unilaterally claim locked tokens

`amount` - amount to tokens to be granted, keep in mind that all the tokens have different precision

## Just do it

There is a shell script to work with Rust CLI, it is responsible for compiling and running programs.

> Before run shell script do `chmod +x run.sh`

Script has two commands: create-proposal & execute.

First we need to create proposal and add grants there

``` bash
./run.sh -c create-proposal -w walletPath -g ../grants.json -n RPCLink

-c - command, required argument
-w - wallet, path to the key, to use ledger put smth like usb://ledger?key=0
-g - grants, file with grants
-n - node, link to Solana RPC, by default https://api.mainnet-beta.solana.com/
```

Once we run this command CLI generates two .json files, one with instructions which were executed(instructions to add transaction to the proposal) and second one transaction_to_execute.json contains transactions we need to execute(actual grant transactions).

Once proposal created it's time to execute all the proposal transactions. For this run

``` bash
./run.sh -c execute -w walletPath -n RPCLink

-c - command, required argument
-w - wallet, path to the key, to use ledger put smth like usb://ledger?key=0
-n - node, link to Solana RPC, by default https://api.mainnet-beta.solana.com/
```

As a result we will receive `transactions.json` file with content like this:

``` json
[
    {
        "wallet":"address",
        "grant_type":"Daily",
        "start":null,
        "periods":1,
        "allow_clawback":true,
        "amount":2000000,
        "tx":"base64 string"}
]
```

Where `tx` is what we actually need to create custom instruction at DAO

✌️
