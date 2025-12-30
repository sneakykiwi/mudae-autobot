#![allow(dead_code)]

use crate::client::{DiscordMessage, Embed};
use regex::Regex;
use std::sync::LazyLock;

static KAKERA_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d+)\s*<:kakera").unwrap()
});

static CLAIM_EMOJI_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(💖|❤️|💕|💗|💘|💝)$").unwrap()
});

#[derive(Debug, Clone)]
pub struct ParsedCharacter {
    pub name: String,
    pub series: String,
    pub kakera_value: Option<u32>,
    pub image_url: Option<String>,
    pub is_claimed: bool,
    pub claim_rank: Option<u32>,
    pub is_wished: bool,
}

#[derive(Debug, Clone)]
pub enum MudaeMessage {
    CharacterRoll {
        character: ParsedCharacter,
        message_id: u64,
        channel_id: u64,
        has_claim_button: bool,
        claim_button_id: Option<String>,
    },
    KakeraLoot {
        message_id: u64,
        channel_id: u64,
        kakera_type: KakeraType,
        button_id: Option<String>,
    },
    CharacterInfo {
        name: String,
        series: String,
        exists: bool,
    },
    RollsRemaining {
        count: u32,
        reset_time: Option<String>,
    },
    ClaimAvailable {
        available: bool,
        reset_time: Option<String>,
    },
    DailyReady,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KakeraType {
    Purple,
    Blue,
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    Rainbow,
    Light,
    Unknown,
}

impl KakeraType {
    pub fn from_color(color: Option<u32>) -> Self {
        match color {
            Some(0x9B59B6) => KakeraType::Purple,
            Some(0x3498DB) => KakeraType::Blue,
            Some(0x1ABC9C) => KakeraType::Teal,
            Some(0x2ECC71) => KakeraType::Green,
            Some(0xF1C40F) => KakeraType::Yellow,
            Some(0xE67E22) => KakeraType::Orange,
            Some(0xE74C3C) => KakeraType::Red,
            Some(0xFFB6C1) => KakeraType::Pink,
            Some(0x00FFFF) => KakeraType::Rainbow,
            Some(0xFFFFFF) => KakeraType::Light,
            _ => KakeraType::Unknown,
        }
    }
}

pub struct MudaeParser;

impl MudaeParser {
    pub fn parse(message: &DiscordMessage) -> MudaeMessage {
        if let Some(embed) = message.embeds.first() {
            if Self::is_character_info(embed) {
                return Self::parse_character_info(embed);
            }
            
            if Self::is_character_roll(embed) {
                return Self::parse_character_roll(message, embed);
            }
            
            if Self::is_kakera_loot(message) {
                return Self::parse_kakera_loot(message);
            }
        }
        
        if Self::is_rolls_info(&message.content) {
            return Self::parse_rolls_info(&message.content);
        }
        
        if Self::is_claim_info(&message.content) {
            return Self::parse_claim_info(&message.content);
        }
        
        MudaeMessage::Unknown
    }

    fn is_character_roll(embed: &Embed) -> bool {
        embed.author.is_some() && embed.description.is_some()
    }

    fn parse_character_roll(message: &DiscordMessage, embed: &Embed) -> MudaeMessage {
        let name = embed.author
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_default();
        
        let description = embed.description.as_deref().unwrap_or("");
        let series = Self::extract_series(description);
        
        let kakera_value = embed.footer
            .as_ref()
            .and_then(|f| Self::extract_kakera_value(&f.text));
        
        let image_url = embed.image.as_ref().map(|i| i.url.clone());
        
        let is_claimed = description.contains("Belongs to");
        let claim_rank = Self::extract_claim_rank(description);
        let is_wished = description.contains("💖") || description.contains("❤️");
        
        let (has_claim_button, claim_button_id) = Self::find_claim_button(&message.components);

        MudaeMessage::CharacterRoll {
            character: ParsedCharacter {
                name,
                series,
                kakera_value,
                image_url,
                is_claimed,
                claim_rank,
                is_wished,
            },
            message_id: message.id,
            channel_id: message.channel_id,
            has_claim_button,
            claim_button_id,
        }
    }

    fn extract_series(description: &str) -> String {
        let lines: Vec<&str> = description.lines().collect();
        if let Some(first_line) = lines.first() {
            first_line.trim().to_string()
        } else {
            String::new()
        }
    }

    fn extract_kakera_value(text: &str) -> Option<u32> {
        KAKERA_REGEX.captures(text)
            .and_then(|caps| caps.get(1))
            .and_then(|m| m.as_str().parse().ok())
    }

    fn extract_claim_rank(description: &str) -> Option<u32> {
        let rank_regex = Regex::new(r"Claims: #(\d+)").ok()?;
        rank_regex.captures(description)
            .and_then(|caps| caps.get(1))
            .and_then(|m| m.as_str().parse().ok())
    }

