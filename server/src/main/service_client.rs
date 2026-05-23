fn service_get_json(server_url: &str, path: &str) -> anyhow::Result<Value> {
    http_request_json(server_url, "GET", path, None)
}

fn service_get_bytes(server_url: &str, path: &str) -> anyhow::Result<Vec<u8>> {
    http_request(server_url, "GET", path, None)
}

fn service_post_bytes(server_url: &str, path: &str, body: &Value) -> anyhow::Result<Vec<u8>> {
    http_request(server_url, "POST", path, Some(body))
}

fn service_open_url(server_url: &str, udid: &str, url: &str) -> anyhow::Result<()> {
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({ "action": "openUrl", "url": url }),
    )
}

fn service_launch(server_url: &str, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({ "action": "launch", "bundleId": bundle_id }),
    )
}

fn service_performance_json(
    server_url: &str,
    udid: &str,
    pid: Option<i32>,
) -> anyhow::Result<Value> {
    let mut path = format!(
        "/api/simulators/{}/performance?windowMs=120000",
        url_path_component(udid)
    );
    if let Some(pid) = pid {
        path.push_str(&format!("&pid={pid}"));
    }
    service_get_json(server_url, &path)
}

fn service_post_sample(
    server_url: &str,
    udid: &str,
    pid: i32,
    seconds: u64,
) -> anyhow::Result<Value> {
    http_request_json(
        server_url,
        "POST",
        &format!(
            "/api/simulators/{}/processes/{pid}/sample?seconds={}",
            url_path_component(udid),
            seconds.clamp(1, 30)
        ),
        None,
    )
}

fn run_stats_watch(
    server_url: &str,
    udid: &str,
    pid: Option<i32>,
    interval: f64,
) -> anyhow::Result<()> {
    let interval = Duration::from_secs_f64(interval.clamp(0.25, 60.0));
    loop {
        let stats = service_performance_json(server_url, udid, pid)?;
        print_performance_line(&stats)?;
        std::thread::sleep(interval);
    }
}

fn print_performance_line(stats: &Value) -> anyhow::Result<()> {
    let current = stats
        .get("current")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("No current performance sample is available."))?;
    let pid = current.get("pid").and_then(Value::as_i64).unwrap_or(0);
    let process = stats
        .get("processes")
        .and_then(Value::as_array)
        .and_then(|processes| {
            processes
                .iter()
                .find(|process| process.get("pid").and_then(Value::as_i64) == Some(pid))
        })
        .and_then(|process| process.get("process"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let cpu = current
        .get("cpuPercent")
        .and_then(Value::as_f64)
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "--".to_owned());
    let memory = current
        .get("memoryFootprintBytes")
        .or_else(|| current.get("memoryResidentBytes"))
        .and_then(Value::as_u64)
        .map(format_bytes_cli)
        .unwrap_or_else(|| "--".to_owned());
    let disk = current
        .get("diskWriteBytesPerSecond")
        .and_then(Value::as_f64)
        .map(|value| format!("{}/s", format_bytes_cli(value.max(0.0) as u64)))
        .unwrap_or_else(|| "--".to_owned());
    let network_in = current
        .get("networkReceivedBytesPerSecond")
        .and_then(Value::as_f64)
        .map(|value| format!("{}/s", format_bytes_cli(value.max(0.0) as u64)))
        .unwrap_or_else(|| "--".to_owned());
    let network_out = current
        .get("networkSentBytesPerSecond")
        .and_then(Value::as_f64)
        .map(|value| format!("{}/s", format_bytes_cli(value.max(0.0) as u64)))
        .unwrap_or_else(|| "--".to_owned());
    let connections = current
        .get("networkConnectionCount")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "--".to_owned());
    let hang = current
        .get("hang")
        .and_then(|hang| hang.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    println!(
        "{} pid={} process={} cpu={} memory={} diskWrite={} netIn={} netOut={} connections={} hang={}",
        chrono_like_time_label(),
        pid,
        process,
        cpu,
        memory,
        disk,
        network_in,
        network_out,
        connections,
        hang
    );
    io::stdout().flush()?;
    Ok(())
}

fn format_bytes_cli(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn chrono_like_time_label() -> String {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let seconds = now % 60;
    let minutes = (now / 60) % 60;
    let hours = (now / 3600) % 24;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn service_touch(server_url: &str, udid: &str, x: f64, y: f64, phase: &str) -> anyhow::Result<()> {
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({ "action": "touch", "x": x, "y": y, "phase": phase }),
    )
}

fn service_tap(
    server_url: &str,
    udid: &str,
    x: f64,
    y: f64,
    duration_ms: u64,
) -> anyhow::Result<()> {
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({
            "action": "tap",
            "x": x,
            "y": y,
            "normalized": true,
            "durationMs": duration_ms,
        }),
    )
}

fn service_tap_element(server_url: &str, udid: &str, body: Value) -> anyhow::Result<Value> {
    let mut body = body;
    if let Some(object) = body.as_object_mut() {
        object.insert("action".to_owned(), Value::String("tap".to_owned()));
    } else {
        body = serde_json::json!({ "action": "tap" });
    }
    service_action(server_url, udid, &body)
}

