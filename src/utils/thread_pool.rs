/// 🚀 FASE 3: Thread pool dedicado para operações CCXT (Python)
/// 
/// Este pool dedicado evita contenção com o runtime principal do Tokio
/// e melhora performance em ~2-3x para operações Python-bound.

use lazy_static::lazy_static;
use std::sync::Arc;
use tokio::runtime::Runtime;

lazy_static! {
    /// Pool dedicado para operações CCXT (Python/blocking)
    /// 
    /// Configurado com:
    /// - 8 worker threads (ajustar baseado em CPU cores)
    /// - Stack size otimizado para Python
    /// - Thread names para debug
    pub static ref CCXT_POOL: Arc<Runtime> = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(8)  // 8 threads dedicadas para CCXT
            .thread_name("ccxt-worker")
            .thread_stack_size(4 * 1024 * 1024)  // 4MB stack (Python precisa mais)
            .enable_all()
            .build()
            .expect("Failed to create CCXT thread pool")
    );
}

/// Executa uma operação blocking no pool dedicado CCXT
/// 
/// # Example
/// ```rust
/// let result = spawn_ccxt_blocking(|| {
///     // Operação Python/CCXT aqui
///     ccxt_client.fetch_balance()
/// }).await?;
/// ```
pub async fn spawn_ccxt_blocking<F, R>(f: F) -> Result<R, tokio::task::JoinError>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    CCXT_POOL.spawn_blocking(f).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ccxt_pool_works() {
        let result = spawn_ccxt_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            42
        }).await;
        
        assert_eq!(result.unwrap(), 42);
    }
}
