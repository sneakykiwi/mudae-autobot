use crate::database::{Database, SavedStats};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct ActivityEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct RollEntry {
    pub timestamp: DateTime<Utc>,
    pub character_name: String,
    pub series: String,
    pub kakera_value: Option<u32>,
    pub claimed: bool,
    pub is_wished: bool,
}

#[derive(Debug, Clone)]
pub enum ChannelActivity {
    Roll {
        character_name: String,
        kakera_value: Option<u32>,
        is_wished: bool,
        claimed: bool,
    },
    UserMessage {
        username: String,
        content: String,
    },
    MudaeInfo {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventType {
    Info,
    Success,
    Warning,
    Error,
    Roll,
    Claim,
    Kakera,
    Wishlist,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

#[allow(dead_code)]
pub struct Stats {
    pub start_time: DateTime<Utc>,
    pub characters_rolled: AtomicU64,
    pub characters_claimed: AtomicU64,
    pub wishlist_matches: AtomicU64,
    pub kakera_collected: AtomicU64,
    pub rolls_executed: AtomicU64,
    pub total_uptime_seconds: AtomicU64,
    pub connection_status: RwLock<ConnectionStatus>,
    pub claim_available: AtomicBool,
    pub rolls_remaining: AtomicU64,
    pub next_roll_reset: RwLock<Option<DateTime<Utc>>>,
    pub next_claim_reset: RwLock<Option<DateTime<Utc>>>,
    pub activity_log: RwLock<VecDeque<ActivityEvent>>,
    pub roll_history: RwLock<VecDeque<RollEntry>>,
    pub channel_activity: RwLock<VecDeque<ChannelActivity>>,
    pub user_id: AtomicU64,
    pub username: RwLock<Option<String>>,
    pub paused: AtomicBool,
    max_log_entries: usize,
    max_channel_activity: usize,
}

impl Stats {
    #[allow(dead_code)]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            start_time: Utc::now(),
            characters_rolled: AtomicU64::new(0),
            characters_claimed: AtomicU64::new(0),
            wishlist_matches: AtomicU64::new(0),
            kakera_collected: AtomicU64::new(0),
            rolls_executed: AtomicU64::new(0),
            total_uptime_seconds: AtomicU64::new(0),
            connection_status: RwLock::new(ConnectionStatus::Disconnected),
            claim_available: AtomicBool::new(true),
            rolls_remaining: AtomicU64::new(0),
            next_roll_reset: RwLock::new(None),
            next_claim_reset: RwLock::new(None),
            activity_log: RwLock::new(VecDeque::with_capacity(100)),
            roll_history: RwLock::new(VecDeque::with_capacity(50)),
            channel_activity: RwLock::new(VecDeque::with_capacity(50)),
            user_id: AtomicU64::new(0),
            username: RwLock::new(None),
            paused: AtomicBool::new(false),
            max_log_entries: 100,
            max_channel_activity: 50,
        })
    }

    pub fn from_saved(saved: SavedStats) -> Arc<Self> {
        Arc::new(Self {
            start_time: Utc::now(),
            characters_rolled: AtomicU64::new(saved.characters_rolled),
            characters_claimed: AtomicU64::new(saved.characters_claimed),
            wishlist_matches: AtomicU64::new(saved.wishlist_matches),
            kakera_collected: AtomicU64::new(saved.kakera_collected),
            rolls_executed: AtomicU64::new(saved.rolls_executed),
            total_uptime_seconds: AtomicU64::new(saved.total_uptime_seconds),
            connection_status: RwLock::new(ConnectionStatus::Disconnected),
            claim_available: AtomicBool::new(true),
            rolls_remaining: AtomicU64::new(0),
            next_roll_reset: RwLock::new(None),
            next_claim_reset: RwLock::new(None),
            activity_log: RwLock::new(VecDeque::with_capacity(100)),
            roll_history: RwLock::new(VecDeque::with_capacity(50)),
            channel_activity: RwLock::new(VecDeque::with_capacity(50)),
            user_id: AtomicU64::new(0),
            username: RwLock::new(None),
            paused: AtomicBool::new(false),
            max_log_entries: 100,
            max_channel_activity: 50,
        })
    }

    pub fn to_saved(&self) -> SavedStats {
        let session_uptime = self.uptime().num_seconds().max(0) as u64;
        SavedStats {
            characters_rolled: self.get_rolled(),
            characters_claimed: self.get_claimed(),
            wishlist_matches: self.get_wishlist_matches(),
            kakera_collected: self.get_kakera(),
            rolls_executed: self.get_rolls_executed(),
            total_uptime_seconds: self.total_uptime_seconds.load(Ordering::Relaxed) + session_uptime,
        }
    }

    pub fn save_to_db(&self, db: &Database) -> anyhow::Result<()> {
        let saved = self.to_saved();
        db.save_stats(&saved)
    }

