use crate::stats::{ConnectionStatus, EventType, Stats};
use anyhow::{Context as AnyhowContext, Result};
use serenity_self::async_trait;
use serenity_self::client::Context;
use serenity_self::http::Http;
use serenity_self::model::channel::{Channel, Message, Reaction};
use serenity_self::model::gateway::Ready;
use serenity_self::model::id::{ChannelId, GuildId, MessageId};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub enum GatewayEvent {
    Ready { user_id: u64, username: String, session_id: String },
    MessageCreate(DiscordMessage),
    MessageUpdate(DiscordMessage),
    ReactionAdd { message_id: u64, channel_id: u64, user_id: u64, emoji: String },
    Unknown(String),
}

#[derive(Debug, Clone)]
pub struct DiscordMessage {
    pub id: u64,
    pub channel_id: u64,
    pub author: Author,
    pub content: String,
    pub embeds: Vec<Embed>,
    pub components: Vec<Component>,
}

impl From<&Message> for DiscordMessage {
    fn from(msg: &Message) -> Self {
        let components: Vec<Component> = msg.components.iter().map(|row| {
            let buttons: Vec<Button> = row.components.iter().filter_map(|c| {
                let json = serde_json::to_value(c).ok()?;
                if json.get("type")?.as_u64()? == 2 {
                    Some(Button {
                        button_type: 2,
                        style: json.get("style").and_then(|v| v.as_u64()).map(|s| s as u8),
                        label: json.get("label").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        custom_id: json.get("custom_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        emoji: json.get("emoji").and_then(|e| {
                            Some(ButtonEmoji {
                                name: e.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                id: e.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            })
                        }),
                    })
                } else {
                    None
                }
            }).collect();
            
            Component {
                component_type: 1,
                components: buttons,
            }
        }).collect();

