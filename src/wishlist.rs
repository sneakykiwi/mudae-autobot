#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use strsim::normalized_levenshtein;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WishlistData {
    pub characters: Vec<WishedCharacter>,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WishedCharacter {
    pub name: String,
    pub series: Option<String>,
    pub character_id: Option<String>,
    pub verified: bool,
    pub added_date: DateTime<Utc>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub priority: u8,
}

impl Default for WishlistData {
    fn default() -> Self {
        Self {
            characters: Vec::new(),
            last_updated: Utc::now(),
        }
    }
}

pub struct WishlistManager {
    data: Arc<RwLock<WishlistData>>,
    file_path: String,
    fuzzy_threshold: f64,
    fuzzy_enabled: bool,
    priority_verified: bool,
}

impl WishlistManager {
    pub fn new(file_path: String, fuzzy_threshold: f64, fuzzy_enabled: bool, priority_verified: bool) -> Self {
        Self {
            data: Arc::new(RwLock::new(WishlistData::default())),
            file_path,
            fuzzy_threshold,
            fuzzy_enabled,
            priority_verified,
        }
    }

    pub async fn load(&self) -> Result<()> {
        let path = Path::new(&self.file_path);
        if !path.exists() {
            info!("Wishlist file not found, creating new one");
            self.save().await?;
            return Ok(());
        }

        let content = tokio::fs::read_to_string(path)
            .await
            .context("Failed to read wishlist file")?;
        
        let data: WishlistData = serde_json::from_str(&content)
            .context("Failed to parse wishlist file")?;
        
        *self.data.write().await = data;
        info!("Loaded {} characters from wishlist", self.data.read().await.characters.len());
        Ok(())
    }

    pub async fn save(&self) -> Result<()> {
        let mut data = self.data.write().await;
        data.last_updated = Utc::now();
        
        let content = serde_json::to_string_pretty(&*data)
            .context("Failed to serialize wishlist")?;
        
        tokio::fs::write(&self.file_path, content)
            .await
            .context("Failed to write wishlist file")?;
        
        debug!("Saved wishlist to {}", self.file_path);
        Ok(())
    }

    pub async fn add_character(&self, mut character: WishedCharacter) -> Result<bool> {
        let mut data = self.data.write().await;
        
        let exists = data.characters.iter().any(|c| {
            c.name.to_lowercase() == character.name.to_lowercase()
        });

        if exists {
            warn!("Character '{}' already in wishlist", character.name);
            return Ok(false);
        }

        character.added_date = Utc::now();
        data.characters.push(character.clone());
        drop(data);
        
        self.save().await?;
        info!("Added '{}' to wishlist", character.name);
        Ok(true)
    }

    pub async fn remove_character(&self, name: &str) -> Result<bool> {
        let mut data = self.data.write().await;
        let initial_len = data.characters.len();
        
        data.characters.retain(|c| {
            c.name.to_lowercase() != name.to_lowercase()
        });

        let removed = data.characters.len() < initial_len;
        drop(data);

        if removed {
            self.save().await?;
            info!("Removed '{}' from wishlist", name);
        }
        Ok(removed)
    }