#[allow(clippy::too_many_arguments)]
fn service_wait_for_selector(
    server_url: &str,
    udid: &str,
    action: &str,
    selector: SelectorArgs,
    source: AccessibilitySource,
    max_depth: Option<usize>,
    include_hidden: bool,
    timeout_ms: u64,
    poll_interval_ms: u64,
) -> anyhow::Result<Value> {
    if selector.is_empty() {
        anyhow::bail!("{action} requires a selector flag.");
    }
    let body = serde_json::json!({
        "action": match action {
            "wait-for" => "waitFor",
            "assert" => "assert",
            other => other,
        },
        "selector": selector.to_json(),
        "source": source.as_query_value(),
        "maxDepth": max_depth,
        "includeHidden": include_hidden,
        "timeoutMs": timeout_ms,
        "pollMs": poll_interval_ms,
    });
    service_action(server_url, udid, &body)
}

fn service_batch(
    server_url: &str,
    udid: &str,
    steps: Vec<Value>,
    continue_on_error: bool,
) -> anyhow::Result<Value> {
    service_action(
        server_url,
        udid,
        &serde_json::json!({
            "action": "batch",
            "steps": steps,
            "continueOnError": continue_on_error,
        }),
    )
}

#[allow(clippy::too_many_arguments)]
fn service_swipe(
    server_url: &str,
    udid: &str,
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    duration_ms: u64,
    steps: u32,
) -> anyhow::Result<()> {
    let step_count = steps.max(2);
    let delay_ms = duration_ms / u64::from(step_count);
    let mut events = vec![service_touch_event(start_x, start_y, "began", 0)];
    for index in 1..step_count {
        let progress = f64::from(index) / f64::from(step_count);
        let x = start_x + (end_x - start_x) * progress;
        let y = start_y + (end_y - start_y) * progress;
        events.push(service_touch_event(x, y, "moved", delay_ms));
    }
    events.push(service_touch_event(end_x, end_y, "ended", 0));
    service_touch_sequence(server_url, udid, events)
}

fn service_touch_event(x: f64, y: f64, phase: &str, delay_ms_after: u64) -> Value {
    serde_json::json!({
        "x": x,
        "y": y,
        "phase": phase,
        "delayMsAfter": delay_ms_after,
    })
}

fn service_touch_sequence(server_url: &str, udid: &str, events: Vec<Value>) -> anyhow::Result<()> {
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({ "action": "touchSequence", "events": events }),
    )
}

fn service_key(server_url: &str, udid: &str, key_code: u16, modifiers: u32) -> anyhow::Result<()> {
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({ "action": "key", "keyCode": key_code, "modifiers": modifiers }),
    )
}

fn service_key_sequence(
    server_url: &str,
    udid: &str,
    keys: &[u16],
    delay_ms: u64,
) -> anyhow::Result<()> {
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({ "action": "keySequence", "keyCodes": keys, "delayMs": delay_ms }),
    )
}

fn service_button(
    server_url: &str,
    udid: &str,
    button: &str,
    duration_ms: u32,
) -> anyhow::Result<()> {
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({ "action": "button", "button": button, "durationMs": duration_ms }),
    )
}

fn service_crown(server_url: &str, udid: &str, delta: f64) -> anyhow::Result<()> {
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({ "action": "crown", "delta": delta }),
    )
}

fn service_action_ok(server_url: &str, udid: &str, body: &Value) -> anyhow::Result<()> {
    service_action(server_url, udid, body).map(|_| ())
}

fn service_action(server_url: &str, udid: &str, body: &Value) -> anyhow::Result<Value> {
    let path = format!("/api/simulators/{}/action", url_path_component(udid));
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("action");
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        match http_request_json(server_url, "POST", &path, Some(body)) {
            Ok(value) => return Ok(value),
            Err(error)
                if Instant::now() < deadline
                    && service_post_error_is_retryable(action, &error.to_string()) =>
            {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(error) => return Err(error),
        }
    }
}

fn service_post_ok(server_url: &str, udid: &str, action: &str, body: &Value) -> anyhow::Result<()> {
    let path = format!("/api/simulators/{}/{}", url_path_component(udid), action);
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        match http_request_json(server_url, "POST", &path, Some(body)) {
            Ok(_) => return Ok(()),
            Err(error)
                if Instant::now() < deadline
                    && service_post_error_is_retryable(action, &error.to_string()) =>
            {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(error) => return Err(error),
        }
    }
}

fn service_post_error_is_retryable(action: &str, message: &str) -> bool {
    if !matches!(
        action,
        "boot" | "shutdown" | "erase" | "launch" | "open-url" | "openUrl"
    ) {
        return false;
    }
    let message = message.to_lowercase();
    message.contains("resource temporarily unavailable")
        || message.contains("connection reset by peer")
        || message.contains("broken pipe")
        || message.contains("unexpected eof")
        || message.contains("timed out")
}
