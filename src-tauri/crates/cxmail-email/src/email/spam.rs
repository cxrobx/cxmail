use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Naive Bayes spam classifier trained on user's corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpamFilter {
    ham_count: u64,
    spam_count: u64,
    ham_words: HashMap<String, u64>,
    spam_words: HashMap<String, u64>,
}

impl SpamFilter {
    pub fn new() -> Self {
        Self {
            ham_count: 0,
            spam_count: 0,
            ham_words: HashMap::new(),
            spam_words: HashMap::new(),
        }
    }

    /// Load from disk or create new.
    pub fn load(path: &Path) -> Self {
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(filter) = serde_json::from_str(&data) {
                    return filter;
                }
            }
        }
        Self::new()
    }

    /// Save to disk.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let data = serde_json::to_string(self).unwrap_or_default();
        std::fs::write(path, data)
    }

    /// Tokenize text into words.
    fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 3 && w.len() <= 30)
            .map(|w| w.to_string())
            .collect()
    }

    /// Train the filter with a ham (not spam) message.
    pub fn train_ham(&mut self, text: &str) {
        self.ham_count += 1;
        for word in Self::tokenize(text) {
            *self.ham_words.entry(word).or_insert(0) += 1;
        }
    }

    /// Train the filter with a spam message.
    pub fn train_spam(&mut self, text: &str) {
        self.spam_count += 1;
        for word in Self::tokenize(text) {
            *self.spam_words.entry(word).or_insert(0) += 1;
        }
    }

    /// Classify a message. Returns probability it's spam (0.0 to 1.0).
    pub fn classify(&self, text: &str) -> f64 {
        if self.ham_count == 0 || self.spam_count == 0 {
            return 0.5; // Not enough training data
        }

        let words = Self::tokenize(text);
        let total = (self.ham_count + self.spam_count) as f64;
        let prior_spam = self.spam_count as f64 / total;
        let prior_ham = self.ham_count as f64 / total;

        let mut log_spam = prior_spam.ln();
        let mut log_ham = prior_ham.ln();

        let spam_total: u64 = self.spam_words.values().sum();
        let ham_total: u64 = self.ham_words.values().sum();
        let vocab_size = {
            let mut all: std::collections::HashSet<&String> = self.spam_words.keys().collect();
            all.extend(self.ham_words.keys());
            all.len() as f64
        };

        for word in &words {
            // Laplace smoothing
            let spam_freq = *self.spam_words.get(word).unwrap_or(&0) as f64 + 1.0;
            let ham_freq = *self.ham_words.get(word).unwrap_or(&0) as f64 + 1.0;

            log_spam += (spam_freq / (spam_total as f64 + vocab_size)).ln();
            log_ham += (ham_freq / (ham_total as f64 + vocab_size)).ln();
        }

        // Convert log probabilities to probability using log-sum-exp
        let max_log = log_spam.max(log_ham);
        let spam_exp = (log_spam - max_log).exp();
        let ham_exp = (log_ham - max_log).exp();

        spam_exp / (spam_exp + ham_exp)
    }

    /// Returns true if the message is likely spam (threshold: 0.8).
    pub fn is_spam(&self, text: &str) -> bool {
        self.classify(text) > 0.8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_classification() {
        let mut filter = SpamFilter::new();

        // Train with some ham
        for _ in 0..10 {
            filter.train_ham("meeting agenda project update team discussion");
            filter.train_ham("code review pull request deployment pipeline");
        }

        // Train with some spam
        for _ in 0..10 {
            filter.train_spam("buy cheap pills discount offer limited time");
            filter.train_spam("winner lottery claim prize congratulations free");
        }

        assert!(filter.classify("buy cheap discount pills") > 0.7);
        assert!(filter.classify("project code review meeting") < 0.3);
    }
}
