use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    pub interval_secs: u64,
    pub change_threshold_pct: f64,
    pub candle_interval: String,
    pub streak_len: usize,
    pub close_delay_secs: u64,
    pub min_quote_volume_24h: f64,
    pub telegram_token: String,
    pub telegram_chat_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            change_threshold_pct: 0.5,
            candle_interval: "1m".to_string(),
            streak_len: 5,
            close_delay_secs: 0,
            min_quote_volume_24h: 100_000_000.0,
            telegram_token: String::new(),
            telegram_chat_id: String::new(),
        }
    }
}

pub fn save_config_file(config: &Config) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, data)?;
    Ok(path)
}

pub fn parse_config() -> Result<(Config, bool), Box<dyn std::error::Error>> {
    let mut config = Config::default();
    if let Some(file_config) = load_config_file()? {
        config = file_config;
    }
    apply_env_overrides(&mut config);

    let mut save_config = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--interval-secs" => {
                let value = args.next().ok_or("missing value for --interval-secs")?;
                config.interval_secs = value.parse()?;
            }
            "--change-pct" => {
                let value = args.next().ok_or("missing value for --change-pct")?;
                config.change_threshold_pct = value.parse()?;
            }
            "--candle-interval" => {
                let value = args.next().ok_or("missing value for --candle-interval")?;
                config.candle_interval = value;
            }
            "--streak-len" => {
                let value = args.next().ok_or("missing value for --streak-len")?;
                config.streak_len = value.parse()?;
            }
            "--close-delay-secs" => {
                let value = args.next().ok_or("missing value for --close-delay-secs")?;
                config.close_delay_secs = value.parse()?;
            }
            "--min-quote-volume-24h" => {
                let value = args
                    .next()
                    .ok_or("missing value for --min-quote-volume-24h")?;
                config.min_quote_volume_24h = value.parse()?;
            }
            "--telegram-token" => {
                let value = args.next().ok_or("missing value for --telegram-token")?;
                config.telegram_token = value;
            }
            "--telegram-chat-id" => {
                let value = args.next().ok_or("missing value for --telegram-chat-id")?;
                config.telegram_chat_id = value;
            }
            "--save-config" => {
                save_config = true;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: crypto_tracker [--interval-secs N] [--change-pct P] [--candle-interval I] [--streak-len N] [--close-delay-secs N] [--min-quote-volume-24h V] [--telegram-token T] [--telegram-chat-id ID] [--save-config]"
                );
                println!(
                    "Defaults: --interval-secs 60, --change-pct 0.5, --candle-interval 1m, --streak-len 5, --close-delay-secs 0, --min-quote-volume-24h 100000000"
                );
                println!("Telegram: set TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID or pass flags.");
                println!("Config: saved/loaded from ~/.crypto_tracker/config.json");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {}", arg).into()),
        }
    }

    if config.telegram_token.is_empty() || config.telegram_chat_id.is_empty() {
        return Err(
            "missing Telegram credentials: set TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID".into(),
        );
    }

    Ok((config, save_config))
}

pub fn try_reload_config() -> Result<Option<Config>, Box<dyn std::error::Error>> {
    let mut config = match load_config_file()? {
        Some(cfg) => cfg,
        None => return Ok(None),
    };
    apply_env_overrides(&mut config);
    Ok(Some(config))
}

fn apply_env_overrides(config: &mut Config) {
    if let Ok(token) = std::env::var("TELEGRAM_BOT_TOKEN") {
        if !token.is_empty() {
            config.telegram_token = token;
        }
    }
    if let Ok(chat_id) = std::env::var("TELEGRAM_CHAT_ID") {
        if !chat_id.is_empty() {
            config.telegram_chat_id = chat_id;
        }
    }
}

fn config_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home)
        .join(".crypto_tracker")
        .join("config.json"))
}

fn load_config_file() -> Result<Option<Config>, Box<dyn std::error::Error>> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    let config: Config = serde_json::from_str(&content)?;
    Ok(Some(config))
}
