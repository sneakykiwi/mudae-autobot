use crate::client::{DiscordClient, DiscordMessage, GatewayEvent};
use crate::commands::CommandExecutor;
use crate::config::Config;
use crate::parser::{MudaeMessage, MudaeParser, ParsedCharacter};
use crate::search::{SearchRequest, SearchRequestReceiver, SearchResult};
use crate::stats::{ChannelActivity, EventType, RollEntry, Stats};
use crate::verifier::CharacterVerifier;
use crate::wishlist::WishlistManager;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::{debug, warn};

pub struct MessageHandler {
    config: Config,
    executor: Arc<CommandExecutor>,
    wishlist: Arc<WishlistManager>,
    verifier: Arc<CharacterVerifier>,
    stats: Arc<Stats>,
    client: DiscordClient,
    user_id: u64,
    target_channels: Vec<u64>,
    pending_search: Arc<RwLock<Option<(u64, oneshot::Sender<Option<SearchResult>>)>>>,
    search_rx: SearchRequestReceiver,
}

impl MessageHandler {
    pub fn new(
        config: Config,
        executor: Arc<CommandExecutor>,
        wishlist: Arc<WishlistManager>,
        verifier: Arc<CharacterVerifier>,
        stats: Arc<Stats>,
        target_channels: Vec<u64>,
        client: DiscordClient,
        search_rx: SearchRequestReceiver,
    ) -> Self {
        Self {
            config,
            executor,
            wishlist,
            verifier,
            stats,
            client,
            user_id: 0,
            target_channels,
            pending_search: Arc::new(RwLock::new(None)),
            search_rx,
        }
    }

    #[allow(dead_code)]
    pub fn set_user_id(&mut self, user_id: u64) {
        self.user_id = user_id;
    }

    pub async fn handle_event(&mut self, event: GatewayEvent) {
        match event {
            GatewayEvent::Ready { user_id, username, .. } => {
                debug!("Ready event: user_id={}, username={}", user_id, username);
                self.user_id = user_id;
                self.stats.set_user_id(user_id);
                self.stats.set_username(username.clone()).await;
                self.stats.set_connection_status(crate::stats::ConnectionStatus::Connected).await;
                self.stats.log_event(EventType::Success, format!("Connected as {}", username)).await;
            }
            GatewayEvent::MessageCreate(message) => {
                debug!("MessageCreate event received");
                self.handle_message(message).await;
            }
            GatewayEvent::MessageUpdate(message) => {
                debug!("MessageUpdate event received");
                self.handle_message(message).await;
            }
            GatewayEvent::ReactionAdd { message_id, channel_id, user_id, emoji } => {
                debug!("ReactionAdd event: message_id={}, channel_id={}, user_id={}, emoji={}", 
                    message_id, channel_id, user_id, emoji);
                self.handle_reaction(message_id, channel_id, user_id, &emoji).await;
            }
            GatewayEvent::Unknown(event_type) => {
                debug!("Unknown event type: {}", event_type);
            }
        }
    }

    async fn handle_message(&self, message: DiscordMessage) {
        debug!("Handling message: channel={}, author={}, is_target={}", 
            message.channel_id, message.author.username, self.is_target_channel(message.channel_id));
        
        if !self.is_target_channel(message.channel_id) {
            debug!("Message from non-target channel {}, ignoring", message.channel_id);
            return;
        }

        if self.is_mudae_message(&message) {
            debug!("Detected Mudae message from {}", message.author.username);
            self.handle_mudae_message(&message).await;
        } else {
            debug!("Detected user message from {}", message.author.username);
            self.handle_user_message(&message).await;
        }
    }

    async fn handle_user_message(&self, message: &DiscordMessage) {
        if message.content.is_empty() {
            debug!("User message has empty content, skipping");
            return;
        }

        if message.author.id == self.user_id {
            debug!("Skipping bot's own message to prevent duplicate logging");
            return;
        }

        let content = if message.content.len() > 50 {
            format!("{}...", &message.content[..47])
        } else {
            message.content.clone()
        };

        debug!("Adding user message to channel activity: {}: {}", message.author.username, content);
        self.stats.add_channel_activity(ChannelActivity::UserMessage {
            username: message.author.username.clone(),
            content,
        }).await;
        debug!("Channel activity added successfully");
    }

