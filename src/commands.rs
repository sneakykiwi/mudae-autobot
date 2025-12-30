#![allow(dead_code)]

use crate::client::DiscordClient;
use crate::config::Config;
use crate::stats::{EventType, Stats};
use anyhow::Result;
use chrono::{DateTime, Local, NaiveTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

pub struct CommandExecutor {
    client: DiscordClient,
    config: Config,
    stats: Arc<Stats>,
    roll_cooldowns: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    last_daily: Arc<RwLock<Option<DateTime<Utc>>>>,
    claim_available: Arc<RwLock<bool>>,
}

impl CommandExecutor {
    pub fn new(client: DiscordClient, config: Config, stats: Arc<Stats>) -> Self {
        Self {
            client,
            config,
            stats,
            roll_cooldowns: Arc::new(RwLock::new(HashMap::new())),
            last_daily: Arc::new(RwLock::new(None)),
            claim_available: Arc::new(RwLock::new(true)),
        }
    }

    pub async fn execute_roll(&self, channel_id: u64) -> Result<bool> {
        if !self.config.auto_roll || self.stats.is_paused() {
            return Ok(false);
        }

        let rolls_remaining = self.stats.get_rolls_remaining();
        if rolls_remaining == 0 {
            debug!("No rolls remaining, skipping roll execution");
            return Ok(false);
        }

        let available_commands = self.get_all_available_roll_commands().await;
        if available_commands.is_empty() {
            debug!("No roll commands available (all on cooldown)");
            return Ok(false);
        }

        let mut executed_any = false;
        for cmd in available_commands {
            let current_rolls = self.stats.get_rolls_remaining();
            if current_rolls == 0 {
                debug!("Rolls exhausted during execution, stopping");
                break;
            }

            self.client.send_message(channel_id, &cmd).await?;
            self.update_roll_cooldown(&cmd).await;
            self.stats.increment_rolls_executed();
            self.stats.log_event(EventType::Roll, format!("Executed {}", cmd)).await;
            executed_any = true;
            
            let delay = 500 + rand::random::<u64>() % 1000;
            tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
        }

        Ok(executed_any)
    }

    pub async fn get_all_available_roll_commands(&self) -> Vec<String> {
        let cooldowns = self.roll_cooldowns.read().await;
        let now = Utc::now();
        let cooldown_duration = chrono::Duration::seconds(self.config.roll_cooldown_seconds as i64);

        self.config.roll_commands
            .iter()
            .filter(|cmd| {
                if let Some(last_used) = cooldowns.get(*cmd) {
                    now.signed_duration_since(*last_used) >= cooldown_duration
                } else {
                    true
                }
            })
            .cloned()
            .collect()
    }

    async fn get_available_roll_command(&self) -> Option<String> {
        let available = self.get_all_available_roll_commands().await;
        available.first().cloned()
    }

    async fn update_roll_cooldown(&self, command: &str) {
        let mut cooldowns = self.roll_cooldowns.write().await;
        cooldowns.insert(command.to_string(), Utc::now());
    }

    pub async fn execute_claim(&self, channel_id: u64, message_id: u64) -> Result<()> {
        self.client.add_reaction(channel_id, message_id, "💖").await?;
        debug!("Attempted to claim character (message {})", message_id);
        Ok(())
    }

    pub async fn execute_button_claim(
        &self,
        channel_id: u64,
        message_id: u64,
        button_id: &str,
    ) -> Result<()> {
        self.client.click_button(
            message_id,
            channel_id,
            None,
            Config::mudae_bot_id(),
            button_id,
        ).await?;
        debug!("Clicked claim button {} on message {}", button_id, message_id);
        Ok(())
    }

    pub async fn execute_kakera_react(
        &self,
        channel_id: u64,
        message_id: u64,
        button_id: Option<&str>,
    ) -> Result<()> {
        if !self.config.auto_react_kakera {
            return Ok(());
        }

        if let Some(btn_id) = button_id {
            self.client.click_button(
                message_id,
                channel_id,
                None,
                Config::mudae_bot_id(),
                btn_id,
            ).await?;
            debug!("Clicked kakera button {} on message {}", btn_id, message_id);
        } else {
            if let Err(e) = self.client.add_reaction(channel_id, message_id, "💎").await {
                warn!("Failed to add kakera reaction: {}", e);
            }
        }
        Ok(())
    }

    pub async fn execute_daily_commands(&self, channel_id: u64) -> Result<()> {
        if !self.config.auto_daily {
            return Ok(());
        }

        if !self.should_run_daily().await {
            return Ok(());
        }

        self.client.send_message(channel_id, "$daily").await?;
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        self.client.send_message(channel_id, "$dk").await?;
        self.stats.log_event(EventType::Success, "Executed daily commands".to_string()).await;

        *self.last_daily.write().await = Some(Utc::now());
        Ok(())
    }

    async fn should_run_daily(&self) -> bool {
        let last = self.last_daily.read().await;
        if let Some(last_time) = *last {
            let now = Utc::now();
            let schedule_time = self.parse_schedule_time();
            
            if last_time.date_naive() == now.date_naive() {
                return false;
            }
            
            let current_time = Local::now().time();
            current_time >= schedule_time
        } else {
            true
        }
    }

    fn parse_schedule_time(&self) -> NaiveTime {
        NaiveTime::parse_from_str(&self.config.daily_time, "%H:%M")
            .unwrap_or_else(|_| NaiveTime::from_hms_opt(0, 0, 0).unwrap())
    }

    pub async fn check_rolls(&self, channel_id: u64) -> Result<()> {
        self.client.send_message(channel_id, "$ru").await?;
        Ok(())
    }

    pub async fn check_claim_status(&self, channel_id: u64) -> Result<()> {
        self.client.send_message(channel_id, "$tu").await?;
        Ok(())
    }

    pub async fn verify_character(&self, channel_id: u64, character_name: &str) -> Result<()> {
        let cmd = format!("$im {}", character_name);
        self.client.send_message(channel_id, &cmd).await?;
        debug!("Sent character verification: {}", cmd);
        Ok(())
    }

    pub async fn search_character(&self, channel_id: u64, query: &str) -> Result<()> {
        let cmd = format!("$search {}", query);
        self.client.send_message(channel_id, &cmd).await?;
        debug!("Sent character search: {}", cmd);
        Ok(())
    }

    pub async fn set_claim_available(&self, available: bool) {
        *self.claim_available.write().await = available;
        self.stats.set_claim_available(available);
    }

    pub async fn is_claim_available(&self) -> bool {
        *self.claim_available.read().await
    }

    pub async fn get_time_until_next_roll(&self) -> Option<chrono::Duration> {
        let cooldowns = self.roll_cooldowns.read().await;
        let now = Utc::now();
        let cooldown_secs = self.config.roll_cooldown_seconds as i64;

        let mut min_wait: Option<chrono::Duration> = None;

        for cmd in &self.config.roll_commands {
            if let Some(last_used) = cooldowns.get(cmd) {
                let elapsed = now.signed_duration_since(*last_used);
                let remaining = chrono::Duration::seconds(cooldown_secs) - elapsed;
                
                if remaining > chrono::Duration::zero() {
                    match min_wait {
                        None => min_wait = Some(remaining),
                        Some(current_min) if remaining < current_min => {
                            min_wait = Some(remaining);
                        }
                        _ => {}
                    }
                } else {
                    return Some(chrono::Duration::zero());
                }
            } else {
                return Some(chrono::Duration::zero());
            }
        }

        min_wait
    }

    pub fn get_roll_commands(&self) -> &[String] {
        &self.config.roll_commands
    }

    pub fn is_roll_enabled(&self) -> bool {
        self.config.auto_roll
    }

    pub fn is_kakera_enabled(&self) -> bool {
        self.config.auto_react_kakera
    }

    pub fn is_daily_enabled(&self) -> bool {
        self.config.auto_daily
    }
}

pub struct RollScheduler {
    executor: Arc<CommandExecutor>,
    channels: Vec<u64>,
    stats: Arc<Stats>,
}

impl RollScheduler {
    pub fn new(executor: Arc<CommandExecutor>, channels: Vec<u64>, stats: Arc<Stats>) -> Self {
        Self { executor, channels, stats }
    }

    pub async fn run(&self) {
        self.stats.log_event(EventType::Info, "Roll scheduler started".to_string()).await;
        
        for &channel_id in &self.channels {
            if self.executor.is_daily_enabled() {
                let _ = self.executor.execute_daily_commands(channel_id).await;
            }
        }
        
        loop {
            for &channel_id in &self.channels {
                if !self.executor.is_roll_enabled() || self.stats.is_paused() {
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }

                let rolls_remaining = self.stats.get_rolls_remaining();
                if rolls_remaining == 0 {
                    let time_until_reset = self.stats.format_time_until_roll_reset().await;
                    if time_until_reset == "Unknown" {
                        debug!("No rolls remaining, waiting for reset");
                    } else {
                        debug!("No rolls remaining, waiting for reset ({} remaining)", time_until_reset);
                    }
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                    continue;
                }

                let cmd = self.executor.config.roll_commands.first();
                if let Some(cmd) = cmd {
                    let current_rolls = self.stats.get_rolls_remaining();
                    if current_rolls == 0 {
                        debug!("Rolls exhausted, stopping");
                        continue;
                    }
                    
                    if let Err(e) = self.executor.client.send_message(channel_id, cmd).await {
                        warn!("Failed to send roll command: {}", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        continue;
                    }
                    
                    self.executor.update_roll_cooldown(cmd).await;
                    self.stats.increment_rolls_executed();
                    
                    if current_rolls > 0 {
                        self.stats.set_rolls_remaining(current_rolls - 1);
                    }
                    
                    self.stats.log_event(EventType::Roll, format!("Rolling with {}", cmd)).await;
                    
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                } else {
                    debug!("No roll commands configured");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    }
}
