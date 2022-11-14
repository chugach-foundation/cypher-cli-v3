# cypher toolkit

`cypher-toolkit` provides rust code to integrate with [cypher protocol](https://cypher.trade).

This project contains contains some useful command-line utilities, such as:

- `account`
  - `create`
  - `peek`
- `sub-account`
  - `create`
  - `peek`
- `deposit`
- `withdraw`
- `place-order`
  - `limit`
  - `market`
  - `post-only`
- `cancel-order`
- `prune-orders`
- `faucet` (only available on devnet)
  - `list`
  - `request`

There are also some more complex actions such as:

- [marketmaker]() for Serum Spot markets and cypher's derivatives markets
- [liquidator]() for cypher's platform


## Building

Building the cli is as easy as:

```sh
cargo build --package cypher-cli
```

### ⚠️⚠️ Compilation issues for M1 chips ⚠️⚠️

In order to prevent issues when compiling due to the `agnostic-orderbook`.

In the root directory of the repo:

`rustup override set 1.59.0-x86_64-apple-darwin`

## Usage

Running the crank:

```sh
./target/debug/cypher-cli -u <RPC_URL> -k <KEYPAIR_FILEPATH>
```