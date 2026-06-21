use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use crate::utils::error::AppError;

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Password hashing failed: {}", e)))?
        .to_string();

    Ok(password_hash)
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| AppError::Auth(format!("Invalid password hash format: {}", e)))?;
        
    let argon2 = Argon2::default();
    
    Ok(argon2.verify_password(password.as_bytes(), &parsed_hash).is_ok())
}

fn get_encryption_key() -> Result<Vec<u8>, AppError> {
    // Validated at startup (32 bytes, no insecure fallback) — see config::secrets.
    Ok(crate::config::secrets::encryption_key())
}

pub fn encrypt_env_value(plain_text: &str) -> Result<(String, String), AppError> {
    let key_bytes = get_encryption_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Crypto key initialization failed: {}", e)))?;
    
    let nonce_bytes = rand::random::<[u8; 12]>();
    let nonce = Nonce::from_slice(&nonce_bytes);
    
    let ciphertext = cipher
        .encrypt(nonce, plain_text.as_bytes())
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Encryption failed: {}", e)))?;

    Ok((BASE64.encode(ciphertext), BASE64.encode(nonce_bytes)))
}

pub fn decrypt_env_value(encrypted_base64: &str, nonce_base64: &str) -> Result<String, AppError> {
    let key_bytes = get_encryption_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Crypto key initialization failed: {}", e)))?;

    let ciphertext = BASE64.decode(encrypted_base64)
        .map_err(|e| AppError::Validation(format!("Invalid encrypted base64 payload: {}", e)))?;
    let nonce_bytes = BASE64.decode(nonce_base64)
        .map_err(|e| AppError::Validation(format!("Invalid nonce base64 payload: {}", e)))?;

    let nonce = Nonce::from_slice(&nonce_bytes);
    
    let decrypted_bytes = cipher
        .decrypt(nonce, ciphertext.as_slice())
        .map_err(|e| AppError::Permission(format!("Decryption rejected: {}", e)))?;

    String::from_utf8(decrypted_bytes)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Decrypted payload is not valid UTF-8: {}", e)))
}