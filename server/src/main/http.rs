fn http_request_json(
    server_url: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> anyhow::Result<Value> {
    let body = http_request(server_url, method, path, body)?;
    serde_json::from_slice(&body).context("parse SimDeck service JSON response")
}

fn http_request(
    server_url: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> anyhow::Result<Vec<u8>> {
    let endpoint = HttpEndpoint::parse(server_url)?;
    let mut stream = std::net::TcpStream::connect((endpoint.host.as_str(), endpoint.port))
        .with_context(|| format!("connect to SimDeck service at {server_url}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(180)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let body = body.map(serde_json::to_vec).transpose()?;
    let request = if let Some(body) = body.as_ref() {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            endpoint.host_header(),
            endpoint.origin(),
            body.len(),
        )
    } else {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            endpoint.host_header(),
            endpoint.origin(),
        )
    };
    stream.write_all(request.as_bytes())?;
    if let Some(body) = body.as_ref() {
        stream.write_all(body)?;
    }

    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let (status, headers, body) = parse_http_response(&response)?;
    let body = if response_is_chunked(&headers) {
        decode_chunked_body(body)?
    } else {
        body.to_vec()
    };
    if !(200..300).contains(&status) {
        let message = String::from_utf8_lossy(&body).trim().to_owned();
        anyhow::bail!(
            "SimDeck service returned HTTP {status}{}",
            if message.is_empty() {
                String::new()
            } else {
                format!(": {message}")
            }
        );
    }
    Ok(body)
}

struct HttpEndpoint {
    host: String,
    port: u16,
}

type HttpHeaders = Vec<(String, String)>;

impl HttpEndpoint {
    fn parse(server_url: &str) -> anyhow::Result<Self> {
        let without_scheme = server_url
            .trim_end_matches('/')
            .strip_prefix("http://")
            .ok_or_else(|| anyhow::anyhow!("Only http:// server URLs are supported."))?;
        let authority = without_scheme
            .split('/')
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Server URL must include a host."))?;
        let (host, port) = if let Some(host) = authority.strip_prefix('[') {
            let (host, rest) = host
                .split_once(']')
                .ok_or_else(|| anyhow::anyhow!("Invalid IPv6 server URL host."))?;
            let port = rest
                .strip_prefix(':')
                .map(parse_port)
                .transpose()?
                .unwrap_or(80);
            (host.to_owned(), port)
        } else if let Some((host, port)) = authority.rsplit_once(':') {
            (host.to_owned(), parse_port(port)?)
        } else {
            (authority.to_owned(), 80)
        };
        Ok(Self { host, port })
    }

    fn host_header(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    fn origin(&self) -> String {
        format!("http://{}", self.host_header())
    }
}

fn parse_port(value: &str) -> anyhow::Result<u16> {
    value
        .parse::<u16>()
        .with_context(|| format!("parse port `{value}`"))
}

fn parse_http_response(response: &[u8]) -> anyhow::Result<(u16, HttpHeaders, &[u8])> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("SimDeck service returned a malformed HTTP response."))?;
    let header_bytes = &response[..header_end];
    let body = &response[header_end + 4..];
    let header_text = std::str::from_utf8(header_bytes).context("parse HTTP headers as UTF-8")?;
    let mut lines = header_text.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("SimDeck service returned an empty HTTP response."))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("HTTP response did not include a status code."))?
        .parse::<u16>()
        .context("parse HTTP status code")?;
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    Ok((status, headers, body))
}

fn response_is_chunked(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name == "transfer-encoding"
            && value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
    })
}

fn decode_chunked_body(mut body: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut decoded = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| anyhow::anyhow!("Chunked response ended before a chunk size."))?;
        let size_text = std::str::from_utf8(&body[..line_end])
            .context("parse chunk size as UTF-8")?
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let size = usize::from_str_radix(size_text, 16).context("parse chunk size")?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        if body.len() < size + 2 {
            anyhow::bail!("Chunked response ended before a full chunk.");
        }
        decoded.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

fn url_path_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}
