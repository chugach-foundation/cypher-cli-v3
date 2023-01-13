use chrono::format;
use clap::{App, Arg, ArgMatches, SubCommand};
use cypher_client::cache_account;
use cypher_utils::{
    accounts_cache::AccountsCache, contexts::CypherContext, logging::init_logger,
    services::StreamingAccountInfoService,
};
use fixed::types::I80F48;
use log::{info, warn};
use solana_sdk::signature::Keypair;
use std::{error, sync::Arc};
use tokio::sync::broadcast::channel;

use crate::{
    cli::CliError,
    common::{
        context::{
            builder::ContextBuilder, manager::ContextManager, ExecutionContext, GlobalContext,
            OperationContext,
        },
        oracle::{OracleInfo, OracleProvider},
        strategy::Strategy,
    },
    context::builders::global::GlobalContextBuilder,
    market_maker::{
        config::{
            get_context_builder, get_context_info, get_context_manager_from_config,
            get_hedger_from_config, get_maker_from_config, get_oracle_provider, get_user_info,
            load_config, Config, ConfigError,
        },
        error::Error,
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

    let user_info = match get_user_info(rpc_client.clone(), &mm_config, keypair).await {
        Ok(ui) => ui,
        Err(e) => {
            warn!("There was an error getting user info: {}", e.to_string());
            return Err(Box::new(CliError::MarketMaker(e)));
        }
    };

    let maker_symbol = mm_config.maker_config.symbol.as_str();
    let maker_context_info =
        match get_context_info(rpc_client.clone(), &cypher_ctx, &user_info, maker_symbol).await {
            Ok(mci) => mci,
            Err(e) => {
                warn!(
                    "There was an error getting maker context info: {}",
                    e.to_string()
                );
                return Err(Box::new(CliError::MarketMaker(e)));
            }
        };

    let hedger_symbol = mm_config.hedger_config.symbol.as_str();
    let hedger_context_info =
        match get_context_info(rpc_client.clone(), &cypher_ctx, &user_info, hedger_symbol).await {
            Ok(hci) => hci,
            Err(e) => {
                warn!(
                    "There was an error getting hedger context info: {}",
                    e.to_string()
                );
                return Err(Box::new(CliError::MarketMaker(e)));
            }
        };

    let accounts_cache = Arc::new(AccountsCache::new());

    let global_context_builder: Arc<dyn ContextBuilder<Output = GlobalContext> + Send> =
        Arc::new(GlobalContextBuilder::new(
            accounts_cache.clone(),
            shutdown_sender.clone(),
            user_info.master_account.clone(),
            user_info.sub_account.clone(),
        ));

    let maker_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send> =
        match get_context_builder(
            accounts_cache.clone(),
            shutdown_sender.clone(),
            &maker_context_info,
            &user_info,
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
    let hedger_context_builder: Arc<dyn ContextBuilder<Output = OperationContext> + Send> =
        match get_context_builder(
            accounts_cache.clone(),
            shutdown_sender.clone(),
            &hedger_context_info,
            &user_info,
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

    let maker_oracle_provider: Arc<dyn OracleProvider<Input = GlobalContext> + Send> =
        match get_oracle_provider(
            &maker_context_info,
            shutdown_sender.clone(),
            global_context_builder.clone(),
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                warn!(
                    "There was an error preparing Maker Oracle Provider: {}",
                    e.to_string()
                );
                return Err(Box::new(CliError::MarketMaker(e)));
            }
        };

    let hedger_oracle_provider: Arc<dyn OracleProvider<Input = GlobalContext> + Send> =
        match get_oracle_provider(
            &hedger_context_info,
            shutdown_sender.clone(),
            global_context_builder.clone(),
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                warn!(
                    "There was an error preparing Hedger Oracle Provider: {}",
                    e.to_string()
                );
                return Err(Box::new(CliError::MarketMaker(e)));
            }
        };

    let maker_context_manager: Arc<
        dyn ContextManager<
            Output = ExecutionContext,
            GlobalContextInput = GlobalContext,
            OperationContextInput = OperationContext,
            OracleInfoInput = OracleInfo,
        >,
    > = match get_context_manager_from_config(
        &cypher_ctx,
        &mm_config,
        shutdown_sender.clone(),
        global_context_builder.clone(),
        maker_context_builder.clone(),
        maker_oracle_provider.clone(),
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
    let hedger_context_manager: Arc<
        dyn ContextManager<
            Output = ExecutionContext,
            GlobalContextInput = GlobalContext,
            OperationContextInput = OperationContext,
            OracleInfoInput = OracleInfo,
        >,
    > = match get_context_manager_from_config(
        &cypher_ctx,
        &mm_config,
        shutdown_sender.clone(),
        global_context_builder.clone(),
        hedger_context_builder.clone(),
        hedger_oracle_provider.clone(),
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

    let maker = match get_maker_from_config(
        rpc_client,
        shutdown_sender.clone(),
        maker_context_manager.sender(),
        &cypher_ctx,
        &maker_context_info,
        &mm_config,
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            warn!("There was an error preparing Maker: {}", e.to_string());
            return Err(Box::new(CliError::MarketMaker(e)));
        }
    };
    let hedger = match get_hedger_from_config(&hedger_context_info) {
        Ok(m) => m,
        Err(e) => {
            warn!("There was an error preparing Hedger: {}", e.to_string());
            return Err(Box::new(CliError::MarketMaker(e)));
        }
    };

    // at this point we have prepared everything we need
    // all that is left is spawning tasks for the components that should run concurrently in order to propagate data
    info!("🔥💃 Let's dance! 💃🔥");

    // clone and spawn task for the maker oracle provider
    let maker_oracle_provider_clone = maker_oracle_provider.clone();
    let maker_oracle_provider_handle = tokio::spawn(async move {
        match maker_oracle_provider_clone.start().await {
            Ok(_) => (),
            Err(e) => {
                warn!(
                    "There was an error running Maker Oracle Provider: {:?}",
                    e.to_string()
                );
            }
        }
    });

    // clone and spawn task for the hedger oracle provider
    let hedger_oracle_provider_clone = hedger_oracle_provider.clone();
    let hedger_oracle_provider_handle = tokio::spawn(async move {
        match hedger_oracle_provider_clone.start().await {
            Ok(_) => (),
            Err(e) => {
                warn!(
                    "There was an error running Hedger Oracle Provider: {:?}",
                    e.to_string()
                );
            }
        }
    });

    let global_context_builder_clone = global_context_builder.clone();
    let global_context_handler = tokio::spawn(async move {
        match global_context_builder_clone.start().await {
            Ok(_) => (),
            Err(e) => {
                warn!("There was an error running the Global Context Builder.")
            }
        }
    });

    // clone and spawn task for the maker context builder
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

    // clone and spawn task for the hedger context builder
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

    // clone and spawn task for the maker context manager
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

    // clone and spawn task for the hedger context manager
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

    let maker_clone = maker.clone();
    let maker_handle = tokio::spawn(async move {
        match maker_clone.start().await {
            Ok(_) => (),
            Err(e) => {
                warn!("There was an error running Maker: {:?}", e.to_string());
            }
        }
    });

    // let hedger_runner_opts = RunnerOptions {
    //     name: hedger_context_info.symbol.to_string(),
    //     shutdown: shutdown_sender.clone(),
    //     execution_condition: ExecutionCondition::EventBased,
    //     strategy: hedger.clone(),
    //     context_manager: hedger_context_manager.clone(),
    // };

    // let hedger_runner = Arc::new(Runner::new(hedger_runner_opts));
    // let hedger_runner_clone = hedger_runner.clone();
    // let hedger_runner_handle = tokio::spawn(async move {
    //     match hedger_runner_clone.run().await {
    //         Ok(()) => (),
    //         Err(e) => {
    //             warn!(
    //                 "There was an error running Hedging Strategy Runner: {:?}",
    //                 e.to_string()
    //             );
    //         }
    //     }
    // });

    // only add the necessary subscriptions to the service so the initial account fetching propagates to all listeners
    let streaming_account_service = Arc::new(StreamingAccountInfoService::new(
        accounts_cache.clone(),
        pubsub_client.clone(),
        rpc_client.clone(),
        shutdown_sender.subscribe(),
        &vec![],
    ));

    let mut remaining_accounts = maker_context_info.context_accounts();
    remaining_accounts.extend(hedger_context_info.context_accounts());
    remaining_accounts.extend(vec![
        cache_account::id(),
        user_info.master_account,
        user_info.sub_account,
    ]);
    remaining_accounts.sort_unstable();
    remaining_accounts.dedup(); // do this to avoid unnecessary subscriptions on overlapping accounts
    streaming_account_service
        .add_subscriptions(&remaining_accounts)
        .await;

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
        maker_handle,
    );

    Ok(CliResult {})
}
