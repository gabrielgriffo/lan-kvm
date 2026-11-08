use std::io::{self, Read, Write};
use std::sync::Arc;

use snow::{Builder, HandshakeState, StatelessTransportState};

pub const KEY_LEN: usize = 32;
const PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_SHA256";
const MAX_MESSAGE: usize = 65535;
const TAG_LEN: usize = 16;
const PROLOGUE: &[u8] = b"lan-kvm-v1:";
const MAX_PLAIN: usize = MAX_MESSAGE - TAG_LEN;

pub type Key = [u8; KEY_LEN];

fn noise_error(e: snow::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("falha na autenticação: {e}"),
    )
}

pub fn write_chunk(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
    let len = u16::try_from(data.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bloco grande demais"))?;
    let mut out = Vec::with_capacity(2 + data.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(data);
    w.write_all(&out)
}

pub fn read_chunk(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    r.read_exact(&mut len)?;
    let mut data = vec![0u8; u16::from_be_bytes(len) as usize];
    r.read_exact(&mut data)?;
    Ok(data)
}

fn builder<'a>(key: &'a Key, prologue: &'a [u8]) -> Result<Builder<'a>, snow::Error> {
    Builder::new(PATTERN.parse()?)
        .psk(0, key)?
        .prologue(prologue)
}

fn prologue(mode: u8) -> Vec<u8> {
    let mut p = PROLOGUE.to_vec();
    p.push(mode);
    p
}

pub fn handshake_initiator(
    stream: &mut (impl Read + Write),
    key: &Key,
    mode: u8,
) -> io::Result<Arc<StatelessTransportState>> {
    let prologue = prologue(mode);
    let mut hs = builder(key, &prologue)
        .and_then(Builder::build_initiator)
        .map_err(noise_error)?;
    let mut buf = vec![0u8; MAX_MESSAGE];
    let len = hs.write_message(&[], &mut buf).map_err(noise_error)?;
    write_chunk(stream, &buf[..len])?;
    let reply = read_chunk(stream)?;
    hs.read_message(&reply, &mut buf).map_err(noise_error)?;
    finish(hs)
}

pub fn handshake_responder(
    stream: &mut (impl Read + Write),
    key: &Key,
    mode: u8,
) -> io::Result<Arc<StatelessTransportState>> {
    let prologue = prologue(mode);
    let mut hs = builder(key, &prologue)
        .and_then(Builder::build_responder)
        .map_err(noise_error)?;
    let mut buf = vec![0u8; MAX_MESSAGE];
    let first = read_chunk(stream)?;
    if let Err(e) = hs.read_message(&first, &mut buf) {
        let _ = write_chunk(stream, &[]);
        return Err(noise_error(e));
    }
    let len = hs.write_message(&[], &mut buf).map_err(noise_error)?;
    write_chunk(stream, &buf[..len])?;
    finish(hs)
}

fn finish(hs: HandshakeState) -> io::Result<Arc<StatelessTransportState>> {
    hs.into_stateless_transport_mode()
        .map(Arc::new)
        .map_err(noise_error)
}

pub struct SecureWriter<W> {
    inner: W,
    transport: Arc<StatelessTransportState>,
    nonce: u64,
    buf: Vec<u8>,
}

impl<W: Write> SecureWriter<W> {
    pub fn new(inner: W, transport: Arc<StatelessTransportState>) -> Self {
        Self {
            inner,
            transport,
            nonce: 0,
            buf: vec![0u8; MAX_MESSAGE],
        }
    }

    pub fn get_ref(&self) -> &W {
        &self.inner
    }
}

impl<W: Write> Write for SecureWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        for chunk in data.chunks(MAX_PLAIN) {
            let len = self
                .transport
                .write_message(self.nonce, chunk, &mut self.buf)
                .map_err(noise_error)?;
            self.nonce += 1;
            write_chunk(&mut self.inner, &self.buf[..len])?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub struct SecureReader<R> {
    inner: R,
    transport: Arc<StatelessTransportState>,
    nonce: u64,
    plain: Vec<u8>,
    pos: usize,
}

impl<R: Read> SecureReader<R> {
    pub fn new(inner: R, transport: Arc<StatelessTransportState>) -> Self {
        Self {
            inner,
            transport,
            nonce: 0,
            plain: Vec::new(),
            pos: 0,
        }
    }
}

impl<R: Read> Read for SecureReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.plain.len() {
            let cipher = match read_chunk(&mut self.inner) {
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(0),
                other => other?,
            };
            self.plain.resize(cipher.len(), 0);
            let len = self
                .transport
                .read_message(self.nonce, &cipher, &mut self.plain)
                .map_err(noise_error)?;
            self.nonce += 1;
            self.plain.truncate(len);
            self.pos = 0;
        }
        let n = out.len().min(self.plain.len() - self.pos);
        out[..n].copy_from_slice(&self.plain[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    pub fn pair_of_streams() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = thread::spawn(move || TcpStream::connect(addr).unwrap());
        let (server, _) = listener.accept().unwrap();
        (client.join().unwrap(), server)
    }

    fn connect(
        client_key: Key,
        server_key: Key,
    ) -> (
        io::Result<SecureWriter<TcpStream>>,
        io::Result<SecureReader<TcpStream>>,
    ) {
        let (mut client, mut server) = pair_of_streams();
        let srv = thread::spawn(move || {
            handshake_responder(&mut server, &server_key, b'S')
                .map(|t| SecureReader::new(server, t))
        });
        let cli = handshake_initiator(&mut client, &client_key, b'S')
            .map(|t| SecureWriter::new(client, t));
        (cli, srv.join().unwrap())
    }

    #[test]
    fn same_key_exchanges_messages_including_large_ones() {
        let (writer, reader) = connect([7; KEY_LEN], [7; KEY_LEN]);
        let (mut writer, mut reader) = (writer.unwrap(), reader.unwrap());
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let expected = big.clone();
        let sender = thread::spawn(move || {
            writer.write_all(b"oi").unwrap();
            writer.write_all(&big).unwrap();
        });
        let mut small = [0u8; 2];
        reader.read_exact(&mut small).unwrap();
        assert_eq!(&small, b"oi");
        let mut got = vec![0u8; expected.len()];
        reader.read_exact(&mut got).unwrap();
        sender.join().unwrap();
        assert_eq!(got, expected);
    }

    #[test]
    fn different_modes_fail_the_handshake() {
        let (mut client, mut server) = pair_of_streams();
        let key = [5; KEY_LEN];
        let srv = thread::spawn(move || handshake_responder(&mut server, &key, b'P'));
        let cli = handshake_initiator(&mut client, &key, b'S');
        assert!(srv.join().unwrap().is_err());
        assert!(cli.is_err());
    }

    #[test]
    fn different_keys_fail_the_handshake() {
        let (writer, reader) = connect([1; KEY_LEN], [2; KEY_LEN]);
        assert_eq!(
            reader.err().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(writer.is_err());
    }

    #[test]
    fn traffic_is_not_plaintext_and_tampering_is_detected() {
        let (mut client, mut server) = pair_of_streams();
        let key = [9; KEY_LEN];
        let srv = thread::spawn(move || {
            let t = handshake_responder(&mut server, &key, b'S').unwrap();
            (server, t)
        });
        let t = handshake_initiator(&mut client, &key, b'S').unwrap();
        let (mut raw_server, server_t) = srv.join().unwrap();

        let mut writer = SecureWriter::new(Vec::new(), Arc::clone(&t));
        writer.write_all(b"senha secreta").unwrap();
        let wire = writer.get_ref().clone();
        assert!(!wire.windows(13).any(|w| w == b"senha secreta"));

        let mut tampered = wire.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        let mut reader = SecureReader::new(tampered.as_slice(), Arc::clone(&server_t));
        let mut out = [0u8; 13];
        assert!(reader.read(&mut out).is_err());

        let mut reader = SecureReader::new(wire.as_slice(), server_t);
        reader.read_exact(&mut out).unwrap();
        assert_eq!(&out, b"senha secreta");
        let _ = raw_server.flush();
    }

    #[test]
    fn replayed_chunk_is_rejected() {
        let key = [3; KEY_LEN];
        let (mut client, mut server) = pair_of_streams();
        let srv = thread::spawn(move || handshake_responder(&mut server, &key, b'S').unwrap());
        let t = handshake_initiator(&mut client, &key, b'S').unwrap();
        let server_t = srv.join().unwrap();
        let mut writer = SecureWriter::new(Vec::new(), t);
        writer.write_all(b"um").unwrap();
        let once = writer.get_ref().clone();
        let mut twice = once.clone();
        twice.extend_from_slice(&once);
        let mut reader = SecureReader::new(twice.as_slice(), server_t);
        let mut out = [0u8; 2];
        reader.read_exact(&mut out).unwrap();
        assert!(reader.read(&mut out).is_err());
    }
}
