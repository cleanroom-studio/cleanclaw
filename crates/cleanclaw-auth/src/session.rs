//! Web session tokens (cookie session). 256 bits of entropy, base64
//! url-safe-no-pad.

use base64::Engine;
use rand::RngCore;

pub fn new_token() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}
