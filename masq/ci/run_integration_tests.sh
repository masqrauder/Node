#!/bin/bash -xev
# Copyright (c) 2019, MASQ (https://masq.ai). All rights reserved.
CI_DIR="$( cd "$( dirname "$0" )" && pwd )"
KIND="$1" # should be either 'sudo' or 'user'
TOOLCHAIN_HOME="$2"

export PATH="$PATH:$HOME/.cargo/bin"
source "$CI_DIR"/../../ci/environment.sh "$TOOLCHAIN_HOME"

export RUST_BACKTRACE=full
export RUSTFLAGS="-D warnings -Anon-snake-case"
umask 000

pushd "$CI_DIR/.."
cargo test --release -- --nocapture "_${KIND}_integration"
BUILD_RESULT=$?
if [[ "$(id -u)" == "0" ]]; then
    chmod -R 777 "$CI_DIR/../target"
fi
exit "$BUILD_RESULT"
popd
