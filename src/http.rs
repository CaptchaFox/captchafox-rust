//! The one exchange with siteverify: a form sent by POST over HTTP/1.1, and the body of an answer with status 200.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::VerificationUnavailable;

// siteverify answers with a few hundred bytes; the bound keeps a server that sends without end from filling memory
// until the timeout.
const MAX_ANSWER: usize = 1024 * 1024;

/// Sends the form and returns the body of the answer, if its status is 200. The connection is meant to be used once.
pub(crate) async fn post_form<S>(
    stream: &mut S,
    host: &str,
    path: &str,
    form: &str,
) -> Result<Vec<u8>, VerificationUnavailable>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         User-Agent: captchafox-rust/{version}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Content-Length: {length}\r\n\
         Accept-Encoding: identity\r\n\
         Connection: close\r\n\
         \r\n\
         {form}",
        version = env!("CARGO_PKG_VERSION"),
        length = form.len(),
    );
    let sent = match stream.write_all(request.as_bytes()).await {
        Ok(()) => stream.flush().await,
        Err(error) => Err(error),
    };
    if let Err(error) = sent {
        return Err(VerificationUnavailable::caused_by(
            "the request to siteverify broke off",
            error,
        ));
    }

    let mut answer = Answer {
        stream,
        buffer: Vec::new(),
        received: 0,
    };

    let status = status(&answer.line().await?)?;
    if status != 200 {
        return Err(VerificationUnavailable::new(format!(
            "siteverify answered with HTTP status {status}"
        )));
    }

    match answer.framing().await? {
        Framing::Length(length) => answer.exact(length).await,
        Framing::Chunked => answer.chunked().await,
        Framing::ToEnd => answer.rest().await,
    }
}

enum Framing {
    Length(usize),
    Chunked,
    // Neither Content-Length nor Transfer-Encoding: the body ends with the connection.
    ToEnd,
}

struct Answer<'a, S> {
    stream: &'a mut S,
    // What was received and not yet taken.
    buffer: Vec<u8>,
    received: usize,
}

impl<S: AsyncRead + Unpin> Answer<'_, S> {
    /// Reads more of the answer into the buffer; `false` when the connection has ended.
    async fn fill(&mut self) -> Result<bool, VerificationUnavailable> {
        let mut chunk = [0; 8192];
        let read = match self.stream.read(&mut chunk).await {
            Ok(read) => read,
            Err(error) => {
                return Err(VerificationUnavailable::caused_by(
                    "the answer of siteverify broke off",
                    error,
                ));
            }
        };
        if read == 0 {
            return Ok(false);
        }

        self.received += read;
        if self.received > MAX_ANSWER {
            return Err(malformed("the answer is larger than 1 MiB"));
        }
        self.buffer.extend_from_slice(&chunk[..read]);
        Ok(true)
    }

    /// Takes a line without its CRLF.
    async fn line(&mut self) -> Result<Vec<u8>, VerificationUnavailable> {
        let mut searched = 0;
        loop {
            if let Some(at) = self.buffer[searched..]
                .windows(2)
                .position(|pair| pair == b"\r\n")
            {
                let end = searched + at;
                let line = self.buffer[..end].to_vec();
                self.buffer.drain(..end + 2);
                return Ok(line);
            }

            searched = self.buffer.len().saturating_sub(1);
            if !self.fill().await? {
                return Err(ended_early());
            }
        }
    }

    async fn exact(&mut self, length: usize) -> Result<Vec<u8>, VerificationUnavailable> {
        while self.buffer.len() < length {
            if !self.fill().await? {
                return Err(ended_early());
            }
        }
        Ok(self.buffer.drain(..length).collect())
    }

    async fn rest(&mut self) -> Result<Vec<u8>, VerificationUnavailable> {
        while self.fill().await? {}
        Ok(std::mem::take(&mut self.buffer))
    }

    /// Reads the header fields and decides how the body is delimited (RFC 9112, section 6.3). A framing that is
    /// ambiguous is refused rather than resolved, so that no reading of the answer can differ from the server's.
    async fn framing(&mut self) -> Result<Framing, VerificationUnavailable> {
        let mut length = None;
        let mut chunked = false;
        loop {
            let line = self.line().await?;
            if line.is_empty() {
                break;
            }

            let (name, value) = header(&line)?;
            if name.eq_ignore_ascii_case(b"content-length") {
                if length.is_some() {
                    return Err(malformed("Content-Length is repeated"));
                }
                length = Some(content_length(value)?);
            } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
                if chunked {
                    return Err(malformed("Transfer-Encoding is repeated"));
                }
                if !value.eq_ignore_ascii_case(b"chunked") {
                    return Err(malformed("a transfer coding other than chunked"));
                }
                chunked = true;
            }
        }

        match (chunked, length) {
            (true, Some(_)) => Err(malformed("both Content-Length and Transfer-Encoding")),
            (true, None) => Ok(Framing::Chunked),
            (false, Some(length)) => Ok(Framing::Length(length)),
            (false, None) => Ok(Framing::ToEnd),
        }
    }

    async fn chunked(&mut self) -> Result<Vec<u8>, VerificationUnavailable> {
        let mut body = Vec::new();
        loop {
            let size = chunk_size(&self.line().await?)?;
            if size == 0 {
                break;
            }

            body.extend_from_slice(&self.exact(size).await?);
            if self.exact(2).await? != b"\r\n" {
                return Err(malformed("a chunk is longer than its size"));
            }
        }

        // The trailer fields up to the empty line; none of them is needed.
        while !self.line().await?.is_empty() {}
        Ok(body)
    }
}

