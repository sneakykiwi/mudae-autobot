mod client;
mod commands;
mod config;
mod database;
mod handler;
mod parser;
mod search;
mod setup;
mod stats;
mod tui;
mod utils;
mod verifier;
mod wishlist;

use crate::client::{DiscordClient, EventHandler};
use crate::commands::{CommandExecutor, RollScheduler};
use crate::config::Config;
use crate::database::{ChannelInfo, Database};
use crate::handler::{run_event_loop, MessageHandler};
use crate::search::create_search_channel;
use crate::stats::Stats;
use crate::verifier::CharacterVerifier;
use crate::wishlist::WishlistManager;
use anyhow::{Context, Result};
use clap::Parser;
use serenity_self::model::gateway::GatewayIntents;
use serenity_self::Client;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{error, info};
use tracing_subscriber::{FmtSubscriber, EnvFilter};

#[derive(Parser, Debug)]
#[command(name = "mudae-selfbot")]
#[command(about = "Discord selfbot for automating Mudae interactions", long_about = None)]
struct Args {
    #[arg(short, long, help = "Your Discord user token (optional, can be set via TUI)")]
    token: Option<String>,
    
    #[arg(short, long, help = "Channel IDs (comma-separated, optional, can be set via TUI)", value_delimiter = ',')]
    channels: Option<Vec<u64>>,

    #[arg(long, help = "Disable TUI and use plain logging instead")]
    no_tui: bool,

    #[arg(long, help = "Force setup wizard even if already configured")]
    setup: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    let db = Arc::new(Database::new().context("Failed to initialize database")?);

