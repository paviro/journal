//! System clipboard access.
//!
//! Two backends, so copy/paste behave the same everywhere they can:
//! - On desktop (macOS/Windows) a native clipboard (`arboard`) both reads and
//!   writes the real OS clipboard — so a `Ctrl/Cmd+V` keystroke pastes the system
//!   clipboard, not just an in-app buffer.
//! - Everywhere, copy also emits an OSC 52 escape, which travels over SSH and works
//!   on iSH/termux where a native library has no backend.
//!
//! Reading is native-only ([`system_paste`] returns `None` without a backend); on
//! those hosts paste still arrives through the terminal's bracketed-paste gesture.

use std::io::{self, Write};

/// Push `text` to the system clipboard through every available backend. Best-effort:
/// a terminal that doesn't support OSC 52 ignores the sequence, `arboard` failures
/// are dropped, and the internal yank buffer remains as the in-app fallback.
pub(crate) fn system_copy(text: &str) {
    if text.is_empty() {
        return;
    }
    native_set_text(text);
    let mut stdout = io::stdout();
    let _ = stdout.write_all(osc52(text).as_bytes());
    let _ = stdout.flush();
}

/// Read the system clipboard where a native backend exists (desktop). Returns
/// `None` on hosts without one (iSH/termux/SSH), where the caller falls back to the
/// internal yank buffer and system paste comes through bracketed paste instead.
pub(crate) fn system_paste() -> Option<String> {
    native_get_text()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn native_set_text(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text.to_owned());
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn native_set_text(_text: &str) {}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn native_get_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn native_get_text() -> Option<String> {
    None
}

/// The OSC 52 "set clipboard" sequence for `text`: `ESC ] 52 ; c ; <base64> ESC \`.
fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x1b\\", base64_encode(text.as_bytes()))
}

/// Minimal standard-alphabet base64 (RFC 4648) with `=` padding. Inlined to keep
/// the OSC 52 writer dependency-free.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(n >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        // RFC 4648 test vectors.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_non_ascii() {
        // "täg" is multi-byte UTF-8; encode the raw bytes.
        assert_eq!(base64_encode("täg".as_bytes()), "dMOkZw==");
    }

    #[test]
    fn osc52_wraps_base64_in_the_set_clipboard_sequence() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x1b\\");
    }
}
