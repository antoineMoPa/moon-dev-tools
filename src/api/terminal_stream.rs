//! Terminal stream - a cursor followed by terminal bytes in each browser binary frame.
#[cfg(any(target_arch = "wasm32", test))]
use anyhow::{Result, ensure};

#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn encode(sequence: u64, bytes: &[u8]) -> Vec<u8> {
    let mut frame = sequence.to_le_bytes().to_vec();
    frame.extend_from_slice(bytes);
    frame
}
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn decode(frame: &[u8]) -> Result<(u64, &[u8])> {
    ensure!(frame.len() >= 8, "invalid terminal stream frame");
    Ok((
        u64::from_le_bytes(frame[..8].try_into().unwrap()),
        &frame[8..],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_preserves_split_vt_and_utf8_bytes() {
        for bytes in [b"\x1b[?2004".as_slice(), b"h\xf0\x9f", b"\x98\x80"] {
            let frame = encode(17, bytes);
            assert_eq!(decode(&frame).unwrap(), (17, bytes));
        }
        assert!(decode(b"short").is_err());
    }
}
