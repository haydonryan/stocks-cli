use clap::Parser;
use clap::Subcommand;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs;
use std::path::PathBuf;

/// A CLI tool to fetch stock prices
#[derive(Parser, Debug)]
#[command(name = "stock-cli")]
#[command(about = "Fetch current stock prices and daily changes", long_about = None)]
struct Args {
    /// Output as JSON
    #[arg(short, long)]
    json: bool,

    /// Output tool description for LLM/MCP integration
    #[arg(long)]
    mcp: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Fetch stock data for one or more tickers, separated by commas
    Get {
        /// Stock tickers separated by commas (e.g., AVGO,AAPL,MSFT)
        tickers: String,
    },
    /// Add a ticker to the local watchlist
    Add {
        /// Stock ticker to add (e.g., avgo)
        symbol: String,
    },
    /// Remove a ticker from the local watchlist
    Del {
        /// Stock ticker to remove (e.g., avgo)
        symbol: String,
    },
}

#[derive(Debug, Deserialize)]
struct ChartResponse {
    chart: Chart,
}

#[derive(Debug, Deserialize)]
struct Chart {
    result: Option<Vec<ChartResult>>,
    error: Option<ChartError>,
}

#[derive(Debug, Deserialize)]
struct ChartError {
    code: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct ChartResult {
    meta: ChartMeta,
    indicators: Option<Indicators>,
}

#[derive(Debug, Deserialize)]
struct Indicators {
    quote: Option<Vec<QuoteIndicator>>,
}

#[derive(Debug, Deserialize)]
struct QuoteIndicator {
    open: Option<Vec<Option<f64>>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartMeta {
    symbol: String,
    short_name: Option<String>,
    regular_market_price: Option<f64>,
    chart_previous_close: Option<f64>,
    previous_close: Option<f64>,
    currency: Option<String>,
}

#[derive(Debug, Serialize)]
struct StockData {
    symbol: String,
    name: String,
    open: Option<f64>,
    previous_close: f64,
    current_price: f64,
    change: f64,
    change_percent: f64,
    currency: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Watchlist {
    symbols: Vec<String>,
}

async fn fetch_single_stock(
    client: &reqwest::Client,
    ticker: &str,
) -> Result<StockData, Box<dyn Error + Send + Sync>> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1d&range=2d",
        ticker
    );

    let response = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("Failed to fetch {}: {}", ticker, response.status()).into());
    }

    let data: ChartResponse = response.json().await?;

    if let Some(error) = data.chart.error {
        return Err(format!("{}: {}", error.code, error.description).into());
    }

    let results = data.chart.result.ok_or("No data returned")?;
    let result = results.first().ok_or("Empty result")?;
    let meta = &result.meta;

    let current_price = meta.regular_market_price.ok_or("No current price")?;
    let previous_close = meta
        .chart_previous_close
        .or(meta.previous_close)
        .ok_or("No previous close")?;

    // Try to get today's open from indicators
    let open = result
        .indicators
        .as_ref()
        .and_then(|i| i.quote.as_ref())
        .and_then(|q| q.last())
        .and_then(|q| q.open.as_ref())
        .and_then(|opens| opens.first().copied().flatten());

    let change = current_price - previous_close;
    let change_percent = (change / previous_close) * 100.0;

    Ok(StockData {
        symbol: meta.symbol.clone(),
        name: meta
            .short_name
            .clone()
            .unwrap_or_else(|| meta.symbol.clone()),
        open,
        previous_close,
        current_price,
        change,
        change_percent,
        currency: meta.currency.clone().unwrap_or_else(|| "USD".to_string()),
    })
}

async fn fetch_stock_data(
    tickers: &[&str],
) -> Vec<Result<StockData, Box<dyn Error + Send + Sync>>> {
    let client = reqwest::Client::new();

    let futures: Vec<_> = tickers
        .iter()
        .map(|ticker| {
            let client = client.clone();
            let ticker = ticker.to_string();
            async move { fetch_single_stock(&client, &ticker).await }
        })
        .collect();

    futures::future::join_all(futures).await
}

fn format_price(price: Option<f64>, currency: &str) -> String {
    match price {
        Some(p) => format!("{:.2} {}", p, currency),
        None => "N/A".to_string(),
    }
}

fn format_change(change: f64, percent: f64) -> String {
    let sign = if change >= 0.0 { "+" } else { "" };
    let text = format!("{}{:.2} ({}{:.2}%)", sign, change, sign, percent);
    // Pad before applying color, since ANSI codes break width calculations
    let padded = format!("{:>20}", text);
    if change >= 0.0 {
        padded.green().to_string()
    } else {
        padded.red().to_string()
    }
}