    pub fn increment_rolled(&self) {
        self.characters_rolled.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_claimed(&self) {
        self.characters_claimed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_wishlist_matches(&self) {
        self.wishlist_matches.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_kakera(&self) {
        self.kakera_collected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_rolls_executed(&self) {
        self.rolls_executed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_rolled(&self) -> u64 {
        self.characters_rolled.load(Ordering::Relaxed)
    }

    pub fn get_claimed(&self) -> u64 {
        self.characters_claimed.load(Ordering::Relaxed)
    }

    pub fn get_wishlist_matches(&self) -> u64 {
        self.wishlist_matches.load(Ordering::Relaxed)
    }

    pub fn get_kakera(&self) -> u64 {
        self.kakera_collected.load(Ordering::Relaxed)
    }

    pub fn get_rolls_executed(&self) -> u64 {
        self.rolls_executed.load(Ordering::Relaxed)
    }

    pub fn get_total_uptime_seconds(&self) -> u64 {
        let past = self.total_uptime_seconds.load(Ordering::Relaxed);
        let session = self.uptime().num_seconds().max(0) as u64;
        past + session
    }

    pub async fn set_connection_status(&self, status: ConnectionStatus) {
        *self.connection_status.write().await = status;
    }

    pub async fn get_connection_status(&self) -> ConnectionStatus {
        *self.connection_status.read().await
    }

    pub fn set_claim_available(&self, available: bool) {
        self.claim_available.store(available, Ordering::Relaxed);
    }

    pub fn is_claim_available(&self) -> bool {
        self.claim_available.load(Ordering::Relaxed)
    }

    pub fn set_rolls_remaining(&self, count: u64) {
        self.rolls_remaining.store(count, Ordering::Relaxed);
    }

    pub fn get_rolls_remaining(&self) -> u64 {
        self.rolls_remaining.load(Ordering::Relaxed)
    }

    pub async fn set_next_roll_reset(&self, reset_time: Option<DateTime<Utc>>) {
        *self.next_roll_reset.write().await = reset_time;
    }

    pub async fn get_next_roll_reset(&self) -> Option<DateTime<Utc>> {
        *self.next_roll_reset.read().await
    }

    pub async fn format_time_until_roll_reset(&self) -> String {
        if let Some(reset_time) = self.get_next_roll_reset().await {
            let now = Utc::now();
            if reset_time > now {
                let duration = reset_time.signed_duration_since(now);
                let hours = duration.num_hours();
                let minutes = duration.num_minutes() % 60;
                let seconds = duration.num_seconds() % 60;
                if hours > 0 {
                    format!("{}h {}m {}s", hours, minutes, seconds)
                } else if minutes > 0 {
                    format!("{}m {}s", minutes, seconds)
                } else {
                    format!("{}s", seconds)
                }
            } else {
                "Available".to_string()
            }
        } else {
            "Unknown".to_string()
        }
    }

    pub fn set_user_id(&self, id: u64) {
        self.user_id.store(id, Ordering::Relaxed);
    }

    pub fn get_user_id(&self) -> u64 {
        self.user_id.load(Ordering::Relaxed)
    }

    pub async fn set_username(&self, name: String) {
        *self.username.write().await = Some(name);
    }

    pub async fn get_username(&self) -> Option<String> {
        self.username.read().await.clone()
    }

    pub async fn log_event(&self, event_type: EventType, message: String) {
        let event = ActivityEvent {
            timestamp: Utc::now(),
            event_type,
            message,
        };
        
        let mut log = self.activity_log.write().await;
        if log.len() >= self.max_log_entries {
            log.pop_front();
        }
        log.push_back(event);
    }

    pub async fn add_roll(&self, entry: RollEntry) {
        let mut history = self.roll_history.write().await;
        if history.len() >= 50 {
            history.pop_front();
        }
        history.push_back(entry);
    }

    pub async fn get_activity_log(&self) -> Vec<ActivityEvent> {
        self.activity_log.read().await.iter().cloned().collect()
    }

    pub async fn get_roll_history(&self) -> Vec<RollEntry> {
        self.roll_history.read().await.iter().cloned().collect()
    }

    pub async fn add_channel_activity(&self, activity: ChannelActivity) {
        let mut feed = self.channel_activity.write().await;
        debug!("Adding channel activity, current size: {}, max: {}", feed.len(), self.max_channel_activity);
        if feed.len() >= self.max_channel_activity {
            feed.pop_front();
        }
        feed.push_back(activity.clone());
        debug!("Channel activity added, new size: {}", feed.len());
    }

    pub async fn get_channel_activity(&self) -> Vec<ChannelActivity> {
        let activities: Vec<ChannelActivity> = self.channel_activity.read().await.iter().cloned().collect();
        debug!("get_channel_activity returning {} activities", activities.len());
        activities
    }

    pub fn uptime(&self) -> chrono::Duration {
        Utc::now().signed_duration_since(self.start_time)
    }

    pub fn format_uptime(&self) -> String {
        let duration = self.uptime();
        let hours = duration.num_hours();
        let minutes = duration.num_minutes() % 60;
        let seconds = duration.num_seconds() % 60;
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    }

    pub fn format_total_uptime(&self) -> String {
        let total_seconds = self.get_total_uptime_seconds();
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn toggle_paused(&self) -> bool {
        let was_paused = self.paused.fetch_xor(true, Ordering::Relaxed);
        !was_paused
    }
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            start_time: Utc::now(),
            characters_rolled: AtomicU64::new(0),
            characters_claimed: AtomicU64::new(0),
            wishlist_matches: AtomicU64::new(0),
            kakera_collected: AtomicU64::new(0),
            rolls_executed: AtomicU64::new(0),
            total_uptime_seconds: AtomicU64::new(0),
            connection_status: RwLock::new(ConnectionStatus::Disconnected),
            claim_available: AtomicBool::new(true),
            rolls_remaining: AtomicU64::new(0),
            next_roll_reset: RwLock::new(None),
            next_claim_reset: RwLock::new(None),
            activity_log: RwLock::new(VecDeque::with_capacity(100)),
            roll_history: RwLock::new(VecDeque::with_capacity(50)),
            channel_activity: RwLock::new(VecDeque::with_capacity(50)),
            user_id: AtomicU64::new(0),
            username: RwLock::new(None),
            paused: AtomicBool::new(false),
            max_log_entries: 100,
            max_channel_activity: 50,
        }
    }
}
