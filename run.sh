#!/bin/sh

export $(xargs < .env)

while getopts c:w:g:n:d:a: flag
do
    case "${flag}" in
        c) command=${OPTARG};;
        w) wallet=${OPTARG};;
        g) grants=${OPTARG};;
        n) node=${OPTARG};;
        d) deposit=${OPTARG};;
        a) amount=${OPTARG};;

    esac
done

cd instruction-generator
cargo build

cd ../proposal-creator
cargo build

cd ../

if [ $command == "create-proposal" ]
then
    cd instruction-generator && cargo r -- -w $wallet grant -g $grants && cd ../proposal-creator && cargo r -- -w $wallet -n $node create-proposal -i ../instructions.json && cd ../
elif [ $command == "execute" ]
then
    cd proposal-creator && cargo r -- -w $wallet -n $node execute -t ../transaction_to_execute.json && cd ../
elif [ $command == "withdraw" ]
then
    cd instruction-generator && cargo r -- -w $wallet withdraw -d $deposit -a $amount && cd ../proposal-creator && cargo r -- -w $wallet -n $node execute-withdraw -i ../withdraw.json && cd ../
else
    echo "Unknow command"
fi