    fn find_claim_button(components: &[crate::client::Component]) -> (bool, Option<String>) {
        for component in components {
            for button in &component.components {
                if let Some(emoji) = &button.emoji {
                    if let Some(name) = &emoji.name {
                        if CLAIM_EMOJI_REGEX.is_match(name) {
                            return (true, button.custom_id.clone());
                        }
                    }
                }
                if let Some(label) = &button.label {
                    if label.contains("💖") || label.to_lowercase().contains("marry") {
                        return (true, button.custom_id.clone());
                    }
                }
            }
        }
        (false, None)
    }

    fn is_kakera_loot(message: &DiscordMessage) -> bool {
        for component in &message.components {
            for button in &component.components {
                if let Some(emoji) = &button.emoji {
                    if emoji.name.as_deref().map(|n| n.contains("kakera")).unwrap_or(false) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn parse_kakera_loot(message: &DiscordMessage) -> MudaeMessage {
        let embed_color = message.embeds.first().and_then(|e| e.color);
        let kakera_type = KakeraType::from_color(embed_color);
        
        let button_id = message.components.iter()
            .flat_map(|c| &c.components)
            .find(|b| {
                b.emoji.as_ref()
                    .and_then(|e| e.name.as_ref())
                    .map(|n| n.contains("kakera"))
                    .unwrap_or(false)
            })
            .and_then(|b| b.custom_id.clone());

        MudaeMessage::KakeraLoot {
            message_id: message.id,
            channel_id: message.channel_id,
            kakera_type,
            button_id,
        }
    }

    fn is_character_info(embed: &Embed) -> bool {
        (embed.title.is_some() || embed.author.is_some()) &&
        (embed.fields.as_ref().map(|f| !f.is_empty()).unwrap_or(false) ||
         embed.description.is_some())
    }

    fn parse_character_info(embed: &Embed) -> MudaeMessage {
        let name = embed.author
            .as_ref()
            .map(|a| a.name.clone())
            .or_else(|| embed.title.clone())
            .unwrap_or_default();
        
        let series = embed.description
            .as_ref()
            .map(|d| Self::extract_series(d))
            .unwrap_or_default();

        MudaeMessage::CharacterInfo {
            name,
            series,
            exists: true,
        }
    }

    fn is_rolls_info(content: &str) -> bool {
        content.contains("rolls left") 
            || (content.contains("roll") && content.contains("reset"))
            || content.contains("roulette is limited")
    }

    fn parse_rolls_info(content: &str) -> MudaeMessage {
        if content.contains("roulette is limited") {
            let time_regex = Regex::new(r"(\d+)\s*min\s*left").ok();
            let reset_time = time_regex
                .and_then(|r| r.captures(content))
                .and_then(|caps| caps.get(1))
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .map(|minutes| format!("{}m", minutes));
            
            return MudaeMessage::RollsRemaining {
                count: 0,
                reset_time,
            };
        }

        let rolls_regex = Regex::new(r"(\d+)\s*rolls?\s*left").ok();
        let count = rolls_regex
            .and_then(|r| r.captures(content))
            .and_then(|caps| caps.get(1))
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);

        let reset_time_regex = Regex::new(r"reset\s+(?:in\s+)?(\d+)\s*(?:h|hour|hours|hr|hrs)").ok();
        let reset_time = reset_time_regex
            .and_then(|r| r.captures(content))
            .and_then(|caps| caps.get(1))
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .map(|hours| format!("{}h", hours))
            .or_else(|| {
                let reset_time_regex2 = Regex::new(r"reset\s+(?:in\s+)?(\d+)\s*(?:m|min|minute|minutes)").ok();
                reset_time_regex2
                    .and_then(|r| r.captures(content))
                    .and_then(|caps| caps.get(1))
                    .and_then(|m| m.as_str().parse::<i64>().ok())
                    .map(|minutes| format!("{}m", minutes))
            });

        MudaeMessage::RollsRemaining {
            count,
            reset_time,
        }
    }

    fn is_claim_info(content: &str) -> bool {
        content.contains("claim") && (content.contains("available") || content.contains("reset"))
    }

    fn parse_claim_info(content: &str) -> MudaeMessage {
        let available = content.contains("can claim") || content.contains("claim available");
        
        MudaeMessage::ClaimAvailable {
            available,
            reset_time: None,
        }
    }

    pub fn is_claim_emoji(emoji: &str) -> bool {
        CLAIM_EMOJI_REGEX.is_match(emoji)
    }

    pub fn extract_character_name_from_embed(embed: &Embed) -> Option<String> {
        embed.author.as_ref().map(|a| a.name.clone())
    }

    pub fn extract_series_from_embed(embed: &Embed) -> Option<String> {
        embed.description.as_ref().map(|d| Self::extract_series(d))
    }

    pub fn extract_kakera(text: &str) -> Option<u32> {
        Self::extract_kakera_value(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kakera_regex() {
        let text = "123 <:kakera";
        assert!(KAKERA_REGEX.is_match(text));
        
        let caps = KAKERA_REGEX.captures(text).unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "123");
    }

    #[test]
    fn test_claim_emoji() {
        assert!(MudaeParser::is_claim_emoji("💖"));
        assert!(MudaeParser::is_claim_emoji("❤️"));
        assert!(!MudaeParser::is_claim_emoji("💎"));
    }
}
