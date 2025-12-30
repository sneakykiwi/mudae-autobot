use crate::database::{Database, SavedConfig};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Config {
    pub roll_commands: Vec<String>,
    pub roll_cooldown_seconds: u64,
    pub auto_roll: bool,
    pub auto_react_kakera: bool,
    pub auto_daily: bool,
    pub daily_time: String,
    pub wishlist_enabled: bool,
    pub wishlist_file: String,
    pub auto_verify: bool,
    pub fuzzy_match: bool,
    pub fuzzy_threshold: f64,
}

impl Config {
    pub fn load_from_db(db: &Arc<Database>) -> Self {
        let saved = db.load_config().unwrap_or_default();
        Self::from_saved(saved)
    }

    pub fn save_to_db(&self, db: &Database) -> anyhow::Result<()> {
        let saved = SavedConfig {
            roll_commands: self.roll_commands.clone(),
            roll_cooldown_seconds: self.roll_cooldown_seconds,
            auto_roll: self.auto_roll,
            auto_react_kakera: self.auto_react_kakera,
            auto_daily: self.auto_daily,
            daily_time: self.daily_time.clone(),
            wishlist_enabled: self.wishlist_enabled,
            fuzzy_match: self.fuzzy_match,
            fuzzy_threshold: self.fuzzy_threshold,
        };
        db.save_config(&saved)
    }

    pub fn from_saved(saved: SavedConfig) -> Self {
        Self {
            roll_commands: saved.roll_commands,
            roll_cooldown_seconds: saved.roll_cooldown_seconds,
            auto_roll: saved.auto_roll,
            auto_react_kakera: saved.auto_react_kakera,
            auto_daily: saved.auto_daily,
            daily_time: saved.daily_time,
            wishlist_enabled: saved.wishlist_enabled,
            wishlist_file: "wishlist.json".to_string(),
            auto_verify: true,
            fuzzy_match: saved.fuzzy_match,
            fuzzy_threshold: saved.fuzzy_threshold,
        }
    }

    pub fn mudae_bot_id() -> u64 {
        432610292342587392
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            roll_commands: vec!["$wa".to_string(), "$ha".to_string()],
            roll_cooldown_seconds: 3600,
            auto_roll: true,
            auto_react_kakera: true,
            auto_daily: true,
            daily_time: "00:00".to_string(),
            wishlist_enabled: true,
            wishlist_file: "wishlist.json".to_string(),
            auto_verify: true,
            fuzzy_match: true,
            fuzzy_threshold: 0.8,
        }
    }
}
