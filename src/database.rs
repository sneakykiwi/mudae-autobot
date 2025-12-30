use anyhow::{Context, Result};
use directories::ProjectDirs;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Mutex;
use tracing::{debug, info};

const SCHEMA_SQL: &str = include_str!("../schema.sql");

pub struct Database {
    conn: Mutex<Connection>,
}

#[allow(dead_code)]
impl Database {
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path()?;
        
        info!("Initializing database at: {:?}", db_path);
        
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create data directory")?;
        }

        let db_exists = db_path.exists();
        let conn = Connection::open(&db_path).context("Failed to open database")?;
        
        if !db_exists {
            info!("Creating new database...");
        }

        conn.execute_batch(SCHEMA_SQL).context("Failed to initialize database schema")?;
        
        Self::migrate_existing_tables(&conn)?;
        
        info!("Database ready");
        
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn get_db_path() -> Result<PathBuf> {
        if let Some(proj_dirs) = ProjectDirs::from("com", "mudae", "selfbot") {
            Ok(proj_dirs.data_dir().join("mudae.db"))
        } else {
            Ok(PathBuf::from("mudae.db"))
        }
    }

    fn migrate_existing_tables(conn: &Connection) -> Result<()> {
        Self::add_column_if_missing(conn, "credentials", "username", "TEXT")?;
        Self::add_column_if_missing(conn, "credentials", "user_id", "INTEGER")?;
        Self::add_column_if_missing(conn, "channels", "channel_name", "TEXT")?;
        Self::add_column_if_missing(conn, "channels", "guild_name", "TEXT")?;
        Ok(())
    }

    fn add_column_if_missing(conn: &Connection, table: &str, column: &str, col_type: &str) -> Result<()> {
        let columns: Vec<String> = conn
            .prepare(&format!("PRAGMA table_info({})", table))?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;

        if !columns.contains(&column.to_string()) {
            debug!("Adding column {} to table {}", column, table);
            conn.execute(&format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, col_type), [])?;
        }
        Ok(())
    }

    pub fn save_token(&self, token: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO credentials (id, token, updated_at) VALUES (1, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(id) DO UPDATE SET token = ?, updated_at = CURRENT_TIMESTAMP",
            params![token, token],
        )?;
        Ok(())
    }

    pub fn save_user_info(&self, username: &str, user_id: u64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE credentials SET username = ?, user_id = ? WHERE id = 1",
            params![username, user_id as i64],
        )?;
        Ok(())
    }

    pub fn get_token(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT token FROM credentials WHERE id = 1")?;
        let result = stmt.query_row([], |row| row.get(0));
        match result {
            Ok(token) => Ok(Some(token)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_username(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT username FROM credentials WHERE id = 1")?;
        let result = stmt.query_row([], |row| row.get(0));
        match result {
            Ok(name) => Ok(name),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_channels(&self, channels: &[u64]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM channels", [])?;
        for channel_id in channels {
            conn.execute(
                "INSERT INTO channels (channel_id) VALUES (?)",
                params![*channel_id as i64],
            )?;
        }
        Ok(())
    }

    pub fn save_channel_with_name(&self, channel_id: u64, name: &str, guild: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO channels (channel_id, channel_name, guild_name) VALUES (?, ?, ?)
             ON CONFLICT(channel_id) DO UPDATE SET channel_name = ?, guild_name = ?",
            params![channel_id as i64, name, guild, name, guild],
        )?;
        Ok(())
    }

    pub fn update_channel_name(&self, channel_id: u64, name: &str, guild: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE channels SET channel_name = ?, guild_name = ? WHERE channel_id = ?",
            params![name, guild, channel_id as i64],
        )?;
        Ok(())
    }

    pub fn get_channels(&self) -> Result<Vec<u64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT channel_id FROM channels ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            Ok(id as u64)
        })?;
        
        let mut channels = Vec::new();
        for row in rows {
            channels.push(row?);
        }
        Ok(channels)
    }

    pub fn get_channels_with_names(&self) -> Result<Vec<ChannelInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT channel_id, channel_name, guild_name FROM channels ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok(ChannelInfo {
                id: row.get::<_, i64>(0)? as u64,
                name: row.get(1)?,
                guild: row.get(2)?,
            })
        })?;
        
        let mut channels = Vec::new();
        for row in rows {
            channels.push(row?);
        }
        Ok(channels)
    }

    pub fn add_channel(&self, channel_id: u64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let result = conn.execute(
            "INSERT OR IGNORE INTO channels (channel_id) VALUES (?)",
            params![channel_id as i64],
        )?;
        Ok(result > 0)
    }

    pub fn remove_channel(&self, channel_id: u64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let result = conn.execute(
            "DELETE FROM channels WHERE channel_id = ?",
            params![channel_id as i64],
        )?;
        Ok(result > 0)
    }

    pub fn save_config(&self, config: &SavedConfig) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let roll_commands = config.roll_commands.join(",");
        conn.execute(
            "UPDATE config SET 
                roll_commands = ?,
                roll_cooldown_seconds = ?,
                auto_roll = ?,
                auto_react_kakera = ?,
                auto_daily = ?,
                daily_time = ?,
                wishlist_enabled = ?,
                fuzzy_match = ?,
                fuzzy_threshold = ?
            WHERE id = 1",
            params![
                roll_commands,
                config.roll_cooldown_seconds as i64,
                config.auto_roll as i32,
                config.auto_react_kakera as i32,
                config.auto_daily as i32,
                config.daily_time,
                config.wishlist_enabled as i32,
                config.fuzzy_match as i32,
                config.fuzzy_threshold,
            ],
        )?;
        Ok(())
    }

    pub fn load_config(&self) -> Result<SavedConfig> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT roll_commands, roll_cooldown_seconds, auto_roll, auto_react_kakera,
                    auto_daily, daily_time, wishlist_enabled, fuzzy_match, fuzzy_threshold
             FROM config WHERE id = 1"
        )?;
        
        let result = stmt.query_row([], |row| {
            let roll_commands_str: String = row.get(0)?;
            let roll_commands: Vec<String> = roll_commands_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            
            Ok(SavedConfig {
                roll_commands: if roll_commands.is_empty() {
                    vec!["$wa".to_string(), "$ha".to_string()]
                } else {
                    roll_commands
                },
                roll_cooldown_seconds: row.get::<_, i64>(1)? as u64,
                auto_roll: row.get::<_, i32>(2)? != 0,
                auto_react_kakera: row.get::<_, i32>(3)? != 0,
                auto_daily: row.get::<_, i32>(4)? != 0,
                daily_time: row.get(5)?,
                wishlist_enabled: row.get::<_, i32>(6)? != 0,
                fuzzy_match: row.get::<_, i32>(7)? != 0,
                fuzzy_threshold: row.get(8)?,
            })
        });

        match result {
            Ok(config) => Ok(config),
            Err(_) => Ok(SavedConfig::default()),
        }
    }

    pub fn save_stats(&self, stats: &SavedStats) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE stats SET 
                characters_rolled = ?,
                characters_claimed = ?,
                wishlist_matches = ?,
                kakera_collected = ?,
                rolls_executed = ?,
                total_uptime_seconds = ?,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = 1",
            params![
                stats.characters_rolled as i64,
                stats.characters_claimed as i64,
                stats.wishlist_matches as i64,
                stats.kakera_collected as i64,
                stats.rolls_executed as i64,
                stats.total_uptime_seconds as i64,
            ],
        )?;
        Ok(())
    }

    pub fn load_stats(&self) -> Result<SavedStats> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT characters_rolled, characters_claimed, wishlist_matches, 
                    kakera_collected, rolls_executed, total_uptime_seconds 
             FROM stats WHERE id = 1"
        )?;
        
        let result = stmt.query_row([], |row| {
            Ok(SavedStats {
                characters_rolled: row.get::<_, i64>(0)? as u64,
                characters_claimed: row.get::<_, i64>(1)? as u64,
                wishlist_matches: row.get::<_, i64>(2)? as u64,
                kakera_collected: row.get::<_, i64>(3)? as u64,
                rolls_executed: row.get::<_, i64>(4)? as u64,
                total_uptime_seconds: row.get::<_, i64>(5)? as u64,
            })
        });

        match result {
            Ok(stats) => Ok(stats),
            Err(_) => Ok(SavedStats::default()),
        }
    }

    pub fn has_credentials(&self) -> bool {
        self.get_token().ok().flatten().is_some()
    }

    pub fn has_channels(&self) -> bool {
        self.get_channels().map(|c| !c.is_empty()).unwrap_or(false)
    }

    pub fn is_configured(&self) -> bool {
        self.has_credentials() && self.has_channels()
    }
}

