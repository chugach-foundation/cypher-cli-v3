use chrono::format;
use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_client::cache_account;
use cypher_utils::{
    accounts_cache::AccountsCache, contexts::CypherContext, logging::init_logger,
    services::StreamingAccountInfoService,
};
use fixed::types::I80F48;
use log::{info, warn};
use std::{error, sync::Arc};
use tokio::sync::broadcast::channel;

use crate::{
    cli::CliError,
    common::{
        context::{
            builder::ContextBuilder, manager::ContextManager, GlobalContext, OperationContext,
        },
        orders::OrderManager,
        runner::{ExecutionCondition, Runner, RunnerOptions},
        strategy::Strategy,
    },
    context::builders::global::GlobalContextBuilder,
    market_maker::{
        config::{
            get_context_manager_from_config, get_hedger_context_builder, get_hedger_from_config,
            get_maker_context_builder, get_maker_from_config, get_order_manager_from_config,
            load_config, Config, ConfigError,
        },
        error::Error,
        strategies::{hedging::HedgingStrategy, making::MakingStrategy},
    },
    utils::accounts::{get_or_create_account, get_or_create_sub_account},
};

use super::{command::CliCommand, CliConfig, CliResult};

pub trait MarketMakerSubCommands {
    fn market_maker_subcommands(self) -> Self;
}

impl MarketMakerSubCommands for App<'_, '_> {
    fn market_maker_subcommands(self) -> Self {
        self.subcommand(
            SubCommand::with_name("market-maker")
                .about("Runs a market making bot.")
                .subcommand(
                    SubCommand::with_name("run").about("Runs the market maker.")
                        .arg(
                        Arg::with_name("config")
                            .short("c")
                            .long("config")
                            .value_name("FILE")
                            .takes_value(true)
                            .help("Path to a config file. This config should follow the format displayed in `/cfg/market-maker/default.json`."),
                    )
                )
        )
    }
}

pub fn parse_market_maker_command(
    matches: &ArgMatches,
) -> Result<CliCommand, Box<dyn error::Error>> {
    match matches.subcommand() {
        ("run", Some(matches)) => {
            let path = match matches.value_of("config") {
                Some(s) => s,
                None => {
                    return Err(Box::new(CliError::BadParameters(
                        "Path to config path not provided.".to_string(),
                    )));
                }
            };
            Ok(CliCommand::MarketMaker {
                path: path.to_string(),
            })
        }
        ("", None) => {
            eprintln!("{}", matches.usage());
            Err(Box::new(CliError::CommandNotRecognized(
                "No market maker subcommand given.".to_string(),
            )))
        }
        _ => unreachable!(),
    }
}

