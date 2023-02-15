<div align="center">
  </br>
  <p>
    <img height="50" src="https://cypher.trade/svgs/logo.svg" />
  </p>
  <p>
    <strong>cypher v3 command line utilities</strong>
  </p>
  <p>
    <a href="https://discord.gg/jr9Mu4Uz25">
      <img alt="Discord Chat" src="https://img.shields.io/discord/880917405356945449?color=blue&style=flat-square" />
    </a>
  </p>
  <h4>
    <a href="https://cypher.trade/">cypher.trade</a>
    <span> | </span>
    <a href="https://github.com/chugach-foundation/cypher-client-v3">Rust Clients</a>
    <span> | </span>
    <a href="https://github.com/chugach-foundation/cypher-client-ts-v3">TypeScript Client</a>
  </h4>
  </br>
</div>

#

`cypher-cli` provides rust code that integrates with [cypher protocol](https://cypher.trade).

This project contains contains some useful command-line utilities, such as:

- `account` - utility commands for the accounts
  - `close` - closes an account if possible, requires all sub accounts to have been zeroed out
  - `create` - creates an account
  - `create-whitelisted` - creates a whitelisted account (requires having the pubkeys of the corresponding private clearing and the whitelist accounts) 
  - `peek` - displays the sub accounts
- `sub-account` - utility commands for the sub accounts
  - `close` - closes a sub account if possible, requires all positions to have been closed
  - `create` - creates a sub account
  - `deposit` - deposits an asset
  - `peek` - displays the sub account's deposits, borrows and derivatives positions
  - `transfer` - transfers an asset between sub-accounts
  - `withdraw` - withdraws an asset
- `perps`, `futures` - utility commands to interact with perp and futures markets, commands are similar between the two
  - `book` - displays the orderbook, `symbol` needs to be specified
  - `cancel` - cancels an order by ID
  - `closes` - closes an existing position entirely, if liquidity on the orderbook so allows
  - `market` - submits a market order with the given paremeters, only `size`, `side` and `symbol` need to be specified
  - `orders` - displays open orders by market, along with quantities locked in orders accounts
  - `place` - places an order with the given parameters, `size`, `side`, `symbol`, `order-type` need to be specified
  - `settle` - settles unlocked funds in the orders accounts
- `spot` - utilitity commands to interact with spot markets
  - `book` - displays the orderbook, `symbol` needs to be specified
  - `cancel` - cancels an order by ID
  - `market` - submits a market order with the given paremeters, only `size`, `side` and `symbol` need to be specified
  - `orders` - displays open orders by market, along with quantities locked in orders accounts
  - `place` - places an order with the given parameters, `size`, `side`, `symbol`, `order-type` need to be specified
  - `settle` - settles unlocked funds in the orders accounts
- `faucet` (only available on devnet)
  - `list`
  - `request`

There are also some more complex actions such as:

- `market-maker` - runs a marketmaker with the given config, available for cypher's derivative markets and openbook spot markets using cypher margin accounts
  - check possible configs at `/cfg/market/maker`
- `liquidator` - runs a liquidator with the given config, this is a simple liquidator program with additional functionality being built
  - check possible configs at `/cfg/liquidator`

## Building

Building the cli is as easy as:

### For devnet usage

```sh
cargo build --package cypher-cli --release
```

### For mainnet usage

```sh
cargo build --package cypher-cli --release --no-default-features --features mainnet-beta
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