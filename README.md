# cypher cli

`cypher-cli` provides rust code to integrate with [cypher protocol](https://cypher.trade).

This project contains contains some useful command-line utilities, such as:

- `account` - utility commands for the accounts
  - `create` - creates an account
  - `create-whitelisted` - creates a whitelisted account (requires having the pubkeys of the corresponding private clearing and the whitelist accounts)
  - `close` - closes an account if possible, requires all sub accounts to have been zeroed out
  - `peek` - displays the sub accounts
- `sub-account` - utility commands for the sub accounts
  - `create` - creates a sub account
  - `peek` - displays the sub account's deposits, borrows and derivatives positions
  - `deposit` - deposits an asset
  - `withdraw` - withdraws an asset
  - `transfer` - transfers an asset between sub-accounts
- `perps`, `futures` - utility commands to interact with perp and futures markets, commands are similar between the two
  - `place` - places an order with the given parameters, `size`, `side`, `symbol`, `order-type` can be specified
  - `market` - submits a market order with the given paremeters, only `size`, `side` and `symbol` can be specified
  - `closes` - closes an existing position entirely, if liquidity on the orderbook so allows
  - `orders` - displays open orders by market, along with quantities locked in orders accounts
  - `cancel` - cancels an order by ID
  - `settle` - settles unlocked funds in the orders accounts
- `spot` - utilitity commands to interact with spot markets
  - `place` - places an order with the given parameters, `size`, `side`, `symbol`, `order-type` can be specified
  - `market` - submits a market order with the given paremeters, only `size`, `side` and `symbol` can be specified
  - `orders` - displays open orders by market, along with quantities locked in orders accounts
  - `cancel` - cancels an order by ID
  - `settle` - settles unlocked funds in the orders accounts
- `faucet` (only available on devnet)
  - `list`
  - `request`

There are also some more complex actions such as:

- `marketmaker` - runs a marketmaker with the given config, available for cypher's derivative markets and openbook spot markets using cypher margin accounts
- `liquidator` - runs a liquidator with the given config (wip)


## Building

Building the cli is as easy as:

### For devnet usage

```sh
cargo build --package cypher-cli --release
```

### For mainnet usage

```sh
cargo build --package cypher-cli --release --features mainnet-beta
```

### ⚠️⚠️ Compilation issues for M1 chips ⚠️⚠️

In order to prevent issues when compiling due to the `agnostic-orderbook`.

In the root directory of the repo:

`rustup override set 1.59.0-x86_64-apple-darwin`

## Usage

Running the cli:

```sh
./target/release/cypher-cli -u <RPC_URL> -k <KEYPAIR_FILEPATH>
```

In order to run the market maker and/or liquidator, it is necessary to provide a Streaming RPC endpoint:

```sh
./target/release/cypher-cli -u <RPC_URL> -p <STREAMING_RPC_URL> -k <KEYPAIR_FILEPATH> market-maker run -c -k <MAKER_CONFIG_FILEPATH>
```