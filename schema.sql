-- Mudae Selfbot Database Schema
-- This file is embedded in the executable and runs on startup

-- Schema version tracking
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY
);

-- User credentials and token storage
CREATE TABLE IF NOT EXISTS credentials (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    token TEXT NOT NULL,
    username TEXT,
    user_id INTEGER,
    updated_at TEXT DEFAULT CURRENT_TIMESTAMP
);

-- Monitored Discord channels
CREATE TABLE IF NOT EXISTS channels (
    id INTEGER PRIMARY KEY,
    channel_id INTEGER NOT NULL UNIQUE,
    channel_name TEXT,
    guild_name TEXT,
    added_at TEXT DEFAULT CURRENT_TIMESTAMP
);

-- Bot configuration
CREATE TABLE IF NOT EXISTS config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    roll_commands TEXT DEFAULT '$ma',
    roll_cooldown_seconds INTEGER DEFAULT 3600,
    auto_roll INTEGER DEFAULT 1,
    auto_react_kakera INTEGER DEFAULT 1,
    auto_daily INTEGER DEFAULT 1,
    daily_time TEXT DEFAULT '00:00',
    wishlist_enabled INTEGER DEFAULT 1,
    fuzzy_match INTEGER DEFAULT 1,
    fuzzy_threshold REAL DEFAULT 0.8
);

-- Runtime statistics
CREATE TABLE IF NOT EXISTS stats (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    characters_rolled INTEGER DEFAULT 0,
    characters_claimed INTEGER DEFAULT 0,
    wishlist_matches INTEGER DEFAULT 0,
    kakera_collected INTEGER DEFAULT 0,
    rolls_executed INTEGER DEFAULT 0,
    total_uptime_seconds INTEGER DEFAULT 0,
    updated_at TEXT DEFAULT CURRENT_TIMESTAMP
);

-- Initialize singleton rows
INSERT OR IGNORE INTO config (id) VALUES (1);
INSERT OR IGNORE INTO stats (id) VALUES (1);
INSERT OR IGNORE INTO schema_version (version) VALUES (1);
