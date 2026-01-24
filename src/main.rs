use serde::Deserialize;
use std::time::Duration;

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

struct Config {
    interval_secs: u64,
    change_threshold_pct: f64,
    candle_interval: String,
    streak_len: usize,
}

fn parse_config() -> Result<Config, Box<dyn std::error::Error>> {
    let mut interval_secs: u64 = 30;
    let mut change_threshold_pct: f64 = 1.0;
    let mut candle_interval = String::from("1m");
    let mut streak_len: usize = 3;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--interval-secs" => {
                let value = args.next().ok_or("missing value for --interval-secs")?;
                interval_secs = value.parse()?;
            }
            "--change-pct" => {
                let value = args.next().ok_or("missing value for --change-pct")?;
                change_threshold_pct = value.parse()?;
            }
            "--candle-interval" => {
                let value = args.next().ok_or("missing value for --candle-interval")?;
                candle_interval = value;
            }
            "--streak-len" => {
                let value = args.next().ok_or("missing value for --streak-len")?;
                streak_len = value.parse()?;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: crypto_tracker [--interval-secs N] [--change-pct P] [--candle-interval I] [--streak-len N]"
                );
                println!(
                    "Defaults: --interval-secs 30, --change-pct 1.0, --candle-interval 1m, --streak-len 3"
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {}", arg).into()),
        }
    }

    Ok(Config {
        interval_secs,
        change_threshold_pct,
        candle_interval,
        streak_len,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_config()?;
    let client = reqwest::blocking::Client::new();

    loop {
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
            let limit = config.streak_len + 1;
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
                let close = match kline.get(4).and_then(|value| value.as_str()) {
                    Some(value) => value.parse::<f64>().ok(),
                    None => None,
                };
                let Some(close) = close else { continue };
                closes.push(close);
            }
            if closes.len() < limit {
                continue;
            }

            let mut streak_ok = true;
            let mut last_change_pct = 0.0;
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
                last_change_pct = change_pct;
                change_parts.push(format!("{:.4}%", change_pct));
            }

            if streak_ok {
                let last_close = closes[closes.len() - 1];
                let close_parts: Vec<String> = closes.iter().map(|v| format!("{:.8}", v)).collect();
                let base = symbol.strip_suffix("USDT").unwrap_or(&symbol);
                let trade_url = format!("https://www.binance.com/de/trade/{}_USDT", base);
                println!(
                    "{} change_{}x{}={:.4}% close={} closes=[{}] changes=[{}] {}",
                    symbol,
                    config.candle_interval,
                    config.streak_len,
                    last_change_pct,
                    last_close,
                    close_parts.join(","),
                    change_parts.join(","),
                    trade_url
                );
            }
        }

        std::thread::sleep(Duration::from_secs(config.interval_secs));
    }
}