    async fn handle_mudae_message(&self, message: &DiscordMessage) {
        debug!("Processing Mudae message: embeds={}, components={}", 
               message.embeds.len(), message.components.len());

        let username = self.stats.get_username().await;
        let parsed = MudaeParser::parse(message, username.as_deref());
        
        debug!("Parsed message result: {:?}", std::mem::discriminant(&parsed));
        
        match parsed {
            MudaeMessage::CharacterRoll { character, message_id, channel_id, has_claim_button, claim_button_id } => {
                debug!("Character roll detected: {} from {}", character.name, character.series);

                self.stats.add_channel_activity(ChannelActivity::Roll {
                    character_name: character.name.clone(),
                    kakera_value: character.kakera_value,
                    is_wished: character.is_wished,
                    claimed: character.is_claimed,
                }).await;

                self.handle_character_roll(
                    character,
                    message_id,
                    channel_id,
                    has_claim_button,
                    claim_button_id,
                ).await;
            }
            MudaeMessage::KakeraLoot { message_id, channel_id, kakera_type: _, button_id } => {
                self.handle_kakera_loot(message_id, channel_id, button_id).await;
            }
            MudaeMessage::CharacterInfo { name, series, exists } => {
                let mut pending = self.pending_search.write().await;
                if let Some((expected_channel, response_tx)) = pending.take() {
                    if message.channel_id == expected_channel {
                        let image_url = message.embeds.first()
                            .and_then(|e| e.image.as_ref())
                            .map(|i| i.url.clone());
                        
                        let kakera_value = message.embeds.first()
                            .and_then(|e| e.footer.as_ref())
                            .and_then(|f| MudaeParser::extract_kakera(&f.text));

                        let result = SearchResult {
                            name: name.clone(),
                            series: series.clone(),
                            image_url,
                            kakera_value,
                            exists,
                        };
                        let _ = response_tx.send(Some(result));
                    } else {
                        *pending = Some((expected_channel, response_tx));
                    }
                }
                drop(pending);
                
                if exists {
                    let info_msg = format!("{} ({})", name, series);
                    self.stats.add_channel_activity(ChannelActivity::MudaeInfo { message: info_msg }).await;
                }
                
                self.verifier.handle_mudae_response(&MudaeMessage::CharacterInfo {
                    name,
                    series,
                    exists,
                });
            }
            MudaeMessage::RollsRemaining { count, reset_time } => {
                self.stats.set_rolls_remaining(count as u64);
                
                let reset_datetime = reset_time.as_ref().and_then(|rt| {
                    let parsed = Self::parse_reset_time(rt);
                    debug!("Parsing reset time '{}' -> {:?}", rt, parsed);
                    parsed
                });
                self.stats.set_next_roll_reset(reset_datetime).await;
                debug!("Set next roll reset to: {:?}", reset_datetime);
                
                let msg = if count == 0 {
                    format!("No rolls left ({})", reset_time.as_deref().unwrap_or("reset pending"))
                } else {
                    format!("{} rolls remaining", count)
                };
                self.stats.add_channel_activity(ChannelActivity::MudaeInfo { message: msg.clone() }).await;
                self.stats.log_event(EventType::Info, msg).await;
                debug!("Rolls remaining: {}, reset: {:?}", count, reset_time);
            }
            MudaeMessage::ClaimAvailable { available, reset_time } => {
                self.executor.set_claim_available(available).await;
                self.stats.set_claim_available(available);
                let status = if available { "Claim available!" } else { "Claim on cooldown" };
                self.stats.add_channel_activity(ChannelActivity::MudaeInfo { message: status.to_string() }).await;
                self.stats.log_event(EventType::Info, format!("Claim status: {}", status)).await;
                debug!("Claim available: {}, reset: {:?}", available, reset_time);
            }
            MudaeMessage::DailyReady => {
                self.stats.add_channel_activity(ChannelActivity::MudaeInfo { message: "Daily commands ready!".to_string() }).await;
                self.stats.log_event(EventType::Info, "Daily commands ready".to_string()).await;
            }
            MudaeMessage::Unknown => {
                let mut pending = self.pending_search.write().await;
                if let Some((expected_channel, _)) = pending.as_ref() {
                    if message.channel_id == *expected_channel {
                        if let Some(embed) = message.embeds.first() {
                            if let Some(author) = embed.author.as_ref() {
                                let series = embed.description
                                    .as_ref()
                                    .map(|d| d.lines().next().unwrap_or("").trim().to_string())
                                    .unwrap_or_default();
                                
                                let image_url = embed.image.as_ref()
                                    .map(|i| i.url.clone());
                                
                                let kakera_value = embed.footer.as_ref()
                                    .and_then(|f| MudaeParser::extract_kakera(&f.text));
                                
                                if let Some((_, response_tx)) = pending.take() {
                                    let result = SearchResult {
                                        name: author.name.clone(),
                                        series,
                                        image_url,
                                        kakera_value,
                                        exists: true,
                                    };
                                    let _ = response_tx.send(Some(result));
                                }
                            }
                        }
                    }
                }
                drop(pending);

                if let Some(embed) = message.embeds.first() {
                    if let Some(author) = embed.author.as_ref() {
                        let series = embed.description
                            .as_ref()
                            .map(|d| d.lines().next().unwrap_or("").trim().to_string())
                            .unwrap_or_default();
                        
                        if !author.name.is_empty() {
                            let info_msg = if !series.is_empty() {
                                format!("{} ({})", author.name, series)
                            } else {
                                author.name.clone()
                            };
                            self.stats.add_channel_activity(ChannelActivity::MudaeInfo { message: info_msg }).await;
                        }
                    } else if !message.content.is_empty() {
                        let content = if message.content.len() > 50 {
                            format!("{}...", &message.content[..47])
                        } else {
                            message.content.clone()
                        };
                        self.stats.add_channel_activity(ChannelActivity::MudaeInfo { message: content }).await;
                    }
                    
                    debug!("Unknown Mudae message - embed author: {:?}, has_desc: {}, has_image: {}",
                           embed.author.as_ref().map(|a| &a.name),
                           embed.description.is_some(),
                           embed.image.is_some());
                } else if !message.content.is_empty() {
                    let content = if message.content.len() > 50 {
                        format!("{}...", &message.content[..47])
                    } else {
                        message.content.clone()
                    };
                    self.stats.add_channel_activity(ChannelActivity::MudaeInfo { message: content }).await;
                } else {
                    debug!("Unknown Mudae message format (no embeds, no content)");
                }
            }
        }
    }

