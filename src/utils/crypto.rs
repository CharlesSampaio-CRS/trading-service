use pyo3::prelude::*;

/// Descriptografa uma string usando Fernet via Python
pub fn decrypt_fernet_via_python(encrypted_data: &str, key: &str) -> Result<String, String> {
    Python::with_gil(|py| {
        // Import cryptography.fernet
        let fernet_module = py
            .import("cryptography.fernet")
            .map_err(|e| format!("Failed to import cryptography.fernet: {}", e))?;
        
        // Create Fernet instance
        let fernet_class = fernet_module
            .getattr("Fernet")
            .map_err(|e| format!("Failed to get Fernet class: {}", e))?;
        
        let fernet = fernet_class
            .call1((key,))
            .map_err(|e| format!("Failed to create Fernet instance: {}", e))?;
        
        // Decrypt
        let decrypted_bytes = fernet
            .call_method1("decrypt", (encrypted_data,))
            .map_err(|e| format!("Failed to decrypt: {}", e))?;
        
        // Convert bytes to string
        let decrypted_str: String = decrypted_bytes
            .call_method0("decode")
            .map_err(|e| format!("Failed to decode: {}", e))?
            .extract()
            .map_err(|e| format!("Failed to extract string: {}", e))?;
        
        Ok(decrypted_str)
    })
}

/// Encripta uma string usando Fernet via Python
pub fn encrypt_fernet_via_python(plaintext: &str, key: &str) -> Result<String, String> {
    Python::with_gil(|py| {
        // Import cryptography.fernet
        let fernet_module = py
            .import("cryptography.fernet")
            .map_err(|e| format!("Failed to import cryptography.fernet: {}", e))?;
        
        // Create Fernet instance
        let fernet_class = fernet_module
            .getattr("Fernet")
            .map_err(|e| format!("Failed to get Fernet class: {}", e))?;
        
        let fernet = fernet_class
            .call1((key,))
            .map_err(|e| format!("Failed to create Fernet instance: {}", e))?;
        
        // Convert string to bytes
        let plaintext_bytes = plaintext.as_bytes();
        
        // Encrypt
        let encrypted_bytes = fernet
            .call_method1("encrypt", (plaintext_bytes,))
            .map_err(|e| format!("Failed to encrypt: {}", e))?;
        
        // Convert bytes to string
        let encrypted_str: String = encrypted_bytes
            .call_method0("decode")
            .map_err(|e| format!("Failed to decode: {}", e))?
            .extract()
            .map_err(|e| format!("Failed to extract string: {}", e))?;
        
        Ok(encrypted_str)
    })
}
