use std::io::{self, Read, Write};

use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::secure::{
    handshake_initiator, handshake_responder, read_chunk, write_chunk, Key, SecureReader,
    SecureWriter, KEY_LEN,
};

pub const MODE_SESSION: u8 = b'S';
pub const MODE_PAIR: u8 = b'P';
pub const REPLY_OK: u8 = 0;
pub const REPLY_ALREADY_PAIRED: u8 = 1;
pub const REPLY_BUSY: u8 = 2;
pub const REPLY_NOT_PAIRED: u8 = 3;
pub const REPLY_LOCKED: u8 = 4;

const CODE_SPACE: u32 = 1_000_000;
const IDENTITY: &[u8] = b"lan-kvm-pair-v1";
const TOKEN_LABEL: &[u8] = b"lan-kvm-token-v1";
const CONFIRMATION: &[u8] = b"lan-kvm-paired";

pub fn new_code() -> io::Result<String> {
    let limit = u32::MAX - u32::MAX % CODE_SPACE;
    loop {
        let value = getrandom::u32().map_err(io::Error::other)?;
        if value < limit {
            return Ok(format!("{:06}", value % CODE_SPACE));
        }
    }
}

pub fn is_valid_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())
}

pub fn reply_text(reply: u8) -> &'static str {
    match reply {
        REPLY_OK => "ok",
        REPLY_ALREADY_PAIRED => {
            "o notebook já está pareado; rode `lan-kvm unpair` nele para parear de novo"
        }
        REPLY_BUSY => "o notebook recusou o pedido; espere alguns segundos e tente de novo",
        REPLY_NOT_PAIRED => "o notebook não está pareado com este PC",
        REPLY_LOCKED => {
            "o notebook bloqueou novos pareamentos após códigos errados; reinicie a sessão nele"
        }
        _ => "resposta desconhecida do notebook",
    }
}

fn derive_token(code: &str, stream: &mut (impl Read + Write)) -> io::Result<Key> {
    if !is_valid_code(code) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "o código tem 6 dígitos",
        ));
    }
    let (state, outbound) = Spake2::<Ed25519Group>::start_symmetric(
        &Password::new(code.as_bytes()),
        &Identity::new(IDENTITY),
    );
    write_chunk(stream, &outbound)?;
    let inbound = read_chunk(stream)?;
    let shared = state
        .finish(&inbound)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("pareamento: {e:?}")))?;
    let digest = Sha256::new()
        .chain_update(TOKEN_LABEL)
        .chain_update(&shared)
        .finalize();
    let mut token = [0u8; KEY_LEN];
    token.copy_from_slice(&digest);
    Ok(token)
}

fn confirm_failure(e: io::Error) -> io::Error {
    if e.kind() == io::ErrorKind::PermissionDenied {
        io::Error::new(io::ErrorKind::PermissionDenied, "código incorreto")
    } else {
        e
    }
}

pub fn pair_initiator(stream: &mut (impl Read + Write), code: &str) -> io::Result<Key> {
    let token = derive_token(code, stream)?;
    let transport = handshake_initiator(stream, &token, MODE_PAIR).map_err(confirm_failure)?;
    SecureWriter::new(&mut *stream, transport).write_all(CONFIRMATION)?;
    Ok(token)
}

pub fn pair_responder(stream: &mut (impl Read + Write), code: &str) -> io::Result<Key> {
    let token = derive_token(code, stream)?;
    let transport = handshake_responder(stream, &token, MODE_PAIR).map_err(confirm_failure)?;
    let mut confirmation = vec![0u8; CONFIRMATION.len()];
    SecureReader::new(&mut *stream, transport)
        .read_exact(&mut confirmation)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "o PC não confirmou o pareamento",
            )
        })?;
    if confirmation != CONFIRMATION {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "confirmação de pareamento inválida",
        ));
    }
    Ok(token)
}

pub fn to_hex(key: &Key) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(text: &str) -> Option<Key> {
    let text = text.trim();
    if text.len() != KEY_LEN * 2 || !text.is_ascii() {
        return None;
    }
    let mut key = [0u8; KEY_LEN];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secure::tests::pair_of_streams;
    use std::thread;

    fn run(
        client_code: &'static str,
        server_code: &'static str,
    ) -> (io::Result<Key>, io::Result<Key>) {
        let (mut client, mut server) = pair_of_streams();
        let srv = thread::spawn(move || pair_responder(&mut server, server_code));
        let cli = pair_initiator(&mut client, client_code);
        (cli, srv.join().unwrap())
    }

    #[test]
    fn same_code_gives_same_token_on_both_sides() {
        let (a, b) = run("123456", "123456");
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a, b);
        assert_ne!(a, [0u8; KEY_LEN]);
    }

    #[test]
    fn each_pairing_gives_a_fresh_token() {
        let (a, _) = run("000001", "000001");
        let (b, _) = run("000001", "000001");
        assert_ne!(a.unwrap(), b.unwrap());
    }

    #[test]
    fn wrong_code_fails_on_both_sides() {
        let (a, b) = run("123456", "654321");
        assert_eq!(a.unwrap_err().to_string(), "código incorreto");
        assert_eq!(b.unwrap_err().to_string(), "código incorreto");
    }

    #[test]
    fn responder_does_not_accept_when_initiator_never_confirms() {
        let (mut client, mut server) = pair_of_streams();
        let srv = thread::spawn(move || pair_responder(&mut server, "246810"));
        let token = derive_token("246810", &mut client).unwrap();
        handshake_initiator(&mut client, &token, MODE_PAIR).unwrap();
        drop(client);
        let err = srv.join().unwrap().unwrap_err();
        assert_eq!(err.to_string(), "o PC não confirmou o pareamento");
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..200 {
            assert!(is_valid_code(&new_code().unwrap()));
        }
        assert!(!is_valid_code("12345"));
        assert!(!is_valid_code("12345a"));
        assert!(!is_valid_code("１２３４５６"));
    }

    #[test]
    fn hex_roundtrip_and_rejects_garbage() {
        let key: Key = std::array::from_fn(|i| (i * 7) as u8);
        assert_eq!(from_hex(&to_hex(&key)), Some(key));
        assert_eq!(from_hex(&format!(" {}\n", to_hex(&key))), Some(key));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex(&"zz".repeat(KEY_LEN)), None);
    }
}
