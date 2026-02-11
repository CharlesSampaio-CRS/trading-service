use anyhow::Result;
use mongodb::{Client, Database};
use std::env;

/// MongoDB connection manager
pub struct MongoDBClient {
    client: Client,
    database: Database,
}

impl MongoDBClient {
    /// Create a new MongoDB client connection
    pub async fn new() -> Result<Self> {
        let uri =
            env::var("MONGODB_URI").unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
        let database_name =
            env::var("MONGODB_DATABASE").unwrap_or_else(|_| "trading_service".to_string());

        log::info!("Connecting to MongoDB at {}", uri);

        let client = Client::with_uri_str(&uri).await?;
        let database = client.database(&database_name);

        // Test the connection
        database.list_collection_names().await?;
        log::info!(
            "Successfully connected to MongoDB database: {}",
            database_name
        );

        Ok(Self { client, database })
    }

    /// Get a reference to the database
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Get a reference to the client
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Check if the connection is healthy
    pub async fn health_check(&self) -> Result<bool> {
        self.database.list_collection_names().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires MongoDB to be running
    async fn test_mongodb_connection() {
        dotenv::dotenv().ok();
        env_logger::init();

        let client = MongoDBClient::new().await;
        assert!(client.is_ok());
    }
}
