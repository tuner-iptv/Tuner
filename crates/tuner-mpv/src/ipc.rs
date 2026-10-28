use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Error, Result};

const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Events kept for [`Connection::take_events`]; older ones are dropped.
const MAX_EVENTS: usize = 256;

/// A request/reply channel to one mpv IPC socket.
///
/// mpv interleaves asynchronous events with command replies on the same
/// socket; replies are matched by `request_id`. Events are queued for
/// [`Connection::take_events`].
pub struct Connection {
    stream: UnixStream,
    /// Bytes read but not yet split into lines.
    buf: Vec<u8>,
    events: VecDeque<Value>,
    next_id: u64,
}

impl Connection {
    pub fn new(stream: UnixStream) -> Result<Self> {
        stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
        Ok(Self {
            stream,
            buf: Vec::new(),
            events: VecDeque::new(),
            next_id: 1,
        })
    }

    /// Send a command such as `["loadfile", url, "replace"]` and wait for its
    /// reply. Returns the reply's `data` field (`null` for most commands).
    pub fn command(&mut self, args: &[Value]) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let mut line = json!({ "command": args, "request_id": id }).to_string();
        line.push('\n');
        self.stream.write_all(line.as_bytes())?;

        loop {
            let reply = match self.take_line() {
                Some(line) => serde_json::from_slice::<Value>(&line)?,
                None => {
                    self.fill()?;
                    continue;
                }
            };
            if reply.get("request_id").and_then(Value::as_u64) != Some(id) {
                self.queue(reply);
                continue;
            }
            return match reply.get("error").and_then(Value::as_str) {
                Some("success") => Ok(reply.get("data").cloned().unwrap_or(Value::Null)),
                Some(err) => Err(Error::Command(err.to_string())),
                None => Err(Error::Command("reply without error field".into())),
            };
        }
    }

    /// Read whatever mpv has sent without waiting, queueing its events.
    pub fn poll(&mut self) -> Result<()> {
        self.stream.set_nonblocking(true)?;
        let read = self.read_available();
        self.stream.set_nonblocking(false)?;
        while let Some(line) = self.take_line() {
            // Skip garbage rather than lose the events after it.
            if let Ok(message) = serde_json::from_slice(&line) {
                self.queue(message);
            }
        }
        read
    }

    /// Events received so far (`{"event": "end-file", …}`), oldest first.
    pub fn take_events(&mut self) -> Vec<Value> {
        self.events.drain(..).collect()
    }

    fn read_available(&mut self) -> Result<()> {
        loop {
            match self.fill() {
                Ok(()) => {}
                Err(Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    /// Read one chunk from the socket (blocking up to the reply timeout).
    fn fill(&mut self) -> Result<()> {
        let mut chunk = [0; 4096];
        let n = self.stream.read(&mut chunk)?;
        if n == 0 {
            return Err(Error::Closed);
        }
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(())
    }

    /// The next complete non-empty line, without its newline.
    fn take_line(&mut self) -> Option<Vec<u8>> {
        loop {
            let end = self.buf.iter().position(|&b| b == b'\n')?;
            let mut line: Vec<u8> = self.buf.drain(..=end).collect();
            line.pop();
            if !line.iter().all(u8::is_ascii_whitespace) {
                return Some(line);
            }
        }
    }

    /// Keep events; stale replies (to commands that timed out) are dropped.
    fn queue(&mut self, message: Value) {
        if message.get("event").is_none() {
            return;
        }
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::thread;

    /// Fake mpv: reads one request, emits an unrelated event, then replies.
    fn fake_mpv(reply: impl Fn(u64) -> String + Send + 'static) -> Connection {
        let (client, server) = UnixStream::pair().unwrap();
        thread::spawn(move || {
            let mut reader = BufReader::new(server.try_clone().unwrap());
            let mut writer = server;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let req: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["command"][0], "loadfile");
            let id = req["request_id"].as_u64().unwrap();
            writeln!(writer, r#"{{"event":"start-file","playlist_entry_id":1}}"#).unwrap();
            writeln!(writer, "{}", reply(id)).unwrap();
        });
        Connection::new(client).unwrap()
    }

    #[test]
    fn skips_events_and_returns_data() {
        let mut conn =
            fake_mpv(|id| format!(r#"{{"request_id":{id},"error":"success","data":42}}"#));
        let data = conn
            .command(&[json!("loadfile"), json!("http://a")])
            .unwrap();
        assert_eq!(data, json!(42));
    }

    #[test]
    fn keeps_events_seen_while_waiting_for_a_reply() {
        let mut conn = fake_mpv(|id| format!(r#"{{"request_id":{id},"error":"success"}}"#));
        conn.command(&[json!("loadfile")]).unwrap();
        let events = conn.take_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "start-file");
        assert!(conn.take_events().is_empty());
    }

    #[test]
    fn surfaces_mpv_errors() {
        let mut conn =
            fake_mpv(|id| format!(r#"{{"request_id":{id},"error":"invalid parameter"}}"#));
        let err = conn.command(&[json!("loadfile")]).unwrap_err();
        assert!(matches!(err, Error::Command(msg) if msg == "invalid parameter"));
    }

    #[test]
    fn poll_reads_events_without_waiting() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let mut conn = Connection::new(client).unwrap();
        conn.poll().unwrap();
        assert!(conn.take_events().is_empty());

        // A whole event, garbage, and half of the next event.
        server
            .write_all(b"{\"event\":\"end-file\",\"reason\":\"error\"}\nnot json\n{\"event\":")
            .unwrap();
        conn.poll().unwrap();
        let events = conn.take_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["reason"], "error");

        server.write_all(b"\"idle\"}\n").unwrap();
        conn.poll().unwrap();
        assert_eq!(conn.take_events()[0]["event"], "idle");
    }

    #[test]
    fn closed_socket_is_a_disconnect() {
        let (client, server) = UnixStream::pair().unwrap();
        drop(server);
        let mut conn = Connection::new(client).unwrap();
        let err = conn.command(&[json!("loadfile")]).unwrap_err();
        assert!(err.is_disconnect(), "{err:?}");
        assert!(conn.poll().unwrap_err().is_disconnect());
    }
}
