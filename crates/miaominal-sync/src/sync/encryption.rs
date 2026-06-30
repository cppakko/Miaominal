use aes_gcm::aead::{Aead, KeyInit, Nonce, array::Array};
use aes_gcm::Aes256Gcm;
use anyhow::{Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;

pub fn derive_key_with_params(
    passphrase: &str,
    salt: &[u8],
    memory_cost: u32,
    time_cost: u32,
    parallelism: u32,
    output_len: usize,
) -> Result<[u8; 32]> {
    if output_len != 32 {
        anyhow::bail!("unsupported Argon2 output length: {output_len}");
    }

    let params = Params::new(memory_cost, time_cost, parallelism, Some(output_len))
        .map_err(|e| anyhow::anyhow!("failed to create Argon2 params: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow::anyhow!("Argon2 key derivation failed: {e}"))?;
    Ok(key)
}

pub fn encrypt_with_aad(key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<String> {
    let cipher = Aes256Gcm::new(&Array(*key));
    let nonce = Nonce::<Aes256Gcm>::from(rand::random::<[u8; 12]>());
    let ciphertext = cipher
        .encrypt(
            &nonce,
            aes_gcm::aead::Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| anyhow::anyhow!("AES-GCM encryption failed: {e}"))?;

    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(base64::engine::general_purpose::STANDARD.encode(combined))
}

pub fn decrypt_with_aad(key: &[u8; 32], encoded: &str, aad: &[u8]) -> Result<Vec<u8>> {
    let combined = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("failed to base64-decode ciphertext")?;

    if combined.len() < 12 {
        anyhow::bail!("ciphertext too short to contain nonce");
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher = Aes256Gcm::new(&Array(*key));
    let nonce = Nonce::<Aes256Gcm>::try_from(nonce_bytes)
        .map_err(|_| anyhow::anyhow!("ciphertext nonce must be 12 bytes"))?;

    cipher
        .decrypt(
            &nonce,
            aes_gcm::aead::Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|e| anyhow::anyhow!("AES-GCM decryption failed: {e}"))
}