fn normalize_ticker(symbol: &str) -> String {
    symbol.trim().to_uppercase()
}

fn watchlist_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("stocks-cli")
        .join("stocks.yml")
}

fn load_watchlist() -> Result<Watchlist, Box<dyn Error + Send + Sync>> {
    let path = watchlist_path();
    if !path.exists() {
        return Ok(Watchlist {
            symbols: Vec::new(),
        });
    }

    let content = fs::read_to_string(&path)?;
    if content.trim().is_empty() {
        return Ok(Watchlist {
            symbols: Vec::new(),
        });
    }

    let watchlist: Watchlist = serde_yaml::from_str(&content)?;
    Ok(watchlist)
}

fn save_watchlist(watchlist: &Watchlist) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = watchlist_path();
    let parent = path.parent().ok_or("Invalid watchlist path")?;
    fs::create_dir_all(parent)?;
    let content = serde_yaml::to_string(watchlist)?;
    fs::write(&path, content)?;
    Ok(())
}

fn add_to_watchlist(symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
    let normalized = normalize_ticker(symbol);
    let mut watchlist = load_watchlist()?;
    if watchlist
        .symbols
        .iter()
        .any(|entry| normalize_ticker(entry) == normalized)
    {
        return Ok(false);
    }

    watchlist.symbols.push(normalized);
    save_watchlist(&watchlist)?;
    Ok(true)
}

fn remove_from_watchlist(symbol: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
    let normalized = normalize_ticker(symbol);
    let mut watchlist = load_watchlist()?;
    let before = watchlist.symbols.len();
    watchlist
        .symbols
        .retain(|entry| normalize_ticker(entry) != normalized);
    if watchlist.symbols.len() == before {
        return Ok(false);
    }

    watchlist.symbols.sort();
    save_watchlist(&watchlist)?;
    Ok(true)
}

fn print_mcp_description() {
    let mcp = serde_json::json!({
        "name": "stock-cli",
        "description": "A command-line tool that fetches real-time stock market data. Given one or more stock ticker symbols, it returns the current price, opening price, previous closing price, and the daily price change (both absolute and percentage). Use this tool when you need current stock prices or want to check how a stock is performing today compared to yesterday.",
        "arguments": {
            "tickers": {
                "type": "string",
                "description": "One or more stock ticker symbols separated by commas. Examples: 'AAPL' for Apple, 'MSFT,GOOGL,AMZN' for multiple stocks. Use standard NYSE/NASDAQ ticker symbols.",
                "required": true
            },
            "--json": {
                "type": "flag",
                "description": "Output results as JSON instead of a formatted table. Recommended for programmatic parsing.",
                "required": false
            }
        },
        "output": {
            "table_format": "Human-readable table with columns: TICKER, NAME, OPEN, PREV CLOSE, CURRENT, CHANGE",
            "json_format": {
                "stocks": [
                    {
                        "symbol": "string - Ticker symbol",
                        "name": "string - Company name",
                        "open": "number|null - Today's opening price",
                        "previous_close": "number - Yesterday's closing price",
                        "current_price": "number - Current/latest price",
                        "change": "number - Price change from previous close",
                        "change_percent": "number - Percentage change from previous close",
                        "currency": "string - Currency code (e.g., USD)"
                    }
                ],
                "errors": ["string - Error messages for failed ticker lookups"]
            }
        },
        "examples": [
            {
                "description": "Get current Apple stock price",
                "command": "stock-cli AAPL"
            },
            {
                "description": "Get multiple stocks as JSON",
                "command": "stock-cli --json AAPL,MSFT,GOOGL"
            },
            {
                "description": "Check tech stocks performance",
                "command": "stock-cli NVDA,AMD,INTC"
            }
        ],
        "notes": [
            "Data is sourced from Yahoo Finance and is near real-time during market hours",
            "Use --json flag when you need to parse the output programmatically",
            "Invalid tickers will be reported in the errors array (JSON) or stderr (table)",
            "Prices are displayed in the stock's native currency"
        ]
    });
    println!("{}", serde_json::to_string_pretty(&mcp).unwrap());
}

