#!/bin/sh

export $(xargs < .env)

while getopts w:g:n: flag
do
    case "${flag}" in
        w) wallet=${OPTARG};;
        g) grants=${OPTARG};;
        n) node=${OPTARG};;

    esac
done

cd instruction-generator
cargo build

cd ../proposal-creator
cargo build

cd ../instruction-generator

cargo r -- -w $wallet grant -g $grants

cd ../proposal-creator

cargo r -- -w $wallet -n $node create-proposal -i ../instructions.json
