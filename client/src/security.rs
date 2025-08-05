use chacha20poly1305::aead::Aead;
use chacha20poly1305::KeyInit;

/// Used for encrypting a message for DMs
pub(crate) fn encrypt_bytes(
    recipient_public_key: x25519_dalek::PublicKey,
    my_secret: x25519_dalek::StaticSecret,
    bytes: Vec<u8>,
) -> Vec<u8> {
    // generate our shared secret and initialise chacha20poly1305
    let shared_secret = my_secret.diffie_hellman(&recipient_public_key);
    let key = chacha20poly1305::Key::from_slice(shared_secret.as_bytes());
    let cipher = chacha20poly1305::ChaCha20Poly1305::new(key);

    // generate random nonce
    let mut nonce_bytes = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
    let nonce = chacha20poly1305::Nonce::from_slice(&nonce_bytes);

    // encrypt our data and adds it to our nonce
    let mut encrypted_bytes = nonce.to_vec();
    encrypted_bytes.extend(cipher.encrypt(nonce, bytes.as_ref())
        .expect("Encryption failed"));

    encrypted_bytes
}

/// Used for decrypting a received message in DMs
pub(crate) fn decrypt_bytes(
    sender_public_key: x25519_dalek::PublicKey,
    my_secret: x25519_dalek::StaticSecret,
    encrypted_bytes: Vec<u8>,
) -> Vec<u8> {
    // generate our shared secret and initialise chacha20poly1305
    let shared_secret = my_secret.diffie_hellman(&sender_public_key);
    let key = chacha20poly1305::Key::from_slice(shared_secret.as_bytes());
    let cipher = chacha20poly1305::ChaCha20Poly1305::new(key);

    // split the encrypted message into nonce and ciphertext bytes
    let (nonce_bytes, ciphertext_bytes) = encrypted_bytes.split_at(12);
    let nonce = chacha20poly1305::Nonce::from_slice(nonce_bytes);

    // decrypt the message (and replace if we can't decrypt it)
    let decrypted_bytes = cipher.decrypt(nonce, ciphertext_bytes)
        .unwrap_or_else(|_| "Could not decrypt message".as_bytes().to_vec());

    decrypted_bytes
}

/// wrapper for encrypting dms
pub(crate) fn encrypt_message(
    recipient_public_key: x25519_dalek::PublicKey,
    encryption_secret: x25519_dalek::StaticSecret,
    message: &str,
) -> Vec<u8> {
    encrypt_bytes(
        recipient_public_key,
        encryption_secret,
        Vec::from(message.as_bytes()),
    )
}

/// wrapper for decrypting dms
pub(crate) fn decrypt_message(
    recipient_public_key: x25519_dalek::PublicKey,
    encryption_secret: x25519_dalek::StaticSecret,
    encrypted_bytes: Vec<u8>,
) -> String {
    let decrypted_bytes = decrypt_bytes(
        recipient_public_key,
        encryption_secret,
        encrypted_bytes
    );
    String::from_utf8(decrypted_bytes).unwrap_or_else(|_| "Could not decrypt message".to_string())
}

