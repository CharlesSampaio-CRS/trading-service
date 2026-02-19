// ==================== USER EXCHANGES MANAGEMENT ====================
// Gerenciamento de exchanges conectadas do usuário no MongoDB
// Credenciais são criptografadas com ENCRYPTION_KEY antes de salvar

use crate::{
    database::MongoDB,
    models::{UserExchanges, UserExchangeItem, ExchangeCatalog, DecryptedExchange},
    utils::crypto::{encrypt_fernet_via_python, decrypt_fernet_via_python},
};
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde::{Deserialize, Serialize};
use std::env;
use futures::stream::StreamExt;

// ==================== REQUEST/RESPONSE MODELS ====================

#[derive(Debug, Deserialize)]
pub struct AddExchangeRequest {
    pub exchange_type: String,      // ccxt_id: "mexc", "binance", etc
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AddExchangeResponse {
    pub success: bool,
    pub exchange_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UserExchangeInfo {
    pub exchange_id: String,
    pub exchange_type: String,      // ccxt_id
    pub exchange_name: String,      // nome do catálogo
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_passphrase: Option<bool>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ListExchangesResponse {
    pub success: bool,
    pub exchanges: Vec<UserExchangeInfo>,
    pub count: usize,
}

#[derive(Debug, Deserialize)]
pub struct UpdateExchangeRequest {
    pub is_active: Option<bool>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UpdateExchangeResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeleteExchangeResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ==================== SERVICE FUNCTIONS ====================

/// POST /exchanges - Adiciona nova exchange para o usuário
pub async fn add_user_exchange(
    db: &MongoDB,
    user_id: &str,
    request: AddExchangeRequest,
) -> Result<AddExchangeResponse, String> {
    log::info!("📝 Adding exchange {} for user {}", request.exchange_type, user_id);

    // 1. Buscar exchange no catálogo para validar
    let catalog_collection = db.collection::<ExchangeCatalog>("exchanges");
    let catalog = catalog_collection
        .find_one(doc! { "ccxt_id": &request.exchange_type })
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| format!("Exchange '{}' not found in catalog", request.exchange_type))?;
    
    let catalog_id = catalog._id.ok_or("Exchange catalog has no ID")?;

    // 2. Validar se passphrase é obrigatória
    if catalog.requires_passphrase && request.passphrase.is_none() {
        return Ok(AddExchangeResponse {
            success: false,
            exchange_id: String::new(),
            error: Some("Passphrase is required for this exchange".to_string()),
        });
    }

    // 3. Criptografar credenciais
    let encryption_key = env::var("ENCRYPTION_KEY")
        .map_err(|_| "ENCRYPTION_KEY not found in environment")?;
    
    let api_key_encrypted = encrypt_fernet_via_python(&request.api_key, &encryption_key)
        .map_err(|e| format!("Failed to encrypt API key: {}", e))?;
    
    let api_secret_encrypted = encrypt_fernet_via_python(&request.api_secret, &encryption_key)
        .map_err(|e| format!("Failed to encrypt API secret: {}", e))?;
    
    let passphrase_encrypted = if let Some(passphrase) = &request.passphrase {
        Some(encrypt_fernet_via_python(passphrase, &encryption_key)
            .map_err(|e| format!("Failed to encrypt passphrase: {}", e))?)
    } else {
        None
    };

    // 4. Criar item de exchange
    let now = DateTime::now();
    let new_exchange = UserExchangeItem {
        exchange_id: catalog_id.to_hex(),
        api_key_encrypted,
        api_secret_encrypted,
        passphrase_encrypted,
        is_active: true,
        created_at: Some(now.into()),
        updated_at: Some(now.into()),
        reconnected_at: None,
    };

    // 5. Buscar ou criar documento user_exchanges
    let user_exchanges_collection = db.collection::<UserExchanges>("user_exchanges");
    
    let existing = user_exchanges_collection
        .find_one(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Database error: {}", e))?;

    match existing {
        Some(mut doc) => {
            // Verificar se já existe
            if doc.exchanges.iter().any(|e| e.exchange_id == catalog_id.to_hex()) {
                return Ok(AddExchangeResponse {
                    success: false,
                    exchange_id: String::new(),
                    error: Some("Exchange already connected".to_string()),
                });
            }

            // Adicionar ao array
            doc.exchanges.push(new_exchange);
            doc.updated_at = Some(now.into());

            user_exchanges_collection
                .update_one(
                    doc! { "user_id": user_id },
                    doc! { "$set": { 
                        "exchanges": mongodb::bson::to_bson(&doc.exchanges).map_err(|e| e.to_string())?, 
                        "updated_at": now 
                    } }
                )
                .await
                .map_err(|e| format!("Failed to update document: {}", e))?;;
        }
        None => {
            // Criar novo documento
            let new_doc = UserExchanges {
                id: ObjectId::new(),
                user_id: user_id.to_string(),
                exchanges: vec![new_exchange],
                created_at: Some(now.into()),
                updated_at: Some(now.into()),
            };

            user_exchanges_collection
                .insert_one(new_doc)
                .await
                .map_err(|e| format!("Failed to insert document: {}", e))?;
        }
    }

    log::info!("✅ Exchange {} added successfully for user {}", request.exchange_type, user_id);

    Ok(AddExchangeResponse {
        success: true,
        exchange_id: catalog_id.to_hex(),
        error: None,
    })
}

/// GET /exchanges - Lista exchanges conectadas do usuário (sem credenciais)
pub async fn list_user_exchanges(
    db: &MongoDB,
    user_id: &str,
) -> Result<ListExchangesResponse, String> {
    log::info!("📋 Listing exchanges for user {}", user_id);

    // 1. Buscar exchanges do usuário
    let user_exchanges_collection = db.collection::<UserExchanges>("user_exchanges");
    let user_doc = user_exchanges_collection
        .find_one(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Database error: {}", e))?;

    let exchanges = match user_doc {
        Some(doc) => doc.exchanges,
        None => {
            return Ok(ListExchangesResponse {
                success: true,
                exchanges: vec![],
                count: 0,
            });
        }
    };

    // 2. Buscar info do catálogo
    let catalog_collection = db.collection::<ExchangeCatalog>("exchanges");
    let mut result = Vec::new();

    for ex in exchanges {
        let exchange_oid = ObjectId::parse_str(&ex.exchange_id)
            .map_err(|e| format!("Invalid exchange_id: {}", e))?;
        
        if let Ok(Some(catalog)) = catalog_collection
            .find_one(doc! { "_id": exchange_oid })
            .await
        {
            result.push(UserExchangeInfo {
                exchange_id: ex.exchange_id,
                exchange_type: catalog.ccxt_id,
                exchange_name: catalog.nome.unwrap_or_else(|| "Unknown".to_string()),
                is_active: ex.is_active,
                logo: catalog.logo,
                icon: catalog.icon,
                requires_passphrase: Some(catalog.requires_passphrase),
                created_at: ex.created_at
                    .and_then(|dt| {
                        if let mongodb::bson::Bson::DateTime(dt) = dt {
                            Some(dt.to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "Unknown".to_string()),
            });
        }
    }

    let count = result.len();

    Ok(ListExchangesResponse {
        success: true,
        exchanges: result,
        count,
    })
}

/// PATCH /exchanges/{exchange_id} - Atualiza exchange do usuário
pub async fn update_user_exchange(
    db: &MongoDB,
    user_id: &str,
    exchange_id: &str,
    request: UpdateExchangeRequest,
) -> Result<UpdateExchangeResponse, String> {
    log::info!("🔧 Updating exchange {} for user {}", exchange_id, user_id);

    let user_exchanges_collection = db.collection::<UserExchanges>("user_exchanges");
    
    // Buscar documento
    let mut user_doc = user_exchanges_collection
        .find_one(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or("User has no exchanges")?;

    // Encontrar exchange no array
    let exchange = user_doc.exchanges.iter_mut()
        .find(|e| e.exchange_id == exchange_id)
        .ok_or("Exchange not found")?;

    // Atualizar campos
    if let Some(is_active) = request.is_active {
        exchange.is_active = is_active;
    }

    // Atualizar credenciais se fornecidas
    if request.api_key.is_some() || request.api_secret.is_some() || request.passphrase.is_some() {
        let encryption_key = env::var("ENCRYPTION_KEY")
            .map_err(|_| "ENCRYPTION_KEY not found in environment")?;

        if let Some(api_key) = &request.api_key {
            exchange.api_key_encrypted = encrypt_fernet_via_python(api_key, &encryption_key)
                .map_err(|e| format!("Failed to encrypt API key: {}", e))?;
        }

        if let Some(api_secret) = &request.api_secret {
            exchange.api_secret_encrypted = encrypt_fernet_via_python(api_secret, &encryption_key)
                .map_err(|e| format!("Failed to encrypt API secret: {}", e))?;
        }

        if let Some(passphrase) = &request.passphrase {
            exchange.passphrase_encrypted = Some(encrypt_fernet_via_python(passphrase, &encryption_key)
                .map_err(|e| format!("Failed to encrypt passphrase: {}", e))?);
        }

        exchange.reconnected_at = Some(DateTime::now().into());
    }

    exchange.updated_at = Some(DateTime::now().into());

    // Salvar
    user_exchanges_collection
        .update_one(
            doc! { "user_id": user_id },
            doc! { "$set": { "exchanges": mongodb::bson::to_bson(&user_doc.exchanges).map_err(|e| e.to_string())? } }
        )
        .await
        .map_err(|e| format!("Failed to update: {}", e))?;

    log::info!("✅ Exchange {} updated successfully", exchange_id);

    Ok(UpdateExchangeResponse {
        success: true,
        error: None,
    })
}

/// DELETE /exchanges/{exchange_id} - Remove exchange do usuário
pub async fn delete_user_exchange(
    db: &MongoDB,
    user_id: &str,
    exchange_id: &str,
) -> Result<DeleteExchangeResponse, String> {
    log::info!("🗑️  Deleting exchange {} for user {}", exchange_id, user_id);

    let user_exchanges_collection = db.collection::<UserExchanges>("user_exchanges");
    
    // Remove do array
    let result = user_exchanges_collection
        .update_one(
            doc! { "user_id": user_id },
            doc! { "$pull": { "exchanges": { "exchange_id": exchange_id } } }
        )
        .await
        .map_err(|e| format!("Database error: {}", e))?;

    if result.modified_count == 0 {
        return Ok(DeleteExchangeResponse {
            success: false,
            error: Some("Exchange not found".to_string()),
        });
    }

    log::info!("✅ Exchange {} deleted successfully", exchange_id);

    Ok(DeleteExchangeResponse {
        success: true,
        error: None,
    })
}

/// Busca exchanges do usuário e descriptografa (USO INTERNO - não expor via API)
pub async fn get_user_exchanges_decrypted(
    db: &MongoDB,
    user_id: &str,
) -> Result<Vec<DecryptedExchange>, String> {
    log::debug!("🔓 Fetching and decrypting exchanges for user {}", user_id);

    // 1. Buscar exchanges do usuário
    let user_exchanges_collection = db.collection::<UserExchanges>("user_exchanges");
    let user_doc = user_exchanges_collection
        .find_one(doc! { "user_id": user_id })
        .await
        .map_err(|e| format!("Database error: {}", e))?;

    let exchanges = match user_doc {
        Some(doc) => doc.exchanges,
        None => return Ok(vec![]),
    };

    // Filtrar apenas ativos
    let active_exchanges: Vec<_> = exchanges.into_iter()
        .filter(|e| e.is_active)
        .collect();

    if active_exchanges.is_empty() {
        return Ok(vec![]);
    }

    // 2. Buscar info do catálogo em batch
    let catalog_collection = db.collection::<ExchangeCatalog>("exchanges");
    let exchange_ids: Vec<ObjectId> = active_exchanges
        .iter()
        .filter_map(|ex| ObjectId::parse_str(&ex.exchange_id).ok())
        .collect();

    let mut cursor = catalog_collection
        .find(doc! { "_id": { "$in": exchange_ids } })
        .await
        .map_err(|e| format!("Database error: {}", e))?;

    let mut catalog_map = std::collections::HashMap::new();
    while let Some(catalog) = cursor.next().await {
        if let Ok(catalog) = catalog {
            if let Some(id) = &catalog._id {
                catalog_map.insert(*id, catalog);
            }
        }
    }

    // 3. Descriptografar em paralelo
    let encryption_key = env::var("ENCRYPTION_KEY")
        .map_err(|_| "ENCRYPTION_KEY not found in environment")?;

    let decrypt_tasks: Vec<_> = active_exchanges
        .into_iter()
        .filter_map(|user_exchange| {
            let exchange_oid = ObjectId::parse_str(&user_exchange.exchange_id).ok()?;
            let catalog = catalog_map.get(&exchange_oid)?.clone();
            let key = encryption_key.clone();
            
            Some(tokio::task::spawn_blocking(move || {
                let api_key = decrypt_fernet_via_python(&user_exchange.api_key_encrypted, &key)
                    .unwrap_or_else(|e| {
                        log::error!("Failed to decrypt API key: {}", e);
                        user_exchange.api_key_encrypted.clone()
                    });
                
                let api_secret = decrypt_fernet_via_python(&user_exchange.api_secret_encrypted, &key)
                    .unwrap_or_else(|e| {
                        log::error!("Failed to decrypt API secret: {}", e);
                        user_exchange.api_secret_encrypted.clone()
                    });
                
                let passphrase = user_exchange.passphrase_encrypted.as_ref()
                    .and_then(|p| decrypt_fernet_via_python(p, &key).ok());
                
                DecryptedExchange {
                    exchange_id: user_exchange.exchange_id,
                    ccxt_id: catalog.ccxt_id.clone(),
                    name: catalog.nome.clone().unwrap_or_else(|| "Unknown".to_string()),
                    api_key,
                    api_secret,
                    passphrase,
                    is_active: user_exchange.is_active,
                }
            }))
        })
        .collect();

    let decrypt_results = futures::future::join_all(decrypt_tasks).await;
    
    let mut decrypted_exchanges = Vec::new();
    for result in decrypt_results {
        match result {
            Ok(exchange) => decrypted_exchanges.push(exchange),
            Err(e) => log::error!("Decryption task failed: {}", e),
        }
    }

    Ok(decrypted_exchanges)
}