fn print_stock_table(stocks: &[StockData]) {
    println!(
        "{:<8} {:<32} {:>10} {:>14} {:>14} {:>20}",
        "TICKER", "NAME", "OPEN", "PREV CLOSE", "CURRENT", "CHANGE"
    );
    println!("{}", "-".repeat(102));

    for stock in stocks {
        let name: String = stock.name.chars().take(32).collect();

        println!(
            "{:<8} {:<32} {:>10} {:>14} {:>14} {}",
            stock.symbol,
            name,
            format_price(stock.open, &stock.currency),
            format!("{:.2} {}", stock.previous_close, stock.currency),
            format!("{:.2} {}", stock.current_price, stock.currency),
            format_change(stock.change, stock.change_percent),
        );
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Handle --mcp flag first
    if args.mcp {
        print_mcp_description();
        return;
    }

    let command = args.command.unwrap_or(Commands::Get {
        tickers: String::new(),
    });

    match command {
        Commands::Get { tickers } => {
            if tickers.trim().is_empty() {
                let watchlist = load_watchlist().unwrap_or_else(|e| {
                    eprintln!("Failed to load watchlist: {}", e);
                    std::process::exit(1);
                });
                if watchlist.symbols.is_empty() {
                    println!("Watchlist is empty.");
                    return;
                }

                let tickers: Vec<String> = watchlist
                    .symbols
                    .into_iter()
                    .map(|entry| normalize_ticker(&entry))
                    .collect();
                if tickers.is_empty() {
                    println!("Watchlist is empty.");
                    return;
                }

                let ticker_refs: Vec<&str> = tickers.iter().map(|s: &String| s.as_str()).collect();
                let results = fetch_stock_data(&ticker_refs).await;
                let mut successful: Vec<StockData> = Vec::new();
                let mut failed: Vec<String> = Vec::new();

                for (i, result) in results.into_iter().enumerate() {
                    match result {
                        Ok(stock) => successful.push(stock),
                        Err(e) => failed.push(format!("{}: {}", tickers[i], e)),
                    }
                }

                if args.json {
                    let output = serde_json::json!({
                        "stocks": successful,
                        "errors": failed,
                    });
                    println!("{}", serde_json::to_string_pretty(&output).unwrap());
                } else {
                    if !successful.is_empty() {
                        print_stock_table(&successful);
                    }

                    if !failed.is_empty() {
                        eprintln!("Failed to fetch some tickers:");
                        for error in &failed {
                            eprintln!("  - {}", error);
                        }
                    }
                }

                if successful.is_empty() {
                    std::process::exit(1);
                }
                return;
            }

            let tickers: Vec<String> = tickers
                .split(',')
                .map(normalize_ticker)
                .filter(|t| !t.is_empty())
                .collect();

            if tickers.is_empty() {
                eprintln!("Error: No valid tickers provided");
                std::process::exit(1);
            }

            let ticker_refs: Vec<&str> = tickers.iter().map(|s| s.as_str()).collect();
            if !args.json {
                println!("Fetching stock data for: {}", tickers.join(", "));
            }

            let results = fetch_stock_data(&ticker_refs).await;

            let mut successful: Vec<StockData> = Vec::new();
            let mut failed: Vec<String> = Vec::new();

            for (i, result) in results.into_iter().enumerate() {
                match result {
                    Ok(stock) => successful.push(stock),
                    Err(e) => failed.push(format!("{}: {}", tickers[i], e)),
                }
            }

            if args.json {
                let output = serde_json::json!({
                    "stocks": successful,
                    "errors": failed,
                });
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            } else {
                if !successful.is_empty() {
                    print_stock_table(&successful);
                }

                if !failed.is_empty() {
                    eprintln!("Failed to fetch some tickers:");
                    for error in &failed {
                        eprintln!("  - {}", error);
                    }
                }
            }

            if successful.is_empty() {
                std::process::exit(1);
            }
        }
        Commands::Add { symbol } => match add_to_watchlist(&symbol) {
            Ok(added) => {
                if added {
                    println!("Added {} to watchlist.", normalize_ticker(&symbol));
                } else {
                    println!(
                        "{} is already in your watchlist.",
                        normalize_ticker(&symbol)
                    );
                }
            }
            Err(e) => {
                eprintln!("Failed to update watchlist: {}", e);
                std::process::exit(1);
            }
        },
        Commands::Del { symbol } => match remove_from_watchlist(&symbol) {
            Ok(removed) => {
                if removed {
                    println!("Removed {} from watchlist.", normalize_ticker(&symbol));
                } else {
                    println!("{} is not in your watchlist.", normalize_ticker(&symbol));
                }
            }
            Err(e) => {
                eprintln!("Failed to update watchlist: {}", e);
                std::process::exit(1);
            }
        },
    }
}