/// `HTTP/1.x SP 3DIGIT [SP reason]`
fn status(line: &[u8]) -> Result<u16, VerificationUnavailable> {
    match line {
        [b'H', b'T', b'T', b'P', b'/', b'1', b'.', minor, b' ', hundreds, tens, ones, rest @ ..]
            if minor.is_ascii_digit()
                && hundreds.is_ascii_digit()
                && tens.is_ascii_digit()
                && ones.is_ascii_digit()
                && (rest.is_empty() || rest[0] == b' ') =>
        {
            Ok(u16::from(hundreds - b'0') * 100
                + u16::from(tens - b'0') * 10
                + u16::from(ones - b'0'))
        }
        _ => Err(malformed("no HTTP/1 status line")),
    }
}

fn header(line: &[u8]) -> Result<(&[u8], &[u8]), VerificationUnavailable> {
    let Some(colon) = line.iter().position(|&byte| byte == b':') else {
        return Err(malformed("a header line without a colon"));
    };

    // Also refuses a folded line and whitespace before the colon, which RFC 9112 forbids.
    let name = &line[..colon];
    if name.is_empty() || !name.iter().all(|&byte| is_token(byte)) {
        return Err(malformed("a header name that is no token"));
    }

    let value = &line[colon + 1..];
    let start = value
        .iter()
        .position(|&byte| byte != b' ' && byte != b'\t')
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|&byte| byte != b' ' && byte != b'\t')
        .map_or(start, |last| last + 1);
    Ok((name, &value[start..end]))
}

fn is_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn content_length(value: &[u8]) -> Result<usize, VerificationUnavailable> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return Err(malformed("a Content-Length that is no number"));
    }

    let mut length: usize = 0;
    for digit in value {
        length = length
            .checked_mul(10)
            .and_then(|length| length.checked_add(usize::from(digit - b'0')))
            .filter(|&length| length <= MAX_ANSWER)
            .ok_or_else(|| malformed("the answer is larger than 1 MiB"))?;
    }
    Ok(length)
}

/// `chunk-size [BWS ";" chunk-ext]`; the extensions are not needed.
fn chunk_size(line: &[u8]) -> Result<usize, VerificationUnavailable> {
    let digits = line
        .iter()
        .position(|byte| !byte.is_ascii_hexdigit())
        .unwrap_or(line.len());
    if digits == 0 {
        return Err(malformed("a chunk size that is no hexadecimal number"));
    }

    let extension = line[digits..]
        .iter()
        .find(|&&byte| byte != b' ' && byte != b'\t');
    if extension.is_some_and(|&byte| byte != b';') {
        return Err(malformed("a chunk size that is no hexadecimal number"));
    }

    let mut size: usize = 0;
    for &digit in &line[..digits] {
        let value = char::from(digit).to_digit(16).unwrap_or(0) as usize;
        size = size
            .checked_mul(16)
            .and_then(|size| size.checked_add(value))
            .filter(|&size| size <= MAX_ANSWER)
            .ok_or_else(|| malformed("the answer is larger than 1 MiB"))?;
    }
    Ok(size)
}

fn malformed(problem: &str) -> VerificationUnavailable {
    VerificationUnavailable::new(format!(
        "siteverify answered with a malformed HTTP response: {problem}"
    ))
}

fn ended_early() -> VerificationUnavailable {
    VerificationUnavailable::new("the answer of siteverify ended early")
}
