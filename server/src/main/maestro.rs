fn run_maestro_flow(
    server_url: &str,
    udid: &str,
    flow: &Path,
    artifacts_dir: Option<PathBuf>,
    continue_on_error: bool,
) -> anyhow::Result<Value> {
    let raw = fs::read_to_string(flow)
        .with_context(|| format!("read Maestro flow {}", flow.display()))?;
    let yaml = parse_maestro_flow_yaml(&raw)
        .with_context(|| format!("parse Maestro flow {}", flow.display()))?;
    let commands = maestro_commands_from_flow(&yaml)?;
    let artifact_root = artifacts_dir.unwrap_or_else(|| {
        PathBuf::from("simdeck-artifacts").join(
            flow.file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("maestro-flow"),
        )
    });
    fs::create_dir_all(&artifact_root)?;

    let mut steps = Vec::new();
    let mut failures = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        let started = Instant::now();
        let result = run_maestro_command(server_url, udid, command, &artifact_root);
        match result {
            Ok(detail) => steps.push(serde_json::json!({
                "index": index,
                "ok": true,
                "command": maestro_command_name(command),
                "elapsedMs": started.elapsed().as_millis() as u64,
                "detail": detail,
            })),
            Err(error) => {
                let message = error.to_string();
                let screenshot =
                    capture_maestro_failure_screenshot(server_url, udid, &artifact_root, index + 1)
                        .ok();
                steps.push(serde_json::json!({
                    "index": index,
                    "ok": false,
                    "command": maestro_command_name(command),
                    "elapsedMs": started.elapsed().as_millis() as u64,
                    "error": message,
                    "screenshot": screenshot,
                }));
                failures.push(message);
                if !continue_on_error {
                    break;
                }
            }
        }
    }

    Ok(serde_json::json!({
        "ok": failures.is_empty(),
        "flow": flow,
        "udid": udid,
        "steps": steps,
        "failureCount": failures.len(),
        "artifactsDir": artifact_root,
    }))
}

fn parse_maestro_flow_yaml(raw: &str) -> anyhow::Result<YamlValue> {
    let mut documents = Vec::new();
    for document in serde_yaml::Deserializer::from_str(raw) {
        documents.push(YamlValue::deserialize(document)?);
    }
    match documents.len() {
        0 => Err(anyhow::anyhow!("Maestro flow is empty.")),
        1 => Ok(documents.remove(0)),
        _ => {
            let app_id = documents
                .first()
                .and_then(|value| yaml_string_or_field(value, "appId"));
            let mut commands = documents
                .pop()
                .ok_or_else(|| anyhow::anyhow!("Maestro flow is empty."))?;
            if let Some(app_id) = app_id {
                fill_empty_launch_app_commands(&mut commands, &app_id);
            }
            Ok(commands)
        }
    }
}

fn fill_empty_launch_app_commands(commands: &mut YamlValue, app_id: &str) {
    let Some(commands) = commands.as_sequence_mut() else {
        return;
    };
    for command in commands {
        if command.as_str() == Some("launchApp") {
            let mut mapping = serde_yaml::Mapping::new();
            mapping.insert(
                YamlValue::String("launchApp".to_owned()),
                YamlValue::String(app_id.to_owned()),
            );
            *command = YamlValue::Mapping(mapping);
            continue;
        }
        let Some(mapping) = command.as_mapping_mut() else {
            continue;
        };
        let key = YamlValue::String("launchApp".to_owned());
        let Some(value) = mapping.get_mut(&key) else {
            continue;
        };
        if value.is_null() || value.as_mapping().is_some_and(|mapping| mapping.is_empty()) {
            *value = YamlValue::String(app_id.to_owned());
        }
    }
}

fn maestro_commands_from_flow(flow: &YamlValue) -> anyhow::Result<Vec<YamlValue>> {
    match flow {
        YamlValue::Sequence(commands) => Ok(commands.clone()),
        YamlValue::Mapping(mapping) => mapping
            .get(YamlValue::String("commands".to_owned()))
            .and_then(YamlValue::as_sequence)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("Maestro flow must be a command list or contain `commands`.")
            }),
        _ => Err(anyhow::anyhow!(
            "Maestro flow must be a command list or contain `commands`."
        )),
    }
}

