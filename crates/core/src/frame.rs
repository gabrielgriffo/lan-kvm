use std::fmt;

pub const HEADER_LEN: usize = 4;
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    TooLarge(usize),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::TooLarge(len) => {
                write!(f, "frame de {len} bytes excede o limite de {MAX_PAYLOAD}")
            }
        }
    }
}

impl std::error::Error for FrameError {}

pub fn encode(payload: &[u8], out: &mut Vec<u8>) -> Result<(), FrameError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::TooLarge(payload.len()));
    }
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        if self.buf.len() < HEADER_LEN {
            return Ok(None);
        }
        let len = u32::from_be_bytes(self.buf[..HEADER_LEN].try_into().unwrap()) as usize;
        if len > MAX_PAYLOAD {
            return Err(FrameError::TooLarge(len));
        }
        if self.buf.len() < HEADER_LEN + len {
            return Ok(None);
        }
        let payload = self.buf[HEADER_LEN..HEADER_LEN + len].to_vec();
        self.buf.drain(..HEADER_LEN + len);
        Ok(Some(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        encode(payload, &mut out).unwrap();
        out
    }

    #[test]
    fn roundtrip() {
        let mut d = Decoder::new();
        d.push(&encoded(b"mouse"));
        assert_eq!(d.next_frame().unwrap(), Some(b"mouse".to_vec()));
        assert_eq!(d.next_frame().unwrap(), None);
    }

    #[test]
    fn empty_payload() {
        let mut d = Decoder::new();
        d.push(&encoded(b""));
        assert_eq!(d.next_frame().unwrap(), Some(Vec::new()));
    }

    #[test]
    fn byte_by_byte_arrival() {
        let bytes = encoded(b"tecla");
        let mut d = Decoder::new();
        for (i, b) in bytes.iter().enumerate() {
            d.push(std::slice::from_ref(b));
            let frame = d.next_frame().unwrap();
            if i + 1 < bytes.len() {
                assert_eq!(frame, None);
            } else {
                assert_eq!(frame, Some(b"tecla".to_vec()));
            }
        }
    }

    #[test]
    fn several_frames_in_one_read() {
        let mut bytes = encoded(b"a");
        bytes.extend(encoded(b"bc"));
        bytes.extend(&encoded(b"def")[..3]);
        let mut d = Decoder::new();
        d.push(&bytes);
        assert_eq!(d.next_frame().unwrap(), Some(b"a".to_vec()));
        assert_eq!(d.next_frame().unwrap(), Some(b"bc".to_vec()));
        assert_eq!(d.next_frame().unwrap(), None);
    }

    #[test]
    fn rejects_oversized_header_before_buffering_payload() {
        let mut d = Decoder::new();
        d.push(&((MAX_PAYLOAD as u32) + 1).to_be_bytes());
        assert_eq!(d.next_frame(), Err(FrameError::TooLarge(MAX_PAYLOAD + 1)));
    }

    #[test]
    fn encode_rejects_oversized_payload() {
        let big = vec![0u8; MAX_PAYLOAD + 1];
        let mut out = Vec::new();
        assert_eq!(
            encode(&big, &mut out),
            Err(FrameError::TooLarge(MAX_PAYLOAD + 1))
        );
        assert!(out.is_empty());
    }
}
