mod binance;
mod config;
mod telegram;
mod time_utils;

use binance::{
    SymbolInfo, WsKlineEvent, WsSubscriptionAck, connect_kline_stream, fetch_exchange_info,
    fetch_klines, fetch_quote_volumes_24h, set_socket_read_timeout,
};
use chrono::Local;
use config::{parse_config, save_config_file, try_reload_config};
use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use telegram::{escape_markdown_v2, send_telegram_message};
use time_utils::{format_timestamp_de, format_timestamp_short};
use tungstenite::Message;

const WS_BATCH_SIZE: usize = 100;

fn candle_interval_secs(interval: &str) -> Result<u64, Box<dyn std::error::Error>> {
    if interval.len() < 2 {
        return Err(format!("invalid candle interval: {}", interval).into());
    }

    let (value, unit) = interval.split_at(interval.len() - 1);
    let value: u64 = value.parse()?;
    let seconds = match unit {
        "s" => value,
        "m" => value * 60,
        "h" => value * 60 * 60,
        "d" => value * 60 * 60 * 24,
        "w" => value * 60 * 60 * 24 * 7,
        _ => return Err(format!("unsupported candle interval: {}", interval).into()),
    };
    Ok(seconds)
}

fn log_config(config: &config::Config, source: &str) {
    println!(
        "{}: Loaded config from {} -> min_quote_volume_24h={}, interval_secs={}, change_threshold_pct={}, candle_interval={}, streak_len={}",
        format_timestamp_short(Local::now()),
        source,
        config.min_quote_volume_24h,
        config.interval_secs,
        config.change_threshold_pct,
        config.candle_interval,
        config.streak_len
    );
}

fn load_symbols(
    client: &reqwest::blocking::Client,
    min_quote_volume_24h: f64,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let info = fetch_exchange_info(client)?;
    let volumes = fetch_quote_volumes_24h(client)?;

    let mut tradable: Vec<&SymbolInfo> = info
        .symbols
        .iter()
        .filter(|symbol| symbol.status == "TRADING" && symbol.quote_asset == "USDT")
        .filter(|symbol| {
            volumes
                .get(&symbol.symbol)
                .is_some_and(|v| *v >= min_quote_volume_24h)
        })
        .collect();

    tradable.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    Ok(tradable.into_iter().map(|s| s.symbol.clone()).collect())
}

fn seed_closes(
    client: &reqwest::blocking::Client,
    symbols: &[String],
    candle_interval: &str,
    required_closed_klines: usize,
) -> Result<HashMap<String, VecDeque<f64>>, Box<dyn std::error::Error>> {
    let mut state = HashMap::with_capacity(symbols.len());

    for symbol in symbols {
        let limit = required_closed_klines + 1;
        let klines = fetch_klines(client, symbol, candle_interval, limit)?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;
        let mut closes = VecDeque::with_capacity(required_closed_klines);

        for kline in klines {
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
            let Some(close) = close else {
                continue;
            };
            if closes.len() == required_closed_klines {
                closes.pop_front();
            }
            closes.push_back(close);
        }

        println!(
            "{}: Seeded {} closed klines for {}",
            format_timestamp_short(Local::now()),
            closes.len(),
            symbol
        );
        state.insert(symbol.clone(), closes);
    }

    Ok(state)
}

fn find_matching_window(closes: &VecDeque<f64>, streak_len: usize, threshold: f64) -> Option<Vec<String>> {
    let values: Vec<f64> = closes.iter().copied().collect();
    for change_window in values.windows(streak_len + 1) {
        let mut change_parts: Vec<String> = Vec::with_capacity(streak_len);
        let mut window_ok = true;

        for pair in change_window.windows(2) {
            let prev = pair[0];
            let last = pair[1];
            if prev <= 0.0 {
                window_ok = false;
                break;
            }
            let change_pct = (last - prev) / prev * 100.0;
            if change_pct < threshold {
                window_ok = false;
                break;
            }
            change_parts.push(format!("{:.2}%", change_pct));
        }

        if window_ok {
            return Some(change_parts);
        }
    }

    None
}