fn run_maestro_command(
    server_url: &str,
    udid: &str,
    command: &YamlValue,
    artifacts_dir: &Path,
) -> anyhow::Result<Value> {
    let null_value = YamlValue::Null;
    let (name, value) = if let Some(name) = command.as_str() {
        (name, &null_value)
    } else {
        let Some(mapping) = command.as_mapping() else {
            anyhow::bail!("Maestro command must be a string or mapping.");
        };
        if mapping.len() != 1 {
            anyhow::bail!("Maestro command must contain exactly one action.");
        }
        let (name, value) = mapping.iter().next().unwrap();
        (
            name.as_str()
                .ok_or_else(|| anyhow::anyhow!("Maestro command name must be a string."))?,
            value,
        )
    };
    match name {
        "launchApp" => {
            let bundle_id = maestro_bundle_id(value)?;
            service_launch(server_url, udid, &bundle_id)?;
            Ok(serde_json::json!({ "bundleId": bundle_id }))
        }
        "openLink" => {
            let url = yaml_string_or_field(value, "link")
                .or_else(|| yaml_string_or_field(value, "url"))
                .ok_or_else(|| anyhow::anyhow!("openLink requires a URL."))?;
            service_open_url(server_url, udid, &url)?;
            Ok(serde_json::json!({ "url": url }))
        }
        "tapOn" => {
            let body = maestro_tap_body(value)?;
            service_tap_element(server_url, udid, body)?;
            Ok(Value::Null)
        }
        "inputText" => {
            let text = yaml_string_or_field(value, "text")
                .ok_or_else(|| anyhow::anyhow!("inputText requires text."))?;
            service_action_ok(
                server_url,
                udid,
                &serde_json::json!({ "action": "type", "text": text }),
            )?;
            Ok(Value::Null)
        }
        "eraseText" => {
            let count = yaml_u64_or_field(value, "charactersToErase").unwrap_or(64);
            let keys = vec![42u16; count as usize];
            service_key_sequence(server_url, udid, &keys, 5)?;
            Ok(serde_json::json!({ "charactersToErase": count }))
        }
        "pressKey" => {
            let key = yaml_string_or_field(value, "key")
                .ok_or_else(|| anyhow::anyhow!("pressKey requires a key."))?;
            service_key(server_url, udid, parse_hid_key(&key)?, 0)?;
            Ok(serde_json::json!({ "key": key }))
        }
        "assertVisible" => {
            let selector = maestro_selector(value)?;
            service_wait_for(server_url, udid, "assert", selector, 5_000)?;
            Ok(Value::Null)
        }
        "assertNotVisible" => {
            let selector = maestro_selector(value)?;
            service_wait_for(server_url, udid, "assert-not", selector, 5_000)?;
            Ok(Value::Null)
        }
        "scrollUntilVisible" => {
            let selector_value = yaml_field(value, "element").unwrap_or(value);
            let selector = maestro_selector(selector_value)?;
            let direction =
                yaml_string_or_field(value, "direction").unwrap_or_else(|| "down".to_owned());
            service_action_ok(
                server_url,
                udid,
                &serde_json::json!({
                    "action": "scrollUntilVisible",
                    "selector": selector,
                    "direction": direction,
                    "timeoutMs": yaml_u64_or_field(value, "timeout").unwrap_or(10_000),
                }),
            )?;
            Ok(Value::Null)
        }
        "swipe" => {
            let direction =
                yaml_string_or_field(value, "direction").unwrap_or_else(|| "up".to_owned());
            let preset = match direction.to_ascii_lowercase().as_str() {
                "up" => "scroll-up",
                "down" => "scroll-down",
                "left" => "scroll-left",
                "right" => "scroll-right",
                _ => anyhow::bail!("Unsupported Maestro swipe direction `{direction}`."),
            };
            service_action_ok(
                server_url,
                udid,
                &serde_json::json!({ "action": "gesture", "preset": preset }),
            )?;
            Ok(serde_json::json!({ "direction": direction }))
        }
        "takeScreenshot" => {
            let name = yaml_string_or_field(value, "path")
                .or_else(|| yaml_string_or_field(value, "name"))
                .unwrap_or_else(|| "screenshot".to_owned());
            let path = artifacts_dir.join(format!("{}.png", name.trim_end_matches(".png")));
            let png = service_get_bytes(
                server_url,
                &format!(
                    "/api/simulators/{}/screenshot.png",
                    url_path_component(udid)
                ),
            )?;
            fs::write(&path, png)?;
            Ok(serde_json::json!({ "path": path }))
        }
        "waitForAnimationToEnd" | "waitForAnimationToEnd:" => {
            sleep_ms(
                yaml_u64_or_field(value, "timeout")
                    .unwrap_or(1_000)
                    .min(10_000),
            );
            Ok(Value::Null)
        }
        other => Err(anyhow::anyhow!(
            "Unsupported Maestro command `{other}` in this compatibility runner."
        )),
    }
}

fn maestro_command_name(command: &YamlValue) -> String {
    if let Some(name) = command.as_str() {
        return name.to_owned();
    }
    command
        .as_mapping()
        .and_then(|mapping| mapping.keys().next())
        .and_then(YamlValue::as_str)
        .unwrap_or("unknown")
        .to_owned()
}

fn maestro_bundle_id(value: &YamlValue) -> anyhow::Result<String> {
    yaml_string_or_field(value, "appId")
        .or_else(|| yaml_string_or_field(value, "bundleId"))
        .ok_or_else(|| anyhow::anyhow!("launchApp requires `appId` or `bundleId`."))
}

