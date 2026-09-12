//! Newline-delimited JSON framing.

use std::io::{self, BufRead, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Serializes one message as a single line, trailing newline included.
pub fn encode<T: Serialize>(message: &T) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    Ok(line)
}

pub fn decode<T: DeserializeOwned>(line: &str) -> serde_json::Result<T> {
    serde_json::from_str(line.trim_end())
}

pub fn write_message<W: Write, T: Serialize>(writer: &mut W, message: &T) -> io::Result<()> {
    writer.write_all(encode(message)?.as_bytes())?;
    writer.flush()
}

/// Reads one message; `Ok(None)` at a clean end of stream.
pub fn read_message<R: BufRead, T: DeserializeOwned>(reader: &mut R) -> io::Result<Option<T>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    decode(&line)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::Request;

    #[test]
    fn messages_are_one_line_each() {
        let mut buffer = Vec::new();
        write_message(&mut buffer, &Request::Version).unwrap();
        write_message(&mut buffer, &Request::Windows).unwrap();

        let mut reader = Cursor::new(buffer);
        assert_eq!(
            read_message::<_, Request>(&mut reader).unwrap(),
            Some(Request::Version)
        );
        assert_eq!(
            read_message::<_, Request>(&mut reader).unwrap(),
            Some(Request::Windows)
        );
        assert_eq!(read_message::<_, Request>(&mut reader).unwrap(), None);
    }

    #[test]
    fn garbage_is_invalid_data() {
        let mut reader = Cursor::new(b"not json\n".to_vec());
        let error = read_message::<_, Request>(&mut reader).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
