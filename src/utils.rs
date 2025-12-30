#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use std::time::Instant;

pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.num_seconds();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

pub fn format_timestamp(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

pub fn parse_time(time_str: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    
    let hours: u32 = parts[0].parse().ok()?;
    let minutes: u32 = parts[1].parse().ok()?;
    
    if hours >= 24 || minutes >= 60 {
        return None;
    }
    
    Some((hours, minutes))
}

pub fn random_delay(min_ms: u64, max_ms: u64) -> std::time::Duration {
    let range = max_ms.saturating_sub(min_ms);
    let random_offset = if range > 0 {
        rand::random::<u64>() % range
    } else {
        0
    };
    std::time::Duration::from_millis(min_ms + random_offset)
}

pub fn normalize_character_name(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn normalize_series_name(series: &str) -> String {
    let normalized = series.trim().to_lowercase();
    
    let normalized = normalized
        .replace("!", "")
        .replace("?", "")
        .replace(":", "")
        .replace("'", "")
        .replace("\"", "")
        .replace("-", " ")
        .replace("_", " ");
    
    normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

pub struct RateLimiter {
    last_action: Option<Instant>,
    min_interval: std::time::Duration,
}

impl RateLimiter {
    pub fn new(min_interval_ms: u64) -> Self {
        Self {
            last_action: None,
            min_interval: std::time::Duration::from_millis(min_interval_ms),
        }
    }

    pub async fn wait(&mut self) {
        if let Some(last) = self.last_action {
            let elapsed = last.elapsed();
            if elapsed < self.min_interval {
                let wait_time = self.min_interval - elapsed;
                tokio::time::sleep(wait_time).await;
            }
        }
        self.last_action = Some(Instant::now());
    }

    pub fn can_proceed(&self) -> bool {
        match self.last_action {
            Some(last) => last.elapsed() >= self.min_interval,
            None => true,
        }
    }

    pub fn reset(&mut self) {
        self.last_action = None;
    }
}

pub struct Cooldown {
    cooldowns: std::collections::HashMap<String, Instant>,
    duration: std::time::Duration,
}

impl Cooldown {
    pub fn new(duration_secs: u64) -> Self {
        Self {
            cooldowns: std::collections::HashMap::new(),
            duration: std::time::Duration::from_secs(duration_secs),
        }
    }

    pub fn is_ready(&self, key: &str) -> bool {
        match self.cooldowns.get(key) {
            Some(last) => last.elapsed() >= self.duration,
            None => true,
        }
    }

    pub fn trigger(&mut self, key: &str) {
        self.cooldowns.insert(key.to_string(), Instant::now());
    }

    pub fn remaining(&self, key: &str) -> Option<std::time::Duration> {
        self.cooldowns.get(key).and_then(|last| {
            let elapsed = last.elapsed();
            if elapsed < self.duration {
                Some(self.duration - elapsed)
            } else {
                None
            }
        })
    }

    pub fn clear(&mut self, key: &str) {
        self.cooldowns.remove(key);
    }

    pub fn clear_all(&mut self) {
        self.cooldowns.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::seconds(30)), "30s");
        assert_eq!(format_duration(Duration::seconds(90)), "1m 30s");
        assert_eq!(format_duration(Duration::seconds(3661)), "1h 1m 1s");
    }

    #[test]
    fn test_parse_time() {
        assert_eq!(parse_time("12:30"), Some((12, 30)));
        assert_eq!(parse_time("00:00"), Some((0, 0)));
        assert_eq!(parse_time("23:59"), Some((23, 59)));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("12:60"), None);
        assert_eq!(parse_time("invalid"), None);
    }

    #[test]
    fn test_normalize_character_name() {
        assert_eq!(normalize_character_name("  Rem  "), "rem");
        assert_eq!(normalize_character_name("Emilia-tan"), "emiliatan");
        assert_eq!(normalize_character_name("Kaguya  Shinomiya"), "kaguya shinomiya");
    }

    #[test]
    fn test_normalize_series_name() {
        assert_eq!(normalize_series_name("Re:Zero"), "re zero");
        assert_eq!(normalize_series_name("Kaguya-sama: Love is War"), "kaguya sama love is war");
    }

    #[test]
    fn test_truncate_string() {
        assert_eq!(truncate_string("Hello", 10), "Hello");
        assert_eq!(truncate_string("Hello World!", 8), "Hello...");
    }

    #[test]
    fn test_cooldown() {
        let mut cd = Cooldown::new(1);
        assert!(cd.is_ready("test"));
        cd.trigger("test");
        assert!(!cd.is_ready("test"));
    }
}