pub async fn process_market_maker_command(
    config: &CliConfig,
    config_path: &str,
) -> Result<CliResult, Box<dyn error::Error>> {
    _ = init_logger();
    let rpc_client = config.rpc_client.as_ref().unwrap();
    let pubsub_client = config.pubsub_client.as_ref().unwrap();
    let keypair = config.keypair.as_ref().unwrap();

    info!("Setting up components from config..");

    let shutdown_sender = Arc::new(channel::<bool>(1).0);

    let mm_config = match load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            warn!("There was an error loading config: {}", e.to_string());
            return Err(Box::new(CliError::BadParameters(
                "Failed to load market maker config.".to_string(),
            )));
        }
    };

    let cypher_ctx = match CypherContext::load(rpc_client).await {
        Ok(ctx) => ctx,
        Err(e) => {
            warn!(
                "There was an error loading the cypher context: {}",
                e.to_string()
            );
            return Err(Box::new(CliError::ContextError(e)));
        }
    };

    // todo: change account number to allow passing in arg or config
    let (account_state, master_account) =
        match get_or_create_account(rpc_client, &keypair, mm_config.account_number).await {
            Ok(a) => a,
            Err(e) => {
                warn!(
                    "There was an error getting or creating Cypher account: {}",
                    e.to_string()
                );
                return Err(Box::new(CliError::ClientError(e)));
            }
        };

    // todo: change sub account number to allow passing in arg or config
    let (sub_acccount_state, sub_account) = match get_or_create_sub_account(
        rpc_client,
        &keypair,
        &master_account,
        mm_config.sub_account_number,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            warn!(
                "There was an error getting or creating Cypher sub account: {}",
                e.to_string()
            );
            return Err(Box::new(CliError::ClientError(e)));
        }
    };

    let accounts_cache = Arc::new(AccountsCache::new());
    let streaming_account_service = Arc::new(StreamingAccountInfoService::new(
        accounts_cache.clone(),
        pubsub_client.clone(),
        rpc_client.clone(),
        shutdown_sender.subscribe(),
        &vec![],
    ));
    streaming_account_service
        .add_subscriptions(&vec![cache_account::id(), master_account, sub_account])
        .await;

    let global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext> + Send> = Arc::new(
        GlobalContextBuilder::new(accounts_cache.clone(), master_account, sub_account),
    );
    let global_context_builder_clone = global_context_builder.clone();
    let global_context_handler = tokio::spawn(async move {
        match global_context_builder_clone.start().await {
            Ok(_) => (),
            Err(e) => {
                warn!("There was an error running the Global Context Builder.")
            }
        }
    });

    let maker = match get_maker_from_config(&cypher_ctx, &mm_config).await {
        Ok(m) => m,
        Err(e) => {
            warn!("There was an error preparing Maker: {}", e.to_string());
            return Err(Box::new(CliError::MarketMaker(e)));
        }
    };
    let hedger = match get_hedger_from_config(&cypher_ctx, &mm_config) {
        Ok(m) => m,
        Err(e) => {
            warn!("There was an error preparing Hedger: {}", e.to_string());
            return Err(Box::new(CliError::MarketMaker(e)));
        }
    };

    let hedging_strategy: Arc<dyn Strategy> = Arc::new(HedgingStrategy::new(hedger.clone()));
    let making_strategy: Arc<dyn Strategy> = Arc::new(MakingStrategy::new(maker.clone()));

    let order_manager: Arc<dyn OrderManager> =
        match get_order_manager_from_config(&cypher_ctx, &mm_config) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "There was an error preparing Order Manager: {}",
                    e.to_string()
                );
                return Err(Box::new(CliError::MarketMaker(e)));
            }
        };

    let maker_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send> =
        match get_maker_context_builder(
            &streaming_account_service,
            rpc_client,
            &cypher_ctx,
            &mm_config,
            keypair,
            accounts_cache.clone(),
            &master_account,
            &sub_account,
        )
        .await
        {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "There was an error preparing Maker Context Builder: {}",
                    e.to_string()
                );
                return Err(Box::new(CliError::MarketMaker(e)));
            }
        };
    let maker_ctx_builder_handler_clone = maker_context_builder.clone();
    let maker_ctx_builder_handler = tokio::spawn(async move {
        match maker_ctx_builder_handler_clone.start().await {
            Ok(_) => (),
            Err(e) => {
                warn!(
                    "There was an error running Maker Context Builder: {:?}",
                    e.to_string()
                );
            }
        }
    });

    let hedger_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send> =
        match get_hedger_context_builder(
            &streaming_account_service,
            rpc_client,
            &cypher_ctx,
            &mm_config,
            keypair,
            accounts_cache.clone(),
            &master_account,
            &sub_account,
        )
        .await
        {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "There was an error preparing Hedger Context Builder: {}",
                    e.to_string()
                );
                return Err(Box::new(CliError::MarketMaker(e)));
            }
        };
    let hedger_ctx_builder_handler_clone = hedger_context_builder.clone();
    let hedger_ctx_builder_handler = tokio::spawn(async move {
        match hedger_ctx_builder_handler_clone.start().await {
            Ok(_) => (),
            Err(e) => {
                warn!(
                    "There was an error running Heger Context Builder: {:?}",
                    e.to_string()
                );
            }
        }
    });

    let maker_context_manager: Arc<dyn ContextManager> = match get_context_manager_from_config(
        &cypher_ctx,
        &mm_config,
        global_context_builder.clone(),
        maker_context_builder.clone(),
    ) {
        Ok(m) => m,
        Err(e) => {
            warn!(
                "There was an error preparing Maker Context Manager: {}",
                e.to_string()
            );
            return Err(Box::new(CliError::MarketMaker(e)));
        }
    };
    let maker_context_manager_clone = maker_context_manager.clone();
    let maker_context_manager_handle = tokio::spawn(async move {
        match maker_context_manager_clone.start().await {
            Ok(_) => (),
            Err(e) => {
                warn!(
                    "There was an error running Maker Context Manager: {:?}",
                    e.to_string()
                );
            }
        }
    });

    let hedger_context_manager: Arc<dyn ContextManager> = match get_context_manager_from_config(
        &cypher_ctx,
        &mm_config,
        global_context_builder.clone(),
        hedger_context_builder.clone(),
    ) {
        Ok(m) => m,
        Err(e) => {
            warn!(
                "There was an error preparing Hedger Context Manager: {}",
                e.to_string()
            );
            return Err(Box::new(CliError::MarketMaker(e)));
        }
    };
    let hedger_context_manager_clone = hedger_context_manager.clone();
    let hedger_context_manager_handle = tokio::spawn(async move {
        match hedger_context_manager_clone.start().await {
            Ok(_) => (),
            Err(e) => {
                warn!(
                    "There was an error running Hedger Context Manager: {:?}",
                    e.to_string()
                );
            }
        }
    });

    info!("Let's dance! 🔥💃");

    let maker_runner_opts = RunnerOptions {
        name: mm_config.maker_config.symbol.to_string(),
        shutdown: shutdown_sender.clone(),
        execution_condition: ExecutionCondition::EventBased,
        strategy: making_strategy.clone(),
        context_manager: maker_context_manager.clone(),
    };

    let maker_runner = Arc::new(Runner::new(maker_runner_opts));
    let maker_runner_clone = maker_runner.clone();
    let maker_runner_handle = tokio::spawn(async move {
        match maker_runner_clone.run().await {
            Ok(()) => (),
            Err(e) => {
                warn!(
                    "There was an error running Making Strategy Runner: {:?}",
                    e.to_string()
                );
            }
        }
    });

    let hedger_runner_opts = RunnerOptions {
        name: mm_config.maker_config.symbol.to_string(),
        shutdown: shutdown_sender.clone(),
        execution_condition: ExecutionCondition::EventBased,
        strategy: hedging_strategy.clone(),
        context_manager: maker_context_manager.clone(),
    };

    let hedger_runner = Arc::new(Runner::new(hedger_runner_opts));
    let hedger_runner_clone = hedger_runner.clone();
    let hedger_runner_handle = tokio::spawn(async move {
        match hedger_runner_clone.run().await {
            Ok(()) => (),
            Err(e) => {
                warn!(
                    "There was an error running Hedging Strategy Runner: {:?}",
                    e.to_string()
                );
            }
        }
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            match shutdown_sender.send(true) {
                Ok(_) => {
                    info!("Sucessfully sent shutdown signal. Waiting for tasks to complete...")
                },
                Err(e) => {
                    warn!("Failed to send shutdown error: {}", e.to_string());
                }
            };
        },
    }

    tokio::join!(
        global_context_handler,
        maker_ctx_builder_handler,
        hedger_ctx_builder_handler,
        maker_context_manager_handle,
        hedger_context_manager_handle,
        maker_runner_handle,
        hedger_runner_handle
    );

    Ok(CliResult {})
}
