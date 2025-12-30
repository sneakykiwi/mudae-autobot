#![allow(dead_code)]

use crate::client::DiscordClient;
use crate::parser::MudaeMessage;
use crate::wishlist::{WishlistManager, WishedCharacter};
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub original_name: String,
    pub canonical_name: Option<String>,
    pub series: Option<String>,
    pub character_id: Option<String>,
    pub exists: bool,
}

pub struct CharacterVerifier {
    client: DiscordClient,
    cache: Arc<DashMap<String, VerificationResult>>,
    pending_verifications: Arc<DashMap<String, oneshot::Sender<VerificationResult>>>,
    verification_channel: u64,
}

impl CharacterVerifier {
    pub fn new(client: DiscordClient, verification_channel: u64) -> Self {
        Self {
            client,
            cache: Arc::new(DashMap::new()),
            pending_verifications: Arc::new(DashMap::new()),
            verification_channel,
        }
    }

    pub async fn verify_character(&self, name: &str) -> Result<VerificationResult> {
        let name_lower = name.to_lowercase();
        
        if let Some(cached) = self.cache.get(&name_lower) {
            debug!("Using cached verification for '{}'", name);
            return Ok(cached.clone());
        }

        let (tx, rx) = oneshot::channel();
        self.pending_verifications.insert(name_lower.clone(), tx);

        let cmd = format!("$im {}", name);
        self.client.send_message(self.verification_channel, &cmd).await?;
        
        let result = tokio::time::timeout(Duration::from_secs(10), rx).await;
        
        self.pending_verifications.remove(&name_lower);

        match result {
            Ok(Ok(verification)) => {
                self.cache.insert(name_lower, verification.clone());
                Ok(verification)
            }
            Ok(Err(_)) => {
                let not_found = VerificationResult {
                    original_name: name.to_string(),
                    canonical_name: None,
                    series: None,
                    character_id: None,
                    exists: false,
                };
                self.cache.insert(name_lower, not_found.clone());
                Ok(not_found)
            }
            Err(_) => {
                warn!("Verification timed out for '{}'", name);
                let not_found = VerificationResult {
                    original_name: name.to_string(),
                    canonical_name: None,
                    series: None,
                    character_id: None,
                    exists: false,
                };
                Ok(not_found)
            }
        }
    }

    pub fn handle_mudae_response(&self, message: &MudaeMessage) {
        if let MudaeMessage::CharacterInfo { name, series, exists } = message {
            let name_lower = name.to_lowercase();
            
            if let Some((_, tx)) = self.pending_verifications.remove(&name_lower) {
                let result = VerificationResult {
                    original_name: name.clone(),
                    canonical_name: Some(name.clone()),
                    series: Some(series.clone()),
                    character_id: None,
                    exists: *exists,
                };
                let _ = tx.send(result);
            }
            
            for pending in self.pending_verifications.iter() {
                let pending_name = pending.key();
                if name_lower.contains(pending_name) || pending_name.contains(&name_lower) {
                    if let Some((_, tx)) = self.pending_verifications.remove(pending_name) {
                        let result = VerificationResult {
                            original_name: pending_name.clone(),
                            canonical_name: Some(name.clone()),
                            series: Some(series.clone()),
                            character_id: None,
                            exists: *exists,
                        };
                        let _ = tx.send(result);
                    }
                    break;
                }
            }
        }
    }

    pub async fn verify_batch(&self, names: Vec<String>) -> Vec<VerificationResult> {
        let mut results = Vec::new();
        
        for name in names {
            match self.verify_character(&name).await {
                Ok(result) => results.push(result),
                Err(e) => {
                    warn!("Failed to verify '{}': {}", name, e);
                    results.push(VerificationResult {
                        original_name: name,
                        canonical_name: None,
                        series: None,
                        character_id: None,
                        exists: false,
                    });
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        
        results
    }

    pub fn is_cached(&self, name: &str) -> bool {
        self.cache.contains_key(&name.to_lowercase())
    }

    pub fn get_cached(&self, name: &str) -> Option<VerificationResult> {
        self.cache.get(&name.to_lowercase()).map(|r| r.clone())
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
        info!("Verification cache cleared");
    }

    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

pub struct WishlistVerifier {
    verifier: Arc<CharacterVerifier>,
    wishlist: Arc<WishlistManager>,
}

impl WishlistVerifier {
    pub fn new(verifier: Arc<CharacterVerifier>, wishlist: Arc<WishlistManager>) -> Self {
        Self { verifier, wishlist }
    }

    pub async fn verify_unverified_characters(&self) -> Result<VerificationReport> {
        let unverified = self.wishlist.get_unverified().await;
        let total = unverified.len();
        let mut verified_count = 0;
        let mut failed_count = 0;
        let mut results = Vec::new();

        info!("Starting verification of {} unverified characters", total);

        for character in unverified {
            let result = self.verifier.verify_character(&character.name).await?;
            
            if result.exists {
                self.wishlist.update_character_verification(
                    &character.name,
                    true,
                    result.canonical_name.clone(),
                    result.series.clone(),
                    result.character_id.clone(),
                ).await?;
                verified_count += 1;
                info!("Verified: {} -> {:?}", character.name, result.canonical_name);
            } else {
                failed_count += 1;
                warn!("Character not found: {}", character.name);
            }

            results.push(result);
            tokio::time::sleep(Duration::from_secs(3)).await;
        }

        Ok(VerificationReport {
            total,
            verified: verified_count,
            failed: failed_count,
            results,
        })
    }

    pub async fn add_and_verify(&self, name: String, series: Option<String>) -> Result<bool> {
        let result = self.verifier.verify_character(&name).await?;

        if !result.exists {
            warn!("Character '{}' does not exist in Mudae", name);
            return Ok(false);
        }

        let character = WishedCharacter {
            name: result.canonical_name.unwrap_or(name.clone()),
            series: result.series.or(series),
            character_id: result.character_id,
            verified: true,
            added_date: chrono::Utc::now(),
            notes: None,
            priority: 0,
        };

        self.wishlist.add_character(character).await
    }

    pub async fn add_unverified(&self, name: String, series: Option<String>) -> Result<bool> {
        let character = WishedCharacter {
            name,
            series,
            character_id: None,
            verified: false,
            added_date: chrono::Utc::now(),
            notes: Some("Pending verification".to_string()),
            priority: 0,
        };

        self.wishlist.add_character(character).await
    }
}

#[derive(Debug)]
pub struct VerificationReport {
    pub total: usize,
    pub verified: usize,
    pub failed: usize,
    pub results: Vec<VerificationResult>,
}

impl VerificationReport {
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.verified as f64 / self.total as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_result() {
        let result = VerificationResult {
            original_name: "Rem".to_string(),
            canonical_name: Some("Rem".to_string()),
            series: Some("Re:Zero".to_string()),
            character_id: None,
            exists: true,
        };

        assert!(result.exists);
        assert_eq!(result.canonical_name, Some("Rem".to_string()));
    }

    #[test]
    fn test_verification_report() {
        let report = VerificationReport {
            total: 10,
            verified: 8,
            failed: 2,
            results: vec![],
        };

        assert_eq!(report.success_rate(), 80.0);
    }
}