    pub async fn update_character_verification(
        &self,
        name: &str,
        verified: bool,
        canonical_name: Option<String>,
        series: Option<String>,
        character_id: Option<String>,
    ) -> Result<bool> {
        let mut data = self.data.write().await;
        
        let character = data.characters.iter_mut().find(|c| {
            c.name.to_lowercase() == name.to_lowercase()
        });

        if let Some(c) = character {
            c.verified = verified;
            if let Some(cn) = canonical_name {
                c.name = cn;
            }
            if series.is_some() {
                c.series = series;
            }
            if character_id.is_some() {
                c.character_id = character_id;
            }
            drop(data);
            self.save().await?;
            info!("Updated verification for '{}'", name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn get_characters(&self) -> Vec<WishedCharacter> {
        self.data.read().await.characters.clone()
    }

    pub async fn is_wished(&self, name: &str, series: Option<&str>) -> Option<WishedCharacter> {
        let data = self.data.read().await;
        
        for character in &data.characters {
            if self.matches_character(character, name, series) {
                return Some(character.clone());
            }
        }
        None
    }

    fn matches_character(&self, wished: &WishedCharacter, name: &str, series: Option<&str>) -> bool {
        let name_lower = name.to_lowercase();
        let wished_name_lower = wished.name.to_lowercase();

        if wished_name_lower == name_lower {
            return self.matches_series(wished, series);
        }

        if self.fuzzy_enabled {
            let similarity = normalized_levenshtein(&wished_name_lower, &name_lower);
            if similarity >= self.fuzzy_threshold {
                return self.matches_series(wished, series);
            }
        }

        false
    }

    fn matches_series(&self, wished: &WishedCharacter, series: Option<&str>) -> bool {
        match (&wished.series, series) {
            (Some(wished_series), Some(rolled_series)) => {
                let ws_lower = wished_series.to_lowercase();
                let rs_lower = rolled_series.to_lowercase();
                
                if ws_lower == rs_lower {
                    return true;
                }

                if self.fuzzy_enabled {
                    let similarity = normalized_levenshtein(&ws_lower, &rs_lower);
                    return similarity >= self.fuzzy_threshold;
                }
                false
            }
            (None, _) => true,
            (Some(_), None) => true,
        }
    }

    pub async fn get_all(&self) -> Vec<WishedCharacter> {
        let data = self.data.read().await;
        let mut characters = data.characters.clone();
        
        if self.priority_verified {
            characters.sort_by(|a, b| {
                match (a.verified, b.verified) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => b.priority.cmp(&a.priority),
                }
            });
        } else {
            characters.sort_by(|a, b| b.priority.cmp(&a.priority));
        }
        
        characters
    }

    pub async fn get_unverified(&self) -> Vec<WishedCharacter> {
        let data = self.data.read().await;
        data.characters.iter()
            .filter(|c| !c.verified)
            .cloned()
            .collect()
    }

    pub async fn get_verified(&self) -> Vec<WishedCharacter> {
        let data = self.data.read().await;
        data.characters.iter()
            .filter(|c| c.verified)
            .cloned()
            .collect()
    }

    pub async fn search(&self, query: &str) -> Vec<WishedCharacter> {
        let data = self.data.read().await;
        let query_lower = query.to_lowercase();
        
        data.characters.iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&query_lower) ||
                c.series.as_ref()
                    .map(|s| s.to_lowercase().contains(&query_lower))
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    pub async fn count(&self) -> usize {
        self.data.read().await.characters.len()
    }

    pub async fn set_priority(&self, name: &str, priority: u8) -> Result<bool> {
        let mut data = self.data.write().await;
        
        let character = data.characters.iter_mut().find(|c| {
            c.name.to_lowercase() == name.to_lowercase()
        });

        if let Some(c) = character {
            c.priority = priority;
            drop(data);
            self.save().await?;
            info!("Set priority {} for '{}'", priority, name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn import(&self, characters: Vec<WishedCharacter>) -> Result<usize> {
        let mut data = self.data.write().await;
        let mut added = 0;

        for character in characters {
            let exists = data.characters.iter().any(|c| {
                c.name.to_lowercase() == character.name.to_lowercase()
            });

            if !exists {
                data.characters.push(character);
                added += 1;
            }
        }

        drop(data);
        self.save().await?;
        info!("Imported {} characters", added);
        Ok(added)
    }

    pub async fn export(&self) -> Result<String> {
        let data = self.data.read().await;
        serde_json::to_string_pretty(&*data).context("Failed to export wishlist")
    }

    pub async fn clear(&self) -> Result<usize> {
        let mut data = self.data.write().await;
        let count = data.characters.len();
        data.characters.clear();
        drop(data);
        self.save().await?;
        info!("Cleared {} characters from wishlist", count);
        Ok(count)
    }
}

impl WishedCharacter {
    pub fn new(name: String) -> Self {
        Self {
            name,
            series: None,
            character_id: None,
            verified: false,
            added_date: Utc::now(),
            notes: None,
            priority: 0,
        }
    }

    pub fn with_series(mut self, series: String) -> Self {
        self.series = Some(series);
        self
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_notes(mut self, notes: String) -> Self {
        self.notes = Some(notes);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_matching() {
        let threshold = 0.8;
        
        let sim1 = normalized_levenshtein("rem", "rem");
        assert!(sim1 >= threshold);
        
        let sim2 = normalized_levenshtein("rem", "ram");
        assert!(sim2 < threshold);
        
        let sim3 = normalized_levenshtein("emilia", "emilia");
        assert!(sim3 >= threshold);
    }

    #[test]
    fn test_wished_character_builder() {
        let char = WishedCharacter::new("Rem".to_string())
            .with_series("Re:Zero".to_string())
            .with_priority(10)
            .with_notes("Best girl".to_string());
        
        assert_eq!(char.name, "Rem");
        assert_eq!(char.series, Some("Re:Zero".to_string()));
        assert_eq!(char.priority, 10);
        assert_eq!(char.notes, Some("Best girl".to_string()));
    }
}