fn send_momentum_alert(
    client: &reqwest::blocking::Client,
    config: &config::Config,
    symbol: &str,
    change_parts: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let request_ts = format_timestamp_de(Local::now());
    let base = symbol.strip_suffix("USDT").unwrap_or(symbol);
    let trade_url = format!("https://www.binance.com/de/trade/{}_USDT", base);
    let pair = format!("{}/USDT", base);
    let changes = change_parts.join(" > ");
    let config_info = format!(
        "Refresh: {}s\nThreshold: {:.2}%\nCandle Length: {}",
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
        client,
        &config.telegram_token,
        &config.telegram_chat_id,
        &message,
    )?;
    println!(
        "{}: Telegram message sent for {}",
        format_timestamp_short(Local::now()),
        symbol
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut config, save_config) = parse_config()?;
    log_config(&config, "startup");
    if save_config {
        let path = save_config_file(&config)?;
        println!("Saved config to {}", path.display());
    }
    let client = reqwest::blocking::Client::new();
    let mut alert_active_by_symbol: HashMap<String, bool> = HashMap::new();

    loop {
        if let Ok(Some(updated)) = try_reload_config() {
            config = updated;
            log_config(&config, "reload");
        }
        println!("{}: Refreshing symbol universe", format_timestamp_short(Local::now()));
        let symbols = load_symbols(&client, config.min_quote_volume_24h)?;
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

        let candle_secs = candle_interval_secs(&config.candle_interval)?;
        let lookback_candles = config.interval_secs.div_ceil(candle_secs) as usize;
        let required_closed_klines = lookback_candles.max(config.streak_len) + 1;
        let mut closes_by_symbol = seed_closes(
            &client,
            &symbols,
            config.candle_interval.as_str(),
            required_closed_klines,
        )?;
        for symbol in &symbols {
            let is_match = closes_by_symbol
                .get(symbol)
                .and_then(|closes| {
                    find_matching_window(closes, config.streak_len, config.change_threshold_pct)
                });
            let was_active = alert_active_by_symbol.get(symbol).copied().unwrap_or(false);

            match is_match {
                Some(change_parts) => {
                    if !was_active {
                        println!(
                            "{}: {} matched immediately after seeding",
                            format_timestamp_short(Local::now()),
                            symbol
                        );
                        send_momentum_alert(&client, &config, symbol, &change_parts)?;
                    }
                    alert_active_by_symbol.insert(symbol.clone(), true);
                }
                None => {
                    alert_active_by_symbol.insert(symbol.clone(), false);
                }
            }
        }
        alert_active_by_symbol.retain(|symbol, _| symbols.contains(symbol));
        let mut sockets = Vec::new();
        for batch in symbols.chunks(WS_BATCH_SIZE) {
            let mut socket = connect_kline_stream(batch, config.candle_interval.as_str())?;
            set_socket_read_timeout(&mut socket, Duration::from_millis(250))?;
            println!(
                "{}: WebSocket connected for batch with {} symbols",
                format_timestamp_short(Local::now()),
                batch.len()
            );
            sockets.push(socket);
        }
        let refresh_at = Instant::now() + Duration::from_secs(config.interval_secs.max(1));

        println!(
            "{}: WebSocket connected for {} symbols across {} batches",
            format_timestamp_short(Local::now()),
            symbols.len(),
            sockets.len()
        );

        while Instant::now() < refresh_at {
            for socket in &mut sockets {
                let message = match socket.read() {
                    Ok(message) => message,
                    Err(tungstenite::Error::Io(err))
                        if err.kind() == ErrorKind::WouldBlock
                            || err.kind() == ErrorKind::TimedOut =>
                    {
                        continue;
                    }
                    Err(err) => return Err(err.into()),
                };
                let Message::Text(text) = message else {
                    continue;
                };

                if text.contains("\"result\"") {
                    if let Ok(ack) = serde_json::from_str::<WsSubscriptionAck>(&text) {
                        println!(
                            "{}: WebSocket subscription acknowledged (id={})",
                            format_timestamp_short(Local::now()),
                            ack.id.unwrap_or_default()
                        );
                        continue;
                    }
                }

                let event: WsKlineEvent = match serde_json::from_str(&text) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if !event.kline.is_closed {
                    continue;
                }

                let close = match event.kline.close.parse::<f64>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                let symbol = event.symbol;
                let closes = closes_by_symbol
                    .entry(symbol.clone())
                    .or_insert_with(|| VecDeque::with_capacity(required_closed_klines));
                if closes.len() == required_closed_klines {
                    closes.pop_front();
                }
                closes.push_back(close);

                println!(
                    "{}: {} received closed kline (have {}, need {})",
                    format_timestamp_short(Local::now()),
                    symbol,
                    closes.len(),
                    required_closed_klines
                );

                if closes.len() < required_closed_klines {
                    println!(
                        "{}: {} insufficient closed klines (have {}, need {})",
                        format_timestamp_short(Local::now()),
                        symbol,
                        closes.len(),
                        required_closed_klines
                    );
                    continue;
                }

                let is_match =
                    find_matching_window(closes, config.streak_len, config.change_threshold_pct);
                let was_active = alert_active_by_symbol.get(&symbol).copied().unwrap_or(false);
                match is_match {
                    Some(change_parts) => {
                        if !was_active {
                            send_momentum_alert(&client, &config, &symbol, &change_parts)?;
                        }
                        alert_active_by_symbol.insert(symbol, true);
                    }
                    None => {
                        alert_active_by_symbol.insert(symbol, false);
                    }
                }
            }
        }

        println!(
            "{}: Refresh window ended, reconnecting WebSocket",
            format_timestamp_short(Local::now())
        );
    }
}
