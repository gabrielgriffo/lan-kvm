use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use std::time::Instant;

use lan_kvm_core::frame::{self, Decoder};
use lan_kvm_core::proto::Message;

pub const DEFAULT_PORT: u16 = 24801;
pub const PEER_TIMEOUT: Duration = Duration::from_secs(5);

pub fn with_port(addr: &str) -> String {
    if addr.contains(':') {
        addr.to_string()
    } else {
        format!("{addr}:{DEFAULT_PORT}")
    }
}

pub struct Deadline<'a> {
    stream: &'a TcpStream,
    until: Instant,
}

impl<'a> Deadline<'a> {
    pub fn new(stream: &'a TcpStream, limit: Duration) -> Self {
        Self {
            stream,
            until: Instant::now() + limit,
        }
    }
}

impl Read for Deadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "prazo esgotado"));
        }
        self.stream.set_read_timeout(Some(left))?;
        (&*self.stream).read(buf).map_err(|e| match e.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
                io::Error::new(io::ErrorKind::TimedOut, "prazo esgotado")
            }
            _ => e,
        })
    }
}

impl Write for Deadline<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self.stream).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&*self.stream).flush()
    }
}

fn invalid(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

pub fn write_msg(w: &mut impl Write, msg: &Message) -> io::Result<()> {
    let mut payload = Vec::with_capacity(8);
    msg.encode(&mut payload);
    let mut out = Vec::with_capacity(payload.len() + frame::HEADER_LEN);
    frame::encode(&payload, &mut out).map_err(invalid)?;
    w.write_all(&out)
}

pub struct MsgReader<R> {
    inner: R,
    decoder: Decoder,
    buf: [u8; 4096],
}

impl<R: Read> MsgReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            decoder: Decoder::new(),
            buf: [0; 4096],
        }
    }

    pub fn next(&mut self) -> io::Result<Option<Message>> {
        loop {
            if let Some(payload) = self.decoder.next_frame().map_err(invalid)? {
                return Message::decode(&payload).map(Some).map_err(invalid);
            }
            let n = match self.inner.read(&mut self.buf) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                other => other?,
            };
            if n == 0 {
                return Ok(None);
            }
            self.decoder.push(&self.buf[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct InterruptedOnce<'a> {
        interrupted: bool,
        data: &'a [u8],
    }

    impl Read for InterruptedOnce<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.data.read(buf)
        }
    }

    #[test]
    fn deadline_bounds_a_trickling_sender() {
        use std::net::TcpListener;
        use std::thread;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let trickle = thread::spawn(move || {
            let mut s = TcpStream::connect(addr).unwrap();
            for _ in 0..30 {
                if s.write_all(&[0]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        let (server, _) = listener.accept().unwrap();
        server
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let started = Instant::now();
        let mut buf = [0u8; 1000];
        let err = Deadline::new(&server, Duration::from_millis(700))
            .read_exact(&mut buf)
            .unwrap_err();
        let took = started.elapsed();
        assert!(matches!(
            err.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));
        assert!(took < Duration::from_millis(1500), "{took:?}");
        drop(server);
        trickle.join().unwrap();
    }

    #[test]
    fn deadline_allows_fast_reads() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        client.write_all(b"abc").unwrap();
        let mut buf = [0u8; 3];
        Deadline::new(&server, Duration::from_secs(2))
            .read_exact(&mut buf)
            .unwrap();
        assert_eq!(&buf, b"abc");
    }

    #[test]
    fn retries_after_interrupted_read() {
        let mut wire = Vec::new();
        write_msg(&mut wire, &Message::Ping).unwrap();
        let mut r = MsgReader::new(InterruptedOnce {
            interrupted: false,
            data: &wire,
        });
        assert_eq!(r.next().unwrap(), Some(Message::Ping));
    }

    #[test]
    fn writes_and_reads_sequence_then_eof() {
        let msgs = [
            Message::Enter { x: 0, y: 10 },
            Message::Motion { x: 5, y: 11 },
            Message::Leave,
        ];
        let mut wire = Vec::new();
        for m in &msgs {
            write_msg(&mut wire, m).unwrap();
        }
        let mut r = MsgReader::new(wire.as_slice());
        for m in &msgs {
            assert_eq!(r.next().unwrap(), Some(m.clone()));
        }
        assert_eq!(r.next().unwrap(), None);
    }

    #[test]
    fn invalid_payload_is_an_error() {
        let mut wire = Vec::new();
        frame::encode(&[42], &mut wire).unwrap();
        let err = MsgReader::new(wire.as_slice()).next().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
