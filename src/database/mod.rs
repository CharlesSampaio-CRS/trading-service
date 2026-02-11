use mongodb::{Client, Collection, Database};
use std::error::Error;

#[derive(Clone)]
pub struct MongoDB {
    client: Client,
    db: Database,
}

impl MongoDB {
    pub async fn new(uri: &str) -> Result<Self, Box<dyn Error>> {
        // 🚀 FASE 3: Otimizar connection pooling
        let mut client_options = mongodb::options::ClientOptions::parse(uri).await?;
        
        // Connection pool otimizado
        client_options.max_pool_size = Some(20);  // Max 20 conexões simultâneas
        client_options.min_pool_size = Some(5);   // Mantém 5 conexões sempre vivas
        client_options.max_idle_time = Some(std::time::Duration::from_secs(300));  // 5min idle
        
        // Timeouts otimizados
        client_options.connect_timeout = Some(std::time::Duration::from_secs(5));
        client_options.server_selection_timeout = Some(std::time::Duration::from_secs(5));
        
        let client = Client::with_options(client_options)?;
        
        // Extract database name from URI or use default
        let db_name = uri
            .split('/')
            .last()
            .and_then(|s| s.split('?').next())
            .unwrap_or("MultExchange");
        
        let db = client.database(db_name);
        
        // Test connection
        db.list_collection_names().await?;
        
        let mongodb = Self { client, db };
        
        // 🚀 Create indexes for performance
        mongodb.ensure_indexes().await?;
        
        Ok(mongodb)
    }
    
    /// Creates necessary indexes for optimal query performance
    async fn ensure_indexes(&self) -> Result<(), Box<dyn Error>> {
        use mongodb::bson::doc;
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;
        
        log::info!("🔧 Creating database indexes...");
        
        // Index for exchanges: (user_id) - for fast user exchange queries
        let exchanges = self.database().collection::<mongodb::bson::Document>("exchanges");
        
        let exchange_index = IndexModel::builder()
            .keys(doc! { "user_id": 1 })
            .build();
        
        match exchanges.create_index(exchange_index).await {
            Ok(_) => log::info!("   ✅ Index created: exchanges(user_id)"),
            Err(e) => log::debug!("   ℹ️  Index already exists: {}", e),
        }
        
        // Index for tokens_exchanges: (exchange_ccxt_id) - for fast token queries
        let tokens_exchanges = self.database().collection::<mongodb::bson::Document>("tokens_exchanges");
        
        let tokens_index = IndexModel::builder()
            .keys(doc! { "exchange_ccxt_id": 1 })
            .build();
        
        match tokens_exchanges.create_index(tokens_index).await {
            Ok(_) => log::info!("   ✅ Index created: tokens_exchanges(exchange_ccxt_id)"),
            Err(e) => log::debug!("   ℹ️  Index already exists: {}", e),
        }
        
        log::info!("✅ Database indexes ready");
        
        Ok(())
    }
    
    pub fn collection<T: Send + Sync>(&self, name: &str) -> Collection<T> {
        self.db.collection(name)
    }
    
    pub fn database(&self) -> &Database {
        &self.db
    }
    
    pub fn client(&self) -> &Client {
        &self.client
    }
}
