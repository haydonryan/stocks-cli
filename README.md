# stock-cli

`stock-cli` is a simple command-line tool for viewing stock prices and managing a local watchlist.

## Features

- Fetch live stock data (`get`)
- Persist a watchlist in `~/.config/stocks-cli/stocks.yml`
- Add and delete symbols from the watchlist
- Default command shows your watchlist quotes

## Commands

- `stock-cli get AVGO`
- `stock-cli add avgo`
- `stock-cli del avgo`
- `stock-cli` (no args) → fetches and displays all watchlist stocks

## Output

Values are shown as:

`TICKER  NAME  OPEN  PREV CLOSE  CURRENT  CHANGE`

## Build

- `cargo build`
- `cargo run -- <command>`

## Data source

- Market data is fetched from Yahoo Finance chart endpoint.
