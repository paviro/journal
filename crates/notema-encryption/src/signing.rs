use crate::{EncryptionError, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use zeroize::Zeroizing;

/// Generate a fresh Ed25519 signing keypair from OS randomness.
pub(crate) fn generate_signing_key() -> Result<SigningKey> {
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut seed[..])
        .map_err(|error| EncryptionError::Randomness(error.to_string()))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// A signing key's public half encoded as `ed25519:<hex>`.
pub(crate) fn signing_public(signing: &SigningKey) -> String {
    format!(
        "ed25519:{}",
        hex::encode(signing.verifying_key().to_bytes())
    )
}

/// Sign `msg` with `signing`, returning the hex Ed25519 signature.
pub(crate) fn sign_bytes(signing: &SigningKey, msg: &[u8]) -> String {
    hex::encode(signing.sign(msg).to_bytes())
}

/// Parse an `ed25519:<hex>` public key into a verifier, or `None` if malformed.
pub(crate) fn parse_signing_public(signer: &str) -> Option<VerifyingKey> {
    let hex_key = signer.strip_prefix("ed25519:")?;
    let bytes = hex::decode(hex_key).ok()?;
    let array = <[u8; 32]>::try_from(bytes.as_slice()).ok()?;
    VerifyingKey::from_bytes(&array).ok()
}

/// Verify a hex Ed25519 signature by the `ed25519:<hex>` public key over `msg`.
/// Any malformed input verifies as `false` rather than erroring, so a corrupt
/// roster op is simply rejected. Uses strict verification (rejects non-canonical
/// signatures and small-order keys).
pub(crate) fn verify_signature(signer: &str, msg: &[u8], sig_hex: &str) -> bool {
    let Some(verifying) = parse_signing_public(signer) else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return false;
    };
    let Ok(sig_array) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
        return false;
    };
    verifying
        .verify_strict(msg, &Signature::from_bytes(&sig_array))
        .is_ok()
}