#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub id: u64,
    pub name: Option<String>,
    pub guild: Option<String>,
}

impl ChannelInfo {
    pub fn display_name(&self) -> String {
        match (&self.name, &self.guild) {
            (Some(name), Some(guild)) => format!("#{} ({})", name, guild),
            (Some(name), None) => format!("#{}", name),
            (None, _) => format!("{}", self.id),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SavedStats {
    pub characters_rolled: u64,
    pub characters_claimed: u64,
    pub wishlist_matches: u64,
    pub kakera_collected: u64,
    pub rolls_executed: u64,
    pub total_uptime_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct SavedConfig {
    pub roll_commands: Vec<String>,
    pub roll_cooldown_seconds: u64,
    pub auto_roll: bool,
    pub auto_react_kakera: bool,
    pub auto_daily: bool,
    pub daily_time: String,
    pub wishlist_enabled: bool,
    pub fuzzy_match: bool,
    pub fuzzy_threshold: f64,
}

impl Default for SavedConfig {
    fn default() -> Self {
        Self {
            roll_commands: vec!["$wa".to_string(), "$ha".to_string()],
            roll_cooldown_seconds: 3600,
            auto_roll: true,
            auto_react_kakera: true,
            auto_daily: true,
            daily_time: "00:00".to_string(),
            wishlist_enabled: true,
            fuzzy_match: true,
            fuzzy_threshold: 0.8,
        }
    }
}
