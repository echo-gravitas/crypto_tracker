mod binance;
mod config;
mod telegram;
mod time_utils;

use binance::{SymbolInfo, fetch_exchange_info, fetch_klines, fetch_quote_volumes_24h};
use chrono::Local;
use config::{parse_config, save_config_file, try_reload_config};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use telegram::{escape_markdown_v2, send_telegram_message};
use time_utils::{format_timestamp_de, format_timestamp_short, sleep_until_interval_boundary};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut config, save_config) = parse_config()?;
    if save_config {
        let path = save_config_file(&config)?;
        println!("Saved config to {}", path.display());
    }
    if config.interval_secs % 60 != 0 {
        return Err("--interval-secs must be a multiple of 60 for full-minute polling".into());
    }
    let client = reqwest::blocking::Client::new();

    loop {
        println!("{}: Loop start", format_timestamp_short(Local::now()));
        if let Ok(Some(updated)) = try_reload_config() {
            config = updated;
        }
        sleep_until_interval_boundary(config.interval_secs);
        if config.close_delay_secs > 0 {
            std::thread::sleep(Duration::from_secs(config.close_delay_secs));
        }
        let request_time = Local::now();
        let request_ts = format_timestamp_de(request_time);
        println!(
            "{}: Exchange Info Requested",
            format_timestamp_short(request_time)
        );
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;
        let info = fetch_exchange_info(&client)?;
        println!(
            "{}: 24h Volume for all symbols requested",
            format_timestamp_short(Local::now())
        );
        let volumes = fetch_quote_volumes_24h(&client)?;

        let mut tradable: Vec<&SymbolInfo> = info
            .symbols
            .iter()
            .filter(|symbol| symbol.status == "TRADING" && symbol.quote_asset == "USDT")
            .filter(|symbol| {
                volumes
                    .get(&symbol.symbol)
                    .is_some_and(|v| *v >= config.min_quote_volume_24h)
            })
            .collect();

        tradable.sort_by(|a, b| a.symbol.cmp(&b.symbol));

        let symbols: Vec<String> = tradable.iter().map(|s| s.symbol.clone()).collect();
        println!(
            "{}: Pairs above {} 24h quote volume: {}",
            format_timestamp_short(Local::now()),
            config.min_quote_volume_24h,
            symbols.len()
        );
        if symbols.is_empty() {
            return Err("no USDT trading symbols found".into());
        }

        if config.streak_len == 0 {
            return Err("--streak-len must be at least 1".into());
        }

        for symbol in symbols {
            println!(
                "{}: 24h Volume for {} requested",
                format_timestamp_short(Local::now()),
                symbol
            );
            println!(
                "{}: Candle/Kline Data for {} requested",
                format_timestamp_short(Local::now()),
                symbol
            );
            let limit = config.streak_len + 2;
            let klines = fetch_klines(
                &client,
                symbol.as_str(),
                config.candle_interval.as_str(),
                limit,
            )?;
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
                let config_info = format!(
                    "Interval: {}s\nThreshold: {:.2}%\nCandle Length: {}",
                    config.interval_secs, config.change_threshold_pct, config.candle_interval
                );
                let message = format!(
                    "{}\n\n*{}*\n{}\n\n{}\n{}",
                    escape_markdown_v2(&request_ts),
                    escape_markdown_v2(&pair),
                    escape_markdown_v2(&changes),
                    escape_markdown_v2(&config_info),
                    escape_markdown_v2(&trade_url)
                );
                send_telegram_message(
                    &client,
                    &config.telegram_token,
                    &config.telegram_chat_id,
                    &message,
                )?;
                println!("Telegram Message sent");
            }
        }
        println!("{}: Loop end", format_timestamp_short(Local::now()));
    }
}
