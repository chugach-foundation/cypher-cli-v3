use anchor_spl::dex::serum_dex::state::MarketState;
use cypher_client::{
    constants::QUOTE_TOKEN_DECIMALS,
    utils::{
        convert_coin_to_decimals_fixed, convert_price_to_decimals_fixed, fixed_to_ui,
        native_to_ui_fixed,
    },
    Market,
};
use cypher_utils::contexts::{AgnosticOrderBookContext, SerumOrderBookContext};

pub fn display_orderbook(book_ctx: &AgnosticOrderBookContext, market: &dyn Market) {
    let bids = &book_ctx.state.bids;
    let asks = &book_ctx.state.asks;

    println!("Bids: {} - Asks: {}", bids.len(), asks.len());

    let longest = if bids.len() > asks.len() {
        bids.len()
    } else {
        asks.len()
    };

    println!("\n| {:^43} | {:^43} |", "Bids", "Asks",);
    println!(
        "| {:^20} | {:^20} | {:^20} | {:^20} |",
        "Price", "Size", "Size", "Price",
    );

    for i in 0..longest {
        let bid = bids.get(i);
        let ask = asks.get(i);

        let output = if bid.is_some() && ask.is_some() {
            let bid = bid.unwrap();
            let ask = ask.unwrap();
            format!(
                "| {:^20.4} | {:^20.4} | {:^20.4} | {:^20.4} |",
                fixed_to_ui(
                    convert_price_to_decimals_fixed(
                        bid.price,
                        market.base_multiplier(),
                        10u64.pow(market.decimals() as u32),
                        market.quote_multiplier()
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
                // at this point the order size already comes in native units
                // so we do not need to convert from lots to native
                native_to_ui_fixed(bid.base_quantity, market.decimals()),
                native_to_ui_fixed(ask.base_quantity, market.decimals()),
                fixed_to_ui(
                    convert_price_to_decimals_fixed(
                        ask.price,
                        market.base_multiplier(),
                        10u64.pow(market.decimals() as u32),
                        market.quote_multiplier()
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
            )
        } else if bid.is_none() {
            let ask = ask.unwrap();
            format!(
                "| {:^20.4} | {:^20.4} | {:^20.4} | {:^20.4} |",
                0,
                0,
                // at this point the order size already comes in native units
                // so we do not need to convert from lots to native
                native_to_ui_fixed(ask.base_quantity, market.decimals()),
                fixed_to_ui(
                    convert_price_to_decimals_fixed(
                        ask.price,
                        market.base_multiplier(),
                        10u64.pow(market.decimals() as u32),
                        market.quote_multiplier()
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
            )
        } else {
            let bid = bid.unwrap();
            format!(
                "| {:^20.4} | {:^20.4} | {:^20.4} | {:^20.4} |",
                fixed_to_ui(
                    convert_price_to_decimals_fixed(
                        bid.price,
                        market.base_multiplier(),
                        10u64.pow(market.decimals() as u32),
                        market.quote_multiplier()
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
                // at this point the order size already comes in native units
                // so we do not need to convert from lots to native
                native_to_ui_fixed(bid.base_quantity, market.decimals()),
                0,
                0
            )
        };

        println!("{}", output);
    }
}

pub fn display_spot_orderbook(
    book_ctx: &SerumOrderBookContext,
    market: &MarketState,
    decimals: u8,
) {
    let bids = &book_ctx.state.bids;
    let asks = &book_ctx.state.asks;

    println!("Bids: {} - Asks: {}", bids.len(), asks.len());

    let longest = if bids.len() > asks.len() {
        bids.len()
    } else {
        asks.len()
    };

    println!("\n| {:^43} | {:^43} |", "Bids", "Asks",);
    println!(
        "| {:^20} | {:^20} | {:^20} | {:^20} |",
        "Price", "Size", "Size", "Price",
    );

    for i in 0..longest {
        let bid = bids.get(i);
        let ask = asks.get(i);

        let output = if bid.is_some() && ask.is_some() {
            let bid = bid.unwrap();
            let ask = ask.unwrap();
            format!(
                "| {:^20.4} | {:^20.4} | {:^20.4} | {:^20.4} |",
                fixed_to_ui(
                    convert_price_to_decimals_fixed(
                        bid.price,
                        market.coin_lot_size,
                        10u64.pow(decimals as u32),
                        market.pc_lot_size
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
                // at this point the order size already comes in native units
                // so we do not need to convert from lots to native
                native_to_ui_fixed(bid.base_quantity, decimals),
                native_to_ui_fixed(ask.base_quantity, decimals),
                fixed_to_ui(
                    convert_price_to_decimals_fixed(
                        ask.price,
                        market.coin_lot_size,
                        10u64.pow(decimals as u32),
                        market.pc_lot_size
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
            )
        } else if bid.is_none() {
            let ask = ask.unwrap();
            format!(
                "| {:^20.4} | {:^20.4} | {:^20.4} | {:^20.4} |",
                0,
                0,
                // at this point the order size already comes in native units
                // so we do not need to convert from lots to native
                native_to_ui_fixed(ask.base_quantity, decimals),
                fixed_to_ui(
                    convert_price_to_decimals_fixed(
                        ask.price,
                        market.coin_lot_size,
                        10u64.pow(decimals as u32),
                        market.pc_lot_size
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
            )
        } else {
            let bid = bid.unwrap();
            format!(
                "| {:^20.4} | {:^20.4} | {:^20.4} | {:^20.4} |",
                fixed_to_ui(
                    convert_price_to_decimals_fixed(
                        bid.price,
                        market.coin_lot_size,
                        10u64.pow(decimals as u32),
                        market.pc_lot_size
                    ),
                    QUOTE_TOKEN_DECIMALS
                ),
                // at this point the order size already comes in native units
                // so we do not need to convert from lots to native
                native_to_ui_fixed(bid.base_quantity, decimals),
                0,
                0
            )
        };

        println!("{}", output);
    }
}
