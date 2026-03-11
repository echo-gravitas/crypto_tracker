use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::TcpStream;
use std::time::Duration;
use tungstenite::{Message, WebSocket, connect};

#[derive(Deserialize)]
pub struct ExchangeInfo {
    pub symbols: Vec<SymbolInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolInfo {
    pub symbol: String,
    pub status: String,
    pub quote_asset: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ticker24h {
    symbol: String,
    quote_volume: String,
}

pub fn fetch_exchange_info(client: &Client) -> Result<ExchangeInfo, Box<dyn std::error::Error>> {
    let exchange_info_url = "https://api.binance.com/api/v3/exchangeInfo";
    let response = client.get(exchange_info_url).send()?.error_for_status()?;
    let info: ExchangeInfo = response.json()?;
    Ok(info)
}

pub fn fetch_quote_volumes_24h(
    client: &Client,
) -> Result<HashMap<String, f64>, Box<dyn std::error::Error>> {
    let url = "https://api.binance.com/api/v3/ticker/24hr";
    let response = client.get(url).send()?.error_for_status()?;
    let tickers: Vec<Ticker24h> = response.json()?;
    let mut volumes = HashMap::with_capacity(tickers.len());
    for ticker in tickers {
        if let Ok(volume) = ticker.quote_volume.parse::<f64>() {
            volumes.insert(ticker.symbol, volume);
        }
    }
    Ok(volumes)
}

pub fn fetch_klines(
    client: &Client,
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Result<Vec<Vec<serde_json::Value>>, Box<dyn std::error::Error>> {
    let kline_url = "https://api.binance.com/api/v3/klines";
    let limit_str = limit.to_string();
    let response = client
        .get(kline_url)
        .query(&[
            ("symbol", symbol),
            ("interval", interval),
            ("limit", limit_str.as_str()),
        ])
        .send()?
        .error_for_status()?;
    let klines: Vec<Vec<serde_json::Value>> = response.json()?;
    Ok(klines)
}

#[derive(Deserialize)]
pub struct WsKlineEvent {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "k")]
    pub kline: WsKline,
}

#[derive(Deserialize)]
pub struct WsKline {
    #[serde(rename = "x")]
    pub is_closed: bool,
    #[serde(rename = "c")]
    pub close: String,
}

#[derive(Deserialize)]
pub struct WsSubscriptionAck {
    pub id: Option<u64>,
}

pub fn connect_kline_stream(
    symbols: &[String],
    interval: &str,
) -> Result<WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>, Box<dyn std::error::Error>> {
    let (mut socket, _) = connect("wss://stream.binance.com:9443/ws")?;
    let params: Vec<String> = symbols
        .iter()
        .map(|symbol| format!("{}@kline_{}", symbol.to_lowercase(), interval))
        .collect();
    let subscribe = serde_json::json!({
        "method": "SUBSCRIBE",
        "params": params,
        "id": 1
    });
    socket.send(Message::Text(subscribe.to_string()))?;
    Ok(socket)
}

pub fn set_socket_read_timeout(
    socket: &mut WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => {
            stream.set_read_timeout(Some(timeout))?;
        }
        tungstenite::stream::MaybeTlsStream::NativeTls(stream) => {
            stream.get_mut().set_read_timeout(Some(timeout))?;
        }
        _ => {}
    }
    Ok(())
}