    async fn handle_search_request(&self, request: SearchRequest) {
        let SearchRequest { query, channel_id, response_tx } = request;
        
        *self.pending_search.write().await = Some((channel_id, response_tx));
        
        let search_cmd = format!("$im {}", query);
        if let Err(e) = self.client.send_message(channel_id, &search_cmd).await {
            warn!("Failed to send search command: {}", e);
            if let Some((_, tx)) = self.pending_search.write().await.take() {
                let _ = tx.send(None);
            }
        }
        
        let pending = self.pending_search.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            if let Some((_, tx)) = pending.write().await.take() {
                let _ = tx.send(None);
            }
        });
    }

    async fn handle_character_roll(
        &self,
        character: ParsedCharacter,
        message_id: u64,
        channel_id: u64,
        has_claim_button: bool,
        claim_button_id: Option<String>,
    ) {
        self.stats.increment_rolled();
        
        let current_rolls = self.stats.get_rolls_remaining();
        if current_rolls > 0 {
            self.stats.set_rolls_remaining(current_rolls - 1);
        }
        
        let roll_entry = RollEntry {
            timestamp: Utc::now(),
            character_name: character.name.clone(),
            series: character.series.clone(),
            kakera_value: character.kakera_value,
            claimed: character.is_claimed,
            is_wished: character.is_wished,
        };
        self.stats.add_roll(roll_entry).await;

        if character.is_claimed {
            debug!("Character already claimed, skipping");
            return;
        }

        if self.stats.is_paused() {
            debug!("Bot is paused, skipping claim");
            return;
        }

        if !self.executor.is_claim_available().await {
            debug!("Claim not available, skipping");
            return;
        }

        let should_claim = self.should_claim_character(&character).await;
        
        if should_claim {
            self.stats.log_event(EventType::Wishlist, format!("Match found: {}", character.name)).await;
            self.stats.increment_wishlist_matches();
            
            let delay = 100 + rand::random::<u64>() % 500;
            tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;

            let claim_result = if let Some(button_id) = claim_button_id {
                match self.executor.execute_button_claim(channel_id, message_id, &button_id).await {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        warn!("Failed to click claim button: {}", e);
                        self.executor.execute_claim(channel_id, message_id).await
                    }
                }
            } else if has_claim_button {
                self.executor.execute_claim(channel_id, message_id).await
            } else {
                self.executor.execute_claim(channel_id, message_id).await
            };

            match claim_result {
                Ok(_) => {
                    self.stats.increment_claimed();
                    self.stats.log_event(EventType::Claim, format!("Claimed: {}", character.name)).await;
                }
                Err(e) => {
                    self.stats.log_event(EventType::Error, format!("Failed to claim {}: {}", character.name, e)).await;
                    warn!("Failed to claim: {}", e);
                }
            }
        }
    }

    async fn should_claim_character(&self, character: &ParsedCharacter) -> bool {
        if character.is_wished {
            return true;
        }

        if self.config.wishlist_enabled {
            if let Some(_wished) = self.wishlist.is_wished(&character.name, Some(&character.series)).await {
                return true;
            }
        }

        false
    }

    async fn handle_kakera_loot(
        &self,
        message_id: u64,
        channel_id: u64,
        button_id: Option<String>,
    ) {
        if !self.config.auto_react_kakera {
            return;
        }

        self.stats.log_event(EventType::Kakera, "Kakera detected".to_string()).await;
        
        let delay = 50 + rand::random::<u64>() % 200;
        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;

        match self.executor.execute_kakera_react(channel_id, message_id, button_id.as_deref()).await {
            Ok(_) => {
                self.stats.increment_kakera();
                self.stats.log_event(EventType::Success, "Kakera collected".to_string()).await;
            }
            Err(e) => {
                self.stats.log_event(EventType::Error, format!("Failed to collect kakera: {}", e)).await;
                warn!("Failed to react to kakera: {}", e);
            }
        }
    }

    async fn handle_reaction(
        &self,
        message_id: u64,
        channel_id: u64,
        user_id: u64,
        emoji: &str,
    ) {
        if user_id == self.user_id {
            return;
        }

        debug!(
            "Reaction added: {} by {} on message {} in channel {}",
            emoji, user_id, message_id, channel_id
        );
    }

    fn is_target_channel(&self, channel_id: u64) -> bool {
        if self.target_channels.is_empty() {
            return true;
        }
        self.target_channels.contains(&channel_id)
    }

    fn is_mudae_message(&self, message: &DiscordMessage) -> bool {
        message.author.id == Config::mudae_bot_id() ||
        message.author.username.to_lowercase().contains("mudae")
    }

    fn parse_reset_time(reset_time_str: &str) -> Option<chrono::DateTime<Utc>> {
        use regex::Regex;
        use chrono::Utc;
        
        let hours_regex = Regex::new(r"(\d+)\s*h(?:our|ours|r|rs)?\s*(?:(\d+)\s*m(?:in|inute|inutes)?)?").ok()?;
        let minutes_regex = Regex::new(r"(\d+)\s*m(?:in|inute|inutes)?").ok()?;
        
        if let Some(caps) = hours_regex.captures(reset_time_str) {
            let hours: i64 = caps.get(1)?.as_str().parse().ok()?;
            let minutes: i64 = caps.get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            Some(Utc::now() + chrono::Duration::hours(hours) + chrono::Duration::minutes(minutes))
        } else if let Some(caps) = minutes_regex.captures(reset_time_str) {
            let minutes: i64 = caps.get(1)?.as_str().parse().ok()?;
            Some(Utc::now() + chrono::Duration::minutes(minutes))
        } else {
            None
        }
    }
}

pub async fn run_event_loop(
    mut handler: MessageHandler,
    mut event_rx: mpsc::Receiver<GatewayEvent>,
    stats: Arc<Stats>,
) {
    stats.log_event(EventType::Info, "Event loop started".to_string()).await;
    debug!("Event loop started, target channels: {:?}", handler.target_channels);
    
    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                debug!("Received gateway event: {:?}", std::mem::discriminant(&event));
                handler.handle_event(event).await;
            }
            Some(search_req) = handler.search_rx.recv() => {
                debug!("Received search request: {}", search_req.query);
                handler.handle_search_request(search_req).await;
            }
            else => {
                debug!("Event loop ending - channel closed");
                break;
            }
        }
    }
    
    stats.log_event(EventType::Warning, "Event loop ended".to_string()).await;
}
