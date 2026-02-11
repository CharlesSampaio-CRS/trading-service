// Cache utilities - to be implemented
use std::collections::HashMap;
use std::sync::RwLock;

lazy_static::lazy_static! {
    static ref CACHE: RwLock<HashMap<String, String>> = RwLock::new(HashMap::new());
}

pub fn get_cached(key: &str) -> Option<String> {
    CACHE.read().ok()?.get(key).cloned()
}

pub fn set_cache(key: String, value: String) {
    if let Ok(mut cache) = CACHE.write() {
        cache.insert(key, value);
    }
}
