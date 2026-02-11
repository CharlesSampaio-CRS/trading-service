# Trading Service

A Rust-based trading service that integrates with MongoDB for data persistence and uses the Python CCXT library for cryptocurrency exchange interactions.

## Features

- **MongoDB Integration**: Persistent storage for trading data using the official MongoDB driver
- **CCXT Support**: Access to 100+ cryptocurrency exchanges via the Python CCXT library
- **Async/Await**: Built on Tokio for high-performance async operations
- **Type Safety**: Leverages Rust's type system for reliable code
- **Python Interop**: Seamless integration with Python libraries via PyO3

## Prerequisites

- **Rust**: Install from [rustup.rs](https://rustup.rs/) (version 1.70 or later)
- **Python**: Python 3.8 or later
- **MongoDB**: Running MongoDB instance (local or remote)

## Setup

### 1. Install Python Dependencies

```bash
pip install -r requirements.txt
```

Or install CCXT directly:

```bash
pip install ccxt
```

### 2. Configure Environment Variables

Copy the example environment file:

```bash
cp .env.example .env
```

Edit `.env` with your configuration:

```env
# MongoDB Configuration
MONGODB_URI=mongodb://localhost:27017
MONGODB_DATABASE=trading_service

# CCXT Exchange Configuration
CCXT_EXCHANGE=binance
CCXT_API_KEY=your_api_key_here
CCXT_API_SECRET=your_api_secret_here

# Logging
LOG_LEVEL=info
```

### 3. Build the Project

```bash
cargo build --release
```

## Usage

### Run the Service

```bash
cargo run
```

Or with logging:

```bash
RUST_LOG=info cargo run
```

### Using as a Library

Add to your `Cargo.toml`:

```toml
[dependencies]
trading-service = { path = "../trading-service" }
```

Example usage:

```rust
use trading_service::{MongoDBClient, CCXTClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Connect to MongoDB
    let mongo = MongoDBClient::new().await?;
    
    // Initialize CCXT client
    let ccxt = CCXTClient::new()?;
    
    // Fetch ticker data
    let ticker = ccxt.fetch_ticker("BTC/USDT")?;
    println!("BTC/USDT Last Price: ${:?}", ticker.last);
    
    Ok(())
}
```

## API Overview

### MongoDB Client

```rust
// Create connection
let client = MongoDBClient::new().await?;

// Get database reference
let db = client.database();

// Health check
let is_healthy = client.health_check().await?;
```

### CCXT Client

```rust
// Create client
let client = CCXTClient::new()?;

// Fetch ticker
let ticker = client.fetch_ticker("BTC/USDT")?;

// Get markets
let markets = client.get_markets()?;

// Fetch balance (requires API credentials)
let balances = client.fetch_balance()?;
```

## Testing

Run tests:

```bash
cargo test
```

Run tests including integration tests (requires MongoDB and Python/CCXT):

```bash
cargo test -- --ignored
```

## Project Structure

```
trading-service/
├── Cargo.toml              # Rust dependencies
├── requirements.txt         # Python dependencies
├── .env.example            # Example configuration
├── src/
│   ├── lib.rs             # Library exports
│   ├── main.rs            # Main executable
│   ├── mongodb.rs         # MongoDB client implementation
│   └── ccxt.rs            # CCXT Python wrapper
```

## Architecture

The service is built with a modular architecture:

1. **MongoDB Module** (`src/mongodb.rs`): Handles database connections and operations
2. **CCXT Module** (`src/ccxt.rs`): Wraps Python CCXT library using PyO3 for exchange interactions
3. **Main Service** (`src/main.rs`): Orchestrates the components and provides examples

## Dependencies

### Rust Dependencies

- `tokio`: Async runtime
- `mongodb`: Official MongoDB driver
- `pyo3`: Python interoperability
- `serde`: Serialization/deserialization
- `anyhow`: Error handling
- `log` & `env_logger`: Logging
- `dotenv`: Environment variable management

### Python Dependencies

- `ccxt`: Cryptocurrency exchange trading library

## Development

### Adding New Exchange Operations

To add new CCXT operations, extend the `CCXTClient` in `src/ccxt.rs`:

```rust
impl CCXTClient {
    pub fn your_new_method(&self) -> Result<YourType> {
        Python::with_gil(|py| {
            // Your Python/CCXT integration code
        })
    }
}
```

### MongoDB Collections

Access MongoDB collections through the client:

```rust
let db = mongo_client.database();
let collection = db.collection::<YourType>("collection_name");
```

## License

This project is provided as-is for trading and educational purposes.

## Contributing

Contributions are welcome! Please ensure:

1. Code passes `cargo test`
2. Code is formatted with `cargo fmt`
3. Code passes `cargo clippy`