fn maestro_tap_body(value: &YamlValue) -> anyhow::Result<Value> {
    if let Some(point) = yaml_field(value, "point")
        .and_then(YamlValue::as_str)
        .map(str::to_owned)
    {
        let (x, y) = parse_maestro_point(&point)?;
        return Ok(serde_json::json!({ "x": x, "y": y, "normalized": true }));
    }
    Ok(serde_json::json!({
        "selector": maestro_selector(value)?,
        "waitTimeoutMs": yaml_u64_or_field(value, "timeout").unwrap_or(5_000),
    }))
}

fn maestro_selector(value: &YamlValue) -> anyhow::Result<Value> {
    if let Some(text) = value.as_str() {
        return Ok(serde_json::json!({ "text": text, "regex": true }));
    }
    let Some(mapping) = value.as_mapping() else {
        anyhow::bail!("Selector must be a string or mapping.");
    };
    let text = yaml_string_field(mapping, "text");
    let explicit_regex = yaml_bool_field(mapping, "regex");
    let use_regex = explicit_regex.unwrap_or_else(|| text.is_some());
    let id = yaml_string_field(mapping, "id").map(|id| {
        if use_regex && explicit_regex != Some(true) {
            anchored_regex_literal(&id)
        } else {
            id
        }
    });
    Ok(serde_json::json!({
        "text": text,
        "id": id,
        "label": yaml_string_field(mapping, "label"),
        "value": yaml_string_field(mapping, "value"),
        "elementType": yaml_string_field(mapping, "type"),
        "index": yaml_u64_field(mapping, "index"),
        "enabled": yaml_bool_field(mapping, "enabled"),
        "checked": yaml_bool_field(mapping, "checked"),
        "focused": yaml_bool_field(mapping, "focused"),
        "selected": yaml_bool_field(mapping, "selected"),
        "regex": use_regex,
    }))
}

fn anchored_regex_literal(value: &str) -> String {
    format!("^{}$", regex::escape(value))
}

fn service_wait_for(
    server_url: &str,
    udid: &str,
    action: &str,
    selector: Value,
    timeout_ms: u64,
) -> anyhow::Result<()> {
    let action = match action {
        "assert" => "assert",
        "assert-not" | "wait-for-not" => "assertNot",
        "wait-for" => "waitFor",
        other => other,
    };
    service_action_ok(
        server_url,
        udid,
        &serde_json::json!({ "action": action, "selector": selector, "timeoutMs": timeout_ms }),
    )
}

fn parse_maestro_point(point: &str) -> anyhow::Result<(f64, f64)> {
    let (x, y) = point
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("point must be `x,y`."))?;
    let parse = |value: &str| -> anyhow::Result<f64> {
        let value = value.trim();
        if let Some(percent) = value.strip_suffix('%') {
            Ok(percent.parse::<f64>()? / 100.0)
        } else {
            Ok(value.parse::<f64>()?)
        }
    };
    Ok((parse(x)?, parse(y)?))
}

fn capture_maestro_failure_screenshot(
    server_url: &str,
    udid: &str,
    artifacts_dir: &Path,
    step: usize,
) -> anyhow::Result<PathBuf> {
    let path = artifacts_dir.join(format!("failure-step-{step}.png"));
    let png = service_get_bytes(
        server_url,
        &format!(
            "/api/simulators/{}/screenshot.png",
            url_path_component(udid)
        ),
    )?;
    fs::write(&path, png)?;
    Ok(path)
}

fn yaml_field<'a>(value: &'a YamlValue, field: &str) -> Option<&'a YamlValue> {
    value.as_mapping()?.get(YamlValue::String(field.to_owned()))
}

fn yaml_string_or_field(value: &YamlValue, field: &str) -> Option<String> {
    value.as_str().map(str::to_owned).or_else(|| {
        yaml_field(value, field)
            .and_then(YamlValue::as_str)
            .map(str::to_owned)
    })
}

fn yaml_u64_or_field(value: &YamlValue, field: &str) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| yaml_field(value, field).and_then(YamlValue::as_u64))
}

fn yaml_string_field(mapping: &serde_yaml::Mapping, field: &str) -> Option<String> {
    mapping
        .get(YamlValue::String(field.to_owned()))
        .and_then(YamlValue::as_str)
        .map(str::to_owned)
}

fn yaml_u64_field(mapping: &serde_yaml::Mapping, field: &str) -> Option<u64> {
    mapping
        .get(YamlValue::String(field.to_owned()))
        .and_then(YamlValue::as_u64)
}

fn yaml_bool_field(mapping: &serde_yaml::Mapping, field: &str) -> Option<bool> {
    mapping
        .get(YamlValue::String(field.to_owned()))
        .and_then(YamlValue::as_bool)
}