        Self {
            id: msg.id.get(),
            channel_id: msg.channel_id.get(),
            author: Author {
                id: msg.author.id.get(),
                username: msg.author.name.clone(),
                bot: msg.author.bot,
            },
            content: msg.content.clone(),
            embeds: msg.embeds.iter().map(|e| e.into()).collect(),
            components,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Author {
    pub id: u64,
    pub username: String,
    pub bot: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Embed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<EmbedAuthor>,
    pub footer: Option<EmbedFooter>,
    pub fields: Option<Vec<EmbedField>>,
    pub image: Option<EmbedImage>,
    pub color: Option<u32>,
}

impl From<&serenity_self::model::channel::Embed> for Embed {
    fn from(embed: &serenity_self::model::channel::Embed) -> Self {
        Self {
            title: embed.title.clone(),
            description: embed.description.clone(),
            author: embed.author.as_ref().map(|a| EmbedAuthor {
                name: a.name.clone(),
            }),
            footer: embed.footer.as_ref().map(|f| EmbedFooter {
                text: f.text.clone(),
            }),
            fields: if embed.fields.is_empty() {
                None
            } else {
                Some(embed.fields.iter().map(|f| EmbedField {
                    name: f.name.clone(),
                    value: f.value.clone(),
                }).collect())
            },
            image: embed.image.as_ref().map(|i| EmbedImage {
                url: i.url.clone(),
            }),
            color: embed.colour.map(|c| c.0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmbedAuthor {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct EmbedFooter {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct EmbedImage {
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct Component {
    pub component_type: u8,
    pub components: Vec<Button>,
}


#[derive(Debug, Clone)]
pub struct Button {
    pub button_type: u8,
    pub style: Option<u8>,
    pub label: Option<String>,
    pub custom_id: Option<String>,
    pub emoji: Option<ButtonEmoji>,
}

#[derive(Debug, Clone)]
pub struct ButtonEmoji {
    pub name: Option<String>,
    pub id: Option<String>,
}

#[derive(Clone)]
pub struct DiscordClient {
    http: Arc<Http>,
    token: String,
    stats: Option<Arc<Stats>>,
}

impl DiscordClient {
    pub fn new(token: String) -> Self {
        let http = Arc::new(Http::new(&token));
        Self {
            http,
            token,
            stats: None,
        }
    }

    pub fn with_stats(mut self, stats: Arc<Stats>) -> Self {
        self.stats = Some(stats);
        self
    }

    pub fn http(&self) -> Arc<Http> {
        self.http.clone()
    }

    async fn update_status(&self, status: ConnectionStatus) {
        if let Some(ref stats) = self.stats {
            stats.set_connection_status(status).await;
        }
    }

    async fn log_event(&self, event_type: EventType, message: String) {
        if let Some(ref stats) = self.stats {
            stats.log_event(event_type, message).await;
        }
    }

    pub async fn send_message(&self, channel_id: u64, content: &str) -> Result<()> {
        let channel_id = ChannelId::new(channel_id);
        channel_id
            .say(&self.http, content)
            .await
            .context("Failed to send message")?;

        debug!("Sent message to channel {}: {}", channel_id.get(), content);
        Ok(())
    }

    pub async fn add_reaction(&self, channel_id: u64, message_id: u64, emoji: &str) -> Result<()> {
        use serenity_self::model::channel::ReactionType;
        
        let channel_id = ChannelId::new(channel_id);
        let message_id = MessageId::new(message_id);
        
        let reaction_type = if emoji.chars().count() == 1 {
            ReactionType::from(emoji.chars().next().unwrap())
        } else {
            ReactionType::Unicode(emoji.to_string())
        };
        
        channel_id
            .create_reaction(&self.http, message_id, reaction_type)
            .await
            .context("Failed to add reaction")?;

        debug!("Added reaction {} to message {}", emoji, message_id.get());
        Ok(())
    }

    pub async fn click_button(
        &self,
        message_id: u64,
        channel_id: u64,
        guild_id: Option<u64>,
        application_id: u64,
        custom_id: &str,
    ) -> Result<()> {
        use serde_json::json;

        let url = "https://discord.com/api/v10/interactions";
        let nonce = format!("{}", rand::random::<u64>());

        let mut payload = json!({
            "type": 3,
            "nonce": nonce,
            "channel_id": channel_id.to_string(),
            "message_id": message_id.to_string(),
            "application_id": application_id.to_string(),
            "data": {
                "component_type": 2,
                "custom_id": custom_id
            }
        });

        if let Some(gid) = guild_id {
            payload["guild_id"] = json!(gid.to_string());
        }

        let client = reqwest::Client::new();
        let response = client
            .post(url)
            .header("Authorization", &self.token)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("Failed to send button click request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to click button: {} - {}", status, text);
        }

        debug!("Clicked button {} on message {}", custom_id, message_id);
        Ok(())
    }

    pub async fn get_channel(&self, channel_id: u64) -> Result<ChannelResponse> {
        let channel_id = ChannelId::new(channel_id);
        let channel = channel_id
            .to_channel(&self.http)
            .await
            .context("Failed to get channel")?;

        let (name, guild_id) = match channel {
            Channel::Guild(ch) => (
                Some(ch.name().to_string()),
                Some(ch.guild_id.get().to_string()),
            ),
            Channel::Private(ch) => (
                Some(ch.recipient.name.clone()),
                None,
            ),
            _ => (None, None),
        };

        Ok(ChannelResponse {
            id: channel_id.get(),
            name,
            guild_id,
        })
    }

    pub async fn get_guild(&self, guild_id: u64) -> Result<GuildResponse> {
        let guild_id = GuildId::new(guild_id);
        let guild = guild_id
            .to_partial_guild(&self.http)
            .await
            .context("Failed to get guild")?;

        Ok(GuildResponse {
            id: guild_id.get(),
            name: guild.name,
        })
    }

    pub async fn get_current_user(&self) -> Result<UserResponse> {
        let user = self.http
            .get_current_user()
            .await
            .context("Failed to get current user")?;

        Ok(UserResponse {
            id: user.id.get(),
            username: user.name.clone(),
            discriminator: user.discriminator.map(|d| format!("{:04}", d.get())),
            global_name: user.global_name.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct UserResponse {
    pub id: u64,
    pub username: String,
    pub discriminator: Option<String>,
    pub global_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChannelResponse {
    pub id: u64,
    pub name: Option<String>,
    pub guild_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GuildResponse {
    pub id: u64,
    pub name: String,
}

pub struct EventHandler {
    event_tx: mpsc::Sender<GatewayEvent>,
    stats: Option<Arc<Stats>>,
}

impl EventHandler {
    pub fn new(event_tx: mpsc::Sender<GatewayEvent>, stats: Option<Arc<Stats>>) -> Self {
        Self { event_tx, stats }
    }

    async fn update_status(&self, status: ConnectionStatus) {
        if let Some(ref stats) = self.stats {
            stats.set_connection_status(status).await;
        }
    }

    async fn log_event(&self, event_type: EventType, message: String) {
        if let Some(ref stats) = self.stats {
            stats.log_event(event_type, message).await;
        }
    }
}

#[async_trait]
impl serenity_self::client::EventHandler for EventHandler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        let user_id = ready.user.id.get();
        let username = ready.user.name.clone();
        let display_name = ready.user.global_name.clone().unwrap_or(username.clone());
        let session_id = ready.session_id.clone();

        debug!("Discord client ready, updating status to Connected");
        self.update_status(ConnectionStatus::Connected).await;
        self.log_event(EventType::Success, format!("Connected as {}", display_name)).await;

        let event = GatewayEvent::Ready {
            user_id,
            username: username.clone(),
            session_id,
        };

        if let Err(e) = self.event_tx.send(event).await {
            warn!("Failed to send Ready event: {}", e);
        } else {
            debug!("Ready event sent successfully");
        }
    }

    async fn message(&self, _ctx: Context, msg: Message) {
        let discord_msg = DiscordMessage::from(&msg);
        let event = GatewayEvent::MessageCreate(discord_msg);

        if let Err(e) = self.event_tx.send(event).await {
            warn!("Failed to send MessageCreate event: {}", e);
        }
    }

    async fn message_update(
        &self,
        _ctx: Context,
        _old: Option<Message>,
        new: Option<Message>,
        _event: serenity_self::model::event::MessageUpdateEvent,
    ) {
        if let Some(msg) = new {
            let discord_msg = DiscordMessage::from(&msg);
            let event = GatewayEvent::MessageUpdate(discord_msg);

            if let Err(e) = self.event_tx.send(event).await {
                warn!("Failed to send MessageUpdate event: {}", e);
            }
        }
    }


    async fn reaction_add(&self, _ctx: Context, reaction: Reaction) {
        let event = GatewayEvent::ReactionAdd {
            message_id: reaction.message_id.get(),
            channel_id: reaction.channel_id.get(),
            user_id: reaction.user_id.map(|id| id.get()).unwrap_or(0),
            emoji: reaction.emoji.to_string(),
        };

        if let Err(e) = self.event_tx.send(event).await {
            warn!("Failed to send ReactionAdd event: {}", e);
        }
    }
}