    if args.no_tui {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info"));
        let _subscriber = FmtSubscriber::builder()
            .with_env_filter(filter)
            .with_target(false)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .compact()
            .init();
    } else {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("warn"));
        let _subscriber = FmtSubscriber::builder()
            .with_env_filter(filter)
            .with_target(false)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .compact()
            .init();
    }

    if let Some(ref token) = args.token {
        db.save_token(token)?;
    }
    if let Some(ref channels) = args.channels {
        if !channels.is_empty() {
            db.save_channels(channels)?;
        }
    }

    let needs_setup = args.setup || !db.is_configured();
    
    if needs_setup && !args.no_tui {
        let completed = setup::run_setup(db.clone())?;
        if !completed {
            println!("Setup cancelled. Exiting.");
            return Ok(());
        }
    }

    let token = match db.get_token()? {
        Some(t) => t,
        None => {
            if args.no_tui {
                anyhow::bail!("No token configured. Run without --no-tui to set up, or use --token");
            }
            anyhow::bail!("No token configured");
        }
    };

    let channels = db.get_channels()?;
    if channels.is_empty() {
        if args.no_tui {
            anyhow::bail!("No channels configured. Run without --no-tui to set up, or use --channels");
        }
        anyhow::bail!("No channels configured");
    }

    let config = Config::load_from_db(&db);

    let saved_stats = db.load_stats()?;
    let stats = Stats::from_saved(saved_stats);
    stats.set_rolls_remaining(10);

    let client = DiscordClient::new(token.clone()).with_stats(stats.clone());

    if let Ok(user) = client.get_current_user().await {
        let username = user.username.clone();
        let display_name = user.global_name.unwrap_or(username.clone());
        stats.set_username(username.clone()).await;
        let _ = db.save_user_info(&display_name, user.id);
    } else if let Ok(Some(username)) = db.get_username() {
        stats.set_username(username).await;
    }

    let mut channel_infos = db.get_channels_with_names()?;
    
    let channels_clone = channels.clone();
    let client_for_channels = client.clone();
    let db_for_channels = db.clone();
    tokio::spawn(async move {
        for channel_id in channels_clone.iter() {
            if let Ok(channel) = client_for_channels.get_channel(*channel_id).await {
                let guild_name = if let Some(guild_id_str) = &channel.guild_id {
                    if let Ok(guild_id) = guild_id_str.parse::<u64>() {
                        client_for_channels.get_guild(guild_id).await.ok().map(|g| g.name)
                    } else {
                        None
                    }
                } else {
                    None
                };
                
                if let Err(e) = db_for_channels.update_channel_name(
                    *channel_id,
                    channel.name.as_deref().unwrap_or("Unknown"),
                    guild_name.as_deref(),
                ) {
                    error!("Failed to update channel name: {}", e);
                } else {
                    info!("Updated channel info for {}", channel_id);
                }
            }
        }
    });

    if channel_infos.iter().all(|c| c.name.is_none()) {
        channel_infos = channels.iter().map(|&id| ChannelInfo {
            id,
            name: None,
            guild: None,
        }).collect();
    }

    let wishlist = Arc::new(WishlistManager::new(
        config.wishlist_file.clone(),
        config.fuzzy_threshold,
        config.fuzzy_match,
        true,
    ));

    if config.wishlist_enabled {
        wishlist.load().await.context("Failed to load wishlist")?;
    }

    let verification_channel = channels
        .first()
        .copied()
        .unwrap_or(0);

    let verifier = Arc::new(CharacterVerifier::new(
        client.clone(),
        verification_channel,
    ));

    let executor = Arc::new(CommandExecutor::new(client.clone(), config.clone(), stats.clone()));

    let (search_tx, search_rx) = create_search_channel();

    let handler = MessageHandler::new(
        config.clone(),
        executor.clone(),
        wishlist.clone(),
        verifier.clone(),
        stats.clone(),
        channels.clone(),
        client.clone(),
        search_rx,
    );

    let (event_tx, event_rx) = mpsc::channel(100);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let scheduler = RollScheduler::new(
        executor.clone(),
        channels.clone(),
        stats.clone(),
    );

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MESSAGE_REACTIONS;

    stats.set_connection_status(crate::stats::ConnectionStatus::Connecting).await;
    
    let event_handler = EventHandler::new(event_tx.clone(), Some(stats.clone()));
    
    let client_handle = {
        let token = token.clone();
        let stats_for_error = stats.clone();
        tokio::spawn(async move {
            let mut client = match Client::builder(&token, intents)
                .event_handler(event_handler)
                .await
            {
                Ok(client) => client,
                Err(e) => {
                    error!("Failed to create Discord client: {}", e);
                    stats_for_error.set_connection_status(crate::stats::ConnectionStatus::Disconnected).await;
                    return;
                }
            };

            if let Err(e) = client.start().await {
                error!("Client connection error: {}", e);
                stats_for_error.set_connection_status(crate::stats::ConnectionStatus::Disconnected).await;
            }
        })
    };

    let handler_handle = {
        let stats = stats.clone();
        let db = db.clone();
        tokio::spawn(async move {
            run_event_loop(handler, event_rx, stats.clone()).await;
            if let Some(username) = stats.get_username().await {
                let _ = db.save_user_info(&username, stats.get_user_id());
            }
        })
    };

    let scheduler_handle = tokio::spawn(async move {
        scheduler.run().await;
    });

    let stats_save_handle = {
        let stats = stats.clone();
        let db = db.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Err(e) = stats.save_to_db(&db) {
                    error!("Failed to save stats: {}", e);
                }
            }
        })
    };

    let tui_handle = if !args.no_tui {
        let stats = stats.clone();
        let config = config.clone();
        let db = db.clone();
        let wishlist = wishlist.clone();
        let client_for_tui = client.clone();
        Some(tokio::spawn(async move {
            if let Err(e) = tui::run_tui(stats, config, db, wishlist, search_tx, shutdown_rx, channel_infos, Some(client_for_tui)).await {
                error!("TUI error: {}", e);
            }
        }))
    } else {
        None
    };

    if args.no_tui {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            result = client_handle => {
                if let Err(e) = result {
                    error!("Client task panicked: {}", e);
                }
            }
            result = handler_handle => {
                if let Err(e) = result {
                    error!("Handler task panicked: {}", e);
                }
            }
            result = scheduler_handle => {
                if let Err(e) = result {
                    error!("Scheduler task panicked: {}", e);
                }
            }
        }
    } else {
        tokio::select! {
            result = tui_handle.unwrap() => {
                let _ = shutdown_tx.send(true);
                if let Err(e) = result {
                    error!("TUI task panicked: {}", e);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                let _ = shutdown_tx.send(true);
            }
            result = client_handle => {
                let _ = shutdown_tx.send(true);
                if let Err(e) = result {
                    error!("Client task panicked: {}", e);
                }
            }
            result = handler_handle => {
                let _ = shutdown_tx.send(true);
                if let Err(e) = result {
                    error!("Handler task panicked: {}", e);
                }
            }
            result = scheduler_handle => {
                let _ = shutdown_tx.send(true);
                if let Err(e) = result {
                    error!("Scheduler task panicked: {}", e);
                }
            }
        }
    }

    stats_save_handle.abort();
    
    if let Err(e) = stats.save_to_db(&db) {
        error!("Failed to save stats on shutdown: {}", e);
    }
    
    if config.wishlist_enabled {
        if let Err(e) = wishlist.save().await {
            error!("Failed to save wishlist: {}", e);
        }
    }

    Ok(())
}
