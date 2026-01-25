use chrono::{DateTime, Datelike, Local, Timelike, Weekday};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Deserialize)]
struct ExchangeInfo {
    symbols: Vec<SymbolInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolInfo {
    symbol: String,
    status: String,
    quote_asset: String,
}

#[derive(Deserialize, Serialize, Clone)]
struct Config {
    interval_secs: u64,
    change_threshold_pct: f64,
    candle_interval: String,
    streak_len: usize,
    telegram_token: String,
    telegram_chat_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            change_threshold_pct: 0.5,
            candle_interval: "1m".to_string(),
            streak_len: 5,
            telegram_token: String::new(),
            telegram_chat_id: String::new(),
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

fn save_config_file(config: &Config) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, data)?;
    Ok(path)
}

fn parse_config() -> Result<(Config, bool), Box<dyn std::error::Error>> {
    let mut config = Config::default();
    if let Some(file_config) = load_config_file()? {
        config = file_config;
    }

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
                    "Usage: crypto_tracker [--interval-secs N] [--change-pct P] [--candle-interval I] [--streak-len N] [--telegram-token T] [--telegram-chat-id ID] [--save-config]"
                );
                println!(
                    "Defaults: --interval-secs 30, --change-pct 1.0, --candle-interval 1m, --streak-len 3"
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

fn sleep_until_interval_boundary(interval_secs: u64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0));
    let now_ms = now.as_secs() * 1000 + u64::from(now.subsec_millis());
    let interval_ms = interval_secs * 1000;
    if interval_ms == 0 {
        return;
    }
    let remainder = now_ms % interval_ms;
    if remainder == 0 {
        return;
    }
    let sleep_ms = interval_ms - remainder;
    std::thread::sleep(Duration::from_millis(sleep_ms));
}

fn format_timestamp_de(time: DateTime<Local>) -> String {
    let weekday = match time.weekday() {
        Weekday::Mon => "Montag",
        Weekday::Tue => "Dienstag",
        Weekday::Wed => "Mittwoch",
        Weekday::Thu => "Donnerstag",
        Weekday::Fri => "Freitag",
        Weekday::Sat => "Samstag",
        Weekday::Sun => "Sonntag",
    };
    format!(
        "{}, {:02}.{:02}.{:04} {:02}:{:02} Uhr",
        weekday,
        time.day(),
        time.month(),
        time.year(),
        time.hour(),
        time.minute()
    )
}

fn send_telegram_message(
    client: &reqwest::blocking::Client,
    token: &str,
    chat_id: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "MarkdownV2",
        "disable_web_page_preview": true
    });
    let response = client.post(url).json(&payload).send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("telegram sendMessage failed: {} {}", status, body).into());
    }
    Ok(())
}

fn escape_markdown_v2(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '_' | '*' | '[' | ']' | '(' | ')' | '~' | '`' | '>' | '#' | '+' | '-' | '=' | '|'
            | '{' | '}' | '.' | '!' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn escape_markdown_url(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' | ')' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (config, save_config) = parse_config()?;
    if save_config {
        let path = save_config_file(&config)?;
        println!("Saved config to {}", path.display());
    }
    if config.interval_secs % 60 != 0 {
        return Err("--interval-secs must be a multiple of 60 for full-minute polling".into());
    }
    let client = reqwest::blocking::Client::new();

    loop {
        sleep_until_interval_boundary(config.interval_secs);
        let request_time = Local::now();
        let request_ts = format_timestamp_de(request_time);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;
        let exchange_info_url = "https://api.binance.com/api/v3/exchangeInfo";
        let response = client.get(exchange_info_url).send()?.error_for_status()?;
        let info: ExchangeInfo = response.json()?;

        let mut tradable: Vec<&SymbolInfo> = info
            .symbols
            .iter()
            .filter(|symbol| symbol.status == "TRADING" && symbol.quote_asset == "USDT")
            .collect();

        tradable.sort_by(|a, b| a.symbol.cmp(&b.symbol));

        let symbols: Vec<String> = tradable.iter().map(|s| s.symbol.clone()).collect();
        if symbols.is_empty() {
            return Err("no USDT trading symbols found".into());
        }

        if config.streak_len == 0 {
            return Err("--streak-len must be at least 1".into());
        }

        for symbol in symbols {
            let kline_url = "https://api.binance.com/api/v3/klines";
            let limit = config.streak_len + 2;
            let limit_str = limit.to_string();
            let response = client
                .get(kline_url)
                .query(&[
                    ("symbol", symbol.as_str()),
                    ("interval", config.candle_interval.as_str()),
                    ("limit", limit_str.as_str()),
                ])
                .send()?
                .error_for_status()?;
            let klines: Vec<Vec<serde_json::Value>> = response.json()?;
            if klines.len() < limit {
                continue;
            }

            let mut closes: Vec<f64> = Vec::with_capacity(klines.len());
            for kline in &klines {
                let close_time_ms = match kline.get(6) {
                    Some(serde_json::Value::Number(value)) => value.as_i64(),
                    _ => None,
                };
                let Some(close_time_ms) = close_time_ms else {
                    continue;
                };
                if close_time_ms > now_ms {
                    continue;
                }
                let close = match kline.get(4).and_then(|value| value.as_str()) {
                    Some(value) => value.parse::<f64>().ok(),
                    None => None,
                };
                let Some(close) = close else { continue };
                closes.push(close);
            }
            if closes.len() < config.streak_len + 1 {
                continue;
            }
            let start = closes.len() - (config.streak_len + 1);
            let closes = &closes[start..];

            let mut streak_ok = true;
            let mut change_parts: Vec<String> = Vec::with_capacity(config.streak_len);
            for window in closes.windows(2) {
                let prev = window[0];
                let last = window[1];
                if prev <= 0.0 {
                    streak_ok = false;
                    break;
                }
                let change_pct = (last - prev) / prev * 100.0;
                if change_pct < config.change_threshold_pct {
                    streak_ok = false;
                    break;
                }
                change_parts.push(format!("{:.2}%", change_pct));
            }

            if streak_ok {
                let base = symbol.strip_suffix("USDT").unwrap_or(&symbol);
                let trade_url = format!("https://www.binance.com/de/trade/{}_USDT", base);
                let pair = format!("{}/USDT", base);
                let changes = change_parts.join(" > ");
                let message = format!(
                    "{}\n*{}* \\- {}\n[{}]({})",
                    escape_markdown_v2(&request_ts),
                    escape_markdown_v2(&pair),
                    escape_markdown_v2(&changes),
                    escape_markdown_v2("Trade"),
                    escape_markdown_url(&trade_url)
                );
                println!("{}", message);
                send_telegram_message(
                    &client,
                    &config.telegram_token,
                    &config.telegram_chat_id,
                    &message,
                )?;
            }
        }
    }
}
