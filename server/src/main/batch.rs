fn run_batch(
    bridge: &NativeBridge,
    udid: &str,
    steps: Vec<String>,
    file: Option<PathBuf>,
    use_stdin: bool,
    continue_on_error: bool,
) -> anyhow::Result<Value> {
    let step_lines = read_batch_steps(steps, file, use_stdin)?;
    let mut results = Vec::new();
    let mut failures = Vec::new();
    for (index, line) in step_lines.iter().enumerate() {
        let result = run_batch_step(bridge, udid, line);
        match result {
            Ok(action) => {
                results.push(serde_json::json!({ "index": index, "ok": true, "action": action }))
            }
            Err(error) => {
                let message = error.to_string();
                results.push(serde_json::json!({ "index": index, "ok": false, "error": message }));
                failures.push(message);
                if !continue_on_error {
                    return Err(crate::error::AppError::bad_request(format!(
                        "Batch step {} failed: {}",
                        index + 1,
                        failures.last().unwrap()
                    ))
                    .into());
                }
            }
        }
    }
    Ok(serde_json::json!({
        "ok": failures.is_empty(),
        "steps": results,
        "failureCount": failures.len()
    }))
}

fn read_batch_steps(
    steps: Vec<String>,
    file: Option<PathBuf>,
    use_stdin: bool,
) -> anyhow::Result<Vec<String>> {
    let source_count =
        usize::from(!steps.is_empty()) + usize::from(file.is_some()) + usize::from(use_stdin);
    if source_count != 1 {
        return Err(crate::error::AppError::bad_request(
            "Specify exactly one batch source: --step, --file, or --stdin.",
        )
        .into());
    }
    let raw = if use_stdin {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else if let Some(file) = file {
        fs::read_to_string(file)?
    } else {
        return Ok(steps);
    };
    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

fn batch_lines_to_json_steps(step_lines: &[String]) -> anyhow::Result<Vec<Value>> {
    step_lines
        .iter()
        .map(|line| batch_line_to_json_step(line))
        .collect()
}

fn batch_line_to_json_step(line: &str) -> anyhow::Result<Value> {
    let tokens = tokenize_step(line)?;
    let Some(command) = tokens.first().map(String::as_str) else {
        return Err(crate::error::AppError::bad_request("Empty batch step.").into());
    };
    let args = parse_step_options(&tokens[1..]);
    let value = match command {
        "sleep" => serde_json::json!({
            "action": "sleep",
            "ms": parse_batch_sleep_duration_ms(&args, tokens.get(1).map(String::as_str))?,
        }),
        "tap" => {
            let mut step = serde_json::json!({
                "action": "tap",
                "x": args.value("x").and_then(|value| value.parse::<f64>().ok()),
                "y": args.value("y").and_then(|value| value.parse::<f64>().ok()),
                "normalized": args.flag("normalized"),
                "selector": batch_selector_json(&args),
                "durationMs": args.value("duration-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(60),
                "waitTimeoutMs": args.value("wait-timeout-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(0),
                "pollMs": args.value("poll-interval-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(100),
            });
            if let Some(expect) = batch_expectation_json(&args) {
                if let Some(object) = step.as_object_mut() {
                    object.insert("expect".to_owned(), expect);
                }
            }
            step
        }
        "back" => serde_json::json!({
            "action": "back",
            "timeoutMs": args.value("timeout-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(5_000),
            "pollMs": args.value("poll-interval-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(100),
            "fallbackSwipe": !args.flag("no-fallback-swipe"),
        }),
        "wait-for" | "waitFor" => serde_json::json!({
            "action": "waitFor",
            "selector": batch_selector_json(&args),
            "source": args.value("source"),
            "maxDepth": args.value("max-depth").and_then(|value| value.parse::<usize>().ok()),
            "includeHidden": args.flag("include-hidden"),
            "timeoutMs": args.value("timeout-ms").or_else(|| args.value("wait-timeout-ms")).and_then(|value| value.parse::<u64>().ok()).unwrap_or(5_000),
            "pollMs": args.value("poll-interval-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(100),
        }),
        "assert" => serde_json::json!({
            "action": "assert",
            "selector": batch_selector_json(&args),
            "source": args.value("source"),
            "maxDepth": args.value("max-depth").and_then(|value| value.parse::<usize>().ok()),
            "includeHidden": args.flag("include-hidden"),
            "timeoutMs": args.value("timeout-ms").or_else(|| args.value("wait-timeout-ms")).and_then(|value| value.parse::<u64>().ok()).unwrap_or(5_000),
            "pollMs": args.value("poll-interval-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(100),
        }),
        "assert-not" | "assertNot" | "wait-for-not" | "waitForNot" => serde_json::json!({
            "action": "assertNot",
            "selector": batch_selector_json(&args),
            "source": args.value("source"),
            "maxDepth": args.value("max-depth").and_then(|value| value.parse::<usize>().ok()),
            "includeHidden": args.flag("include-hidden"),
            "timeoutMs": args.value("timeout-ms").or_else(|| args.value("wait-timeout-ms")).and_then(|value| value.parse::<u64>().ok()).unwrap_or(5_000),
            "pollMs": args.value("poll-interval-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(100),
        }),
        "scroll-until-visible" | "scrollUntilVisible" => serde_json::json!({
            "action": "scrollUntilVisible",
            "selector": batch_selector_json(&args),
            "source": args.value("source"),
            "maxDepth": args.value("max-depth").and_then(|value| value.parse::<usize>().ok()),
            "includeHidden": args.flag("include-hidden"),
            "timeoutMs": args.value("timeout-ms").or_else(|| args.value("wait-timeout-ms")).and_then(|value| value.parse::<u64>().ok()).unwrap_or(10_000),
            "pollMs": args.value("poll-interval-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(100),
            "direction": args.value("direction").unwrap_or("down"),
        }),
        "key" => serde_json::json!({
            "action": "key",
            "keyCode": parse_hid_key(tokens.get(1).map(String::as_str).unwrap_or(""))?,
            "modifiers": args.value("modifiers").and_then(|value| value.parse::<u32>().ok()).unwrap_or(0),
        }),
        "key-sequence" => serde_json::json!({
            "action": "keySequence",
            "keyCodes": parse_key_list(args.value("keycodes").or_else(|| args.value("keys")).unwrap_or(""))?,
            "delayMs": args.value("delay-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(0),
        }),
        "key-combo" => serde_json::json!({
            "action": "key",
            "keyCode": parse_hid_key(args.value("key").unwrap_or(""))?,
            "modifiers": parse_modifier_mask(args.value("modifiers").unwrap_or(""))?,
        }),
        "touch" => {
            let x = args
                .value("x")
                .or_else(|| tokens.get(1).map(String::as_str))
                .and_then(|value| value.parse::<f64>().ok());
            let y = args
                .value("y")
                .or_else(|| tokens.get(2).map(String::as_str))
                .and_then(|value| value.parse::<f64>().ok());
            serde_json::json!({
                "action": "touch",
                "x": x.unwrap_or(0.0),
                "y": y.unwrap_or(0.0),
                "phase": args.value("phase").unwrap_or("began"),
                "down": args.flag("down"),
                "up": args.flag("up"),
                "delayMs": args.value("delay-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(100),
            })
        }
        "swipe" => {
            let value = |name: &str, index: usize| {
                args.value(name)
                    .or_else(|| tokens.get(index).map(String::as_str))
                    .and_then(|value| value.parse::<f64>().ok())
            };
            serde_json::json!({
                "action": "swipe",
                "startX": value("start-x", 1).unwrap_or(0.5),
                "startY": value("start-y", 2).unwrap_or(0.75),
                "endX": value("end-x", 3).unwrap_or(0.5),
                "endY": value("end-y", 4).unwrap_or(0.25),
                "durationMs": args.value("duration-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(350),
                "steps": args.value("steps").and_then(|value| value.parse::<u32>().ok()).unwrap_or(12),
            })
        }
        "gesture" => serde_json::json!({
            "action": "gesture",
            "preset": tokens.get(1).map(String::as_str).unwrap_or("scroll-down"),
            "durationMs": args.value("duration-ms").and_then(|value| value.parse::<u64>().ok()),
            "delta": args.value("delta").and_then(|value| value.parse::<f64>().ok()),
            "steps": args.value("steps").and_then(|value| value.parse::<u32>().ok()).unwrap_or(12),
        }),
        "type" => serde_json::json!({
            "action": "type",
            "text": tokens.get(1).map(String::as_str).unwrap_or(""),
            "delayMs": args.value("delay-ms").and_then(|value| value.parse::<u64>().ok()).unwrap_or(12),
        }),
        "button" => serde_json::json!({
            "action": "button",
            "button": tokens.get(1).map(String::as_str).unwrap_or(""),
            "durationMs": args.value("duration-ms").and_then(|value| value.parse::<u32>().ok()).unwrap_or(0),
        }),
        "crown" => serde_json::json!({
            "action": "crown",
            "delta": args.value("delta").and_then(|value| value.parse::<f64>().ok()).unwrap_or(50.0),
        }),
        "home" => serde_json::json!({ "action": "home" }),
        "dismiss-keyboard" => serde_json::json!({ "action": "dismissKeyboard" }),
        "app-switcher" => serde_json::json!({ "action": "appSwitcher" }),
        "rotate-left" => serde_json::json!({ "action": "rotateLeft" }),
        "rotate-right" => serde_json::json!({ "action": "rotateRight" }),
        "toggle-appearance" => serde_json::json!({ "action": "toggleAppearance" }),
        "launch" => serde_json::json!({
            "action": "launch",
            "bundleId": tokens.get(1).map(String::as_str).unwrap_or(""),
        }),
        "open-url" => serde_json::json!({
            "action": "openUrl",
            "url": tokens.get(1).map(String::as_str).unwrap_or(""),
        }),
        other => {
            return Err(crate::error::AppError::bad_request(format!(
                "Unsupported daemon batch step `{other}`."
            ))
            .into())
        }
    };
    Ok(value)
}

fn run_batch_step(
    bridge: &NativeBridge,
    udid: &str,
    line: &str,
) -> Result<&'static str, crate::error::AppError> {
    let tokens = tokenize_step(line)?;
    let Some(command) = tokens.first().map(String::as_str) else {
        return Err(crate::error::AppError::bad_request("Empty batch step."));
    };
    match command {
        "sleep" => {
            let args = parse_step_options(&tokens[1..]);
            let duration_ms = parse_batch_sleep_duration_ms(
                &args,
                tokens
                    .iter()
                    .skip(1)
                    .find(|token| !token.starts_with('-'))
                    .map(String::as_str),
            )?;
            sleep_ms(duration_ms);
            Ok("sleep")
        }
        "tap" => {
            let args = parse_step_options(&tokens[1..]);
            let x = args.value("x").and_then(|value| value.parse::<f64>().ok());
            let y = args.value("y").and_then(|value| value.parse::<f64>().ok());
            let normalized = args.flag("normalized");
            let duration_ms = args
                .value("duration-ms")
                .and_then(|value| value.parse().ok())
                .unwrap_or(60);
            let target = resolve_tap_target(
                bridge,
                udid,
                TapTargetRequest {
                    x,
                    y,
                    normalized,
                    selector: batch_selector_from_args(&args),
                    wait_timeout_ms: args
                        .value("wait-timeout-ms")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
                    poll_interval_ms: args
                        .value("poll-interval-ms")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(100),
                },
            )?;
            if bridge_simulator_is_tvos(bridge, udid) {
                press_tvos_remote_key(bridge, udid, HID_KEY_ENTER)?;
            } else if let Some(input) = target.input.as_ref() {
                perform_tap_with_input(input, target.x, target.y, duration_ms)?;
            } else {
                perform_tap(bridge, udid, target.x, target.y, duration_ms)?;
            }
            Ok("tap")
        }
        "back" => {
            perform_swipe(
                bridge,
                udid,
                GestureCoordinates {
                    start_x: 0.02,
                    start_y: 0.5,
                    end_x: 0.85,
                    end_y: 0.5,
                    duration_ms: 350,
                },
                12,
            )?;
            Ok("back")
        }
        "wait-for" | "waitFor" => {
            let args = parse_step_options(&tokens[1..]);
            wait_for_batch_selector(bridge, udid, &args)?;
            Ok("wait-for")
        }
        "assert" => {
            let args = parse_step_options(&tokens[1..]);
            wait_for_batch_selector(bridge, udid, &args)?;
            Ok("assert")
        }
        "swipe" => {
            let args = parse_step_options(&tokens[1..]);
            let start_x = required_f64(&args, "start-x")?;
            let start_y = required_f64(&args, "start-y")?;
            let end_x = required_f64(&args, "end-x")?;
            let end_y = required_f64(&args, "end-y")?;
            let normalized = args.flag("normalized");
            let (start_x, start_y) =
                resolve_touch_point(bridge, udid, start_x, start_y, normalized)?;
            let (end_x, end_y) = resolve_touch_point(bridge, udid, end_x, end_y, normalized)?;
            perform_swipe(
                bridge,
                udid,
                GestureCoordinates {
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                    duration_ms: args
                        .value("duration-ms")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(350),
                },
                args.value("steps")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(12),
            )?;
            Ok("swipe")
        }
        "gesture" => {
            let preset = tokens
                .get(1)
                .ok_or_else(|| crate::error::AppError::bad_request("gesture requires a preset."))?;
            let args = parse_step_options(&tokens[2..]);
            let gesture = gesture_coordinates(
                bridge,
                udid,
                preset,
                args.value("screen-width")
                    .and_then(|value| value.parse().ok()),
                args.value("screen-height")
                    .and_then(|value| value.parse().ok()),
                args.flag("normalized"),
                args.value("delta").and_then(|value| value.parse().ok()),
            )?;
            perform_swipe(
                bridge,
                udid,
                GestureCoordinates {
                    duration_ms: args
                        .value("duration-ms")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(gesture.duration_ms),
                    ..gesture
                },
                12,
            )?;
            Ok("gesture")
        }
        "pinch" => {
            let args = parse_step_options(&tokens[1..]);
            let frames = pinch_frames(
                bridge,
                udid,
                args.value("center-x").and_then(|value| value.parse().ok()),
                args.value("center-y").and_then(|value| value.parse().ok()),
                args.value("start-distance")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(160.0),
                args.value("end-distance")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(80.0),
                args.value("angle-degrees")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0.0),
                args.flag("normalized"),
                args.value("steps")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(12),
            )?;
            run_multitouch_frames(
                bridge,
                udid,
                frames,
                args.value("duration-ms")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(450),
            )?;
            Ok("pinch")
        }
        "rotate-gesture" => {
            let args = parse_step_options(&tokens[1..]);
            let frames = rotate_gesture_frames(
                bridge,
                udid,
                RotateGestureRequest {
                    center_x: args.value("center-x").and_then(|value| value.parse().ok()),
                    center_y: args.value("center-y").and_then(|value| value.parse().ok()),
                    radius: args
                        .value("radius")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(100.0),
                    degrees: args
                        .value("degrees")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(90.0),
                    normalized: args.flag("normalized"),
                    steps: args
                        .value("steps")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(12),
                },
            )?;
            run_multitouch_frames(
                bridge,
                udid,
                frames,
                args.value("duration-ms")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(500),
            )?;
            Ok("rotate-gesture")
        }
        "touch" => {
            let args = parse_step_options(&tokens[1..]);
            let x = required_f64(&args, "x")?;
            let y = required_f64(&args, "y")?;
            let normalized = args.flag("normalized");
            if bridge_simulator_is_tvos(bridge, udid) {
                perform_tvos_touch_command(
                    bridge,
                    udid,
                    args.value("phase").unwrap_or("began"),
                    args.flag("down"),
                    args.flag("up"),
                )?;
            } else {
                let (x, y) = resolve_touch_point(bridge, udid, x, y, normalized)?;
                if args.flag("down") || args.flag("up") {
                    let input = bridge.create_input_session(udid)?;
                    if args.flag("down") {
                        input.send_touch(x, y, "began")?;
                    }
                    if args.flag("down") && args.flag("up") {
                        sleep_ms(
                            args.value("delay-ms")
                                .and_then(|value| value.parse().ok())
                                .unwrap_or(100),
                        );
                    }
                    if args.flag("up") {
                        input.send_touch(x, y, "ended")?;
                    }
                } else {
                    bridge.send_touch(udid, x, y, args.value("phase").unwrap_or("began"))?;
                }
            }
            Ok("touch")
        }
        "type" => {
            let text = tokens.get(1).cloned().unwrap_or_default();
            type_text(bridge, udid, &text, 12)?;
            Ok("type")
        }
        "button" => {
            let button = tokens
                .get(1)
                .ok_or_else(|| crate::error::AppError::bad_request("button requires a name."))?;
            bridge.press_button(udid, button, 0)?;
            Ok("button")
        }
        "crown" => {
            let delta = tokens
                .get(1)
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(50.0);
            bridge.rotate_crown(udid, delta)?;
            Ok("crown")
        }
        "key" => {
            let key = tokens.get(1).ok_or_else(|| {
                crate::error::AppError::bad_request("key requires a keycode or key name.")
            })?;
            bridge.send_key(udid, parse_hid_key(key)?, 0)?;
            Ok("key")
        }
        "key-sequence" => {
            let args = parse_step_options(&tokens[1..]);
            let keys = parse_key_list(
                args.value("keycodes")
                    .or_else(|| args.value("keys"))
                    .ok_or_else(|| {
                        crate::error::AppError::bad_request("key-sequence requires --keycodes.")
                    })?,
            )?;
            let input = bridge.create_input_session(udid)?;
            for (index, key) in keys.iter().enumerate() {
                input.send_key(*key, 0)?;
                if index + 1 < keys.len() {
                    sleep_ms(
                        args.value("delay-ms")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(100),
                    );
                }
            }
            Ok("key-sequence")
        }
        "key-combo" => {
            let args = parse_step_options(&tokens[1..]);
            let modifiers = args.value("modifiers").ok_or_else(|| {
                crate::error::AppError::bad_request("key-combo requires --modifiers.")
            })?;
            let key = args
                .value("key")
                .ok_or_else(|| crate::error::AppError::bad_request("key-combo requires --key."))?;
            bridge.send_key(udid, parse_hid_key(key)?, parse_modifier_mask(modifiers)?)?;
            Ok("key-combo")
        }
        _ => Err(crate::error::AppError::bad_request(format!(
            "Unsupported batch step `{command}`."
        ))),
    }
}

#[derive(Default)]
struct StepOptions {
    values: Vec<(String, String)>,
    flags: Vec<String>,
}

impl StepOptions {
    fn value(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    }

    fn flag(&self, key: &str) -> bool {
        self.flags.iter().any(|candidate| candidate == key)
    }
}

fn parse_step_options(tokens: &[String]) -> StepOptions {
    let mut options = StepOptions::default();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if let Some(stripped) = token.strip_prefix("--") {
            if let Some((key, value)) = stripped.split_once('=') {
                options.values.push((key.to_owned(), value.to_owned()));
            } else if index + 1 < tokens.len() && !tokens[index + 1].starts_with("--") {
                options
                    .values
                    .push((stripped.to_owned(), tokens[index + 1].clone()));
                index += 1;
            } else {
                options.flags.push(stripped.to_owned());
            }
        } else if let Some(stripped) = token.strip_prefix('-') {
            if index + 1 < tokens.len() && !tokens[index + 1].starts_with('-') {
                options
                    .values
                    .push((stripped.to_owned(), tokens[index + 1].clone()));
                index += 1;
            }
        }
        index += 1;
    }
    options
}

fn required_f64(args: &StepOptions, key: &str) -> Result<f64, crate::error::AppError> {
    args.value(key)
        .ok_or_else(|| crate::error::AppError::bad_request(format!("Missing --{key}.")))?
        .parse::<f64>()
        .map_err(|_| crate::error::AppError::bad_request(format!("--{key} must be numeric.")))
}

fn batch_selector_json(args: &StepOptions) -> Value {
    serde_json::json!({
        "text": args.value("text"),
        "id": args.value("id"),
        "label": args.value("label"),
        "value": args.value("value"),
        "elementType": args.value("element-type"),
        "index": args.value("index").and_then(|value| value.parse::<usize>().ok()),
        "enabled": args.value("enabled").and_then(parse_bool_value),
        "checked": args.value("checked").and_then(parse_bool_value),
        "focused": args.value("focused").and_then(parse_bool_value),
        "selected": args.value("selected").and_then(parse_bool_value),
        "regex": args.flag("regex"),
    })
}

fn batch_expectation_json(args: &StepOptions) -> Option<Value> {
    let has_expectation = args.value("expect-id").is_some()
        || args.value("expect-label").is_some()
        || args.value("expect-value").is_some()
        || args.value("expect-element-type").is_some()
        || args.value("expect-type").is_some()
        || args.value("expect-index").is_some();
    has_expectation.then(|| {
        serde_json::json!({
            "selector": {
                "id": args.value("expect-id"),
                "label": args.value("expect-label"),
                "value": args.value("expect-value"),
                "elementType": args
                    .value("expect-element-type")
                    .or_else(|| args.value("expect-type")),
                "index": args
                    .value("expect-index")
                    .and_then(|value| value.parse::<usize>().ok()),
            },
            "source": "native-ax",
            "maxDepth": args
                .value("expect-max-depth")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(8),
            "includeHidden": args.flag("expect-include-hidden"),
            "timeoutMs": args
                .value("expect-timeout-ms")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(5_000),
            "pollMs": args
                .value("poll-interval-ms")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(100),
        })
    })
}

fn batch_selector_from_args(args: &StepOptions) -> ElementSelector {
    ElementSelector {
        id: args.value("id").map(str::to_owned),
        label: args.value("label").map(str::to_owned),
        value: args.value("value").map(str::to_owned),
        element_type: args.value("element-type").map(str::to_owned),
        index: args
            .value("index")
            .and_then(|value| value.parse::<usize>().ok()),
    }
}

fn parse_bool_value(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn wait_for_batch_selector(
    bridge: &NativeBridge,
    udid: &str,
    args: &StepOptions,
) -> Result<(), crate::error::AppError> {
    let selector = batch_selector_from_args(args);
    if selector.id.is_none()
        && selector.label.is_none()
        && selector.value.is_none()
        && selector.element_type.is_none()
    {
        return Err(crate::error::AppError::bad_request(
            "wait-for/assert requires a selector flag.",
        ));
    }

    let timeout_ms = args
        .value("timeout-ms")
        .or_else(|| args.value("wait-timeout-ms"))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(5_000);
    let poll_interval_ms = args
        .value("poll-interval-ms")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(100)
        .max(10);
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let snapshot = bridge.accessibility_snapshot(udid, None)?;
        if snapshot_contains_element(&snapshot, &selector) {
            return Ok(());
        }
        if timeout_ms == 0 || std::time::Instant::now() >= deadline {
            return Err(crate::error::AppError::not_found(
                "No accessibility element matched the selector.",
            ));
        }
        sleep_ms(poll_interval_ms);
    }
}

fn snapshot_contains_element(snapshot: &Value, selector: &ElementSelector) -> bool {
    snapshot
        .get("roots")
        .and_then(Value::as_array)
        .map(|roots| {
            roots
                .iter()
                .any(|root| node_contains_matching_element(root, selector))
        })
        .unwrap_or(false)
}

fn node_contains_matching_element(node: &Value, selector: &ElementSelector) -> bool {
    element_matches(node, selector)
        || node
            .get("children")
            .and_then(Value::as_array)
            .map(|children| {
                children
                    .iter()
                    .any(|child| node_contains_matching_element(child, selector))
            })
            .unwrap_or(false)
}

fn parse_batch_sleep_duration_ms(
    args: &StepOptions,
    positional: Option<&str>,
) -> Result<u64, crate::error::AppError> {
    if let Some(value) = args
        .value("ms")
        .or_else(|| args.value("milliseconds"))
        .or_else(|| args.value("duration-ms"))
    {
        return parse_duration_ms_value(value, "sleep --ms");
    }

    if let Some(value) = args.value("seconds").or_else(|| args.value("s")) {
        return parse_duration_seconds_value(value, "sleep --seconds");
    }

    let Some(value) = positional else {
        return Err(crate::error::AppError::bad_request(
            "sleep requires a duration, for example `sleep 500`, `sleep 500ms`, or `sleep 0.5s`.",
        ));
    };

    parse_duration_literal_ms(value)
}

fn parse_duration_literal_ms(value: &str) -> Result<u64, crate::error::AppError> {
    let value = value.trim();
    if let Some(ms) = value.strip_suffix("ms") {
        return parse_duration_ms_value(ms, "sleep duration");
    }
    if let Some(seconds) = value.strip_suffix('s') {
        return parse_duration_seconds_value(seconds, "sleep duration");
    }
    parse_duration_ms_value(value, "sleep duration")
}

fn parse_duration_ms_value(value: &str, context: &str) -> Result<u64, crate::error::AppError> {
    let duration = value
        .trim()
        .parse::<f64>()
        .map_err(|_| crate::error::AppError::bad_request(format!("{context} must be numeric.")))?;
    finite_non_negative_duration_ms(duration, 1.0, context)
}

fn parse_duration_seconds_value(value: &str, context: &str) -> Result<u64, crate::error::AppError> {
    let duration = value
        .trim()
        .parse::<f64>()
        .map_err(|_| crate::error::AppError::bad_request(format!("{context} must be numeric.")))?;
    finite_non_negative_duration_ms(duration, 1000.0, context)
}

fn finite_non_negative_duration_ms(
    value: f64,
    multiplier: f64,
    context: &str,
) -> Result<u64, crate::error::AppError> {
    if !value.is_finite() || value < 0.0 {
        return Err(crate::error::AppError::bad_request(format!(
            "{context} must be finite and non-negative."
        )));
    }
    Ok((value * multiplier).round() as u64)
}

fn tokenize_step(line: &str) -> Result<Vec<String>, crate::error::AppError> {
    enum State {
        Normal,
        Single,
        Double,
    }
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut state = State::Normal;
    let mut escaping = false;
    let mut saw_boundary = false;
    for character in line.chars() {
        match state {
            State::Normal => {
                if escaping {
                    current.push(character);
                    escaping = false;
                    saw_boundary = true;
                } else if character == '\\' {
                    escaping = true;
                } else if character == '\'' {
                    state = State::Single;
                    saw_boundary = true;
                } else if character == '"' {
                    state = State::Double;
                    saw_boundary = true;
                } else if character.is_whitespace() {
                    if !current.is_empty() || saw_boundary {
                        tokens.push(std::mem::take(&mut current));
                        saw_boundary = false;
                    }
                } else {
                    current.push(character);
                    saw_boundary = true;
                }
            }
            State::Single => {
                if character == '\'' {
                    state = State::Normal;
                } else {
                    current.push(character);
                }
            }
            State::Double => {
                if escaping {
                    current.push(character);
                    escaping = false;
                } else if character == '\\' {
                    escaping = true;
                } else if character == '"' {
                    state = State::Normal;
                } else {
                    current.push(character);
                }
            }
        }
    }
    if escaping {
        return Err(crate::error::AppError::bad_request(
            "Dangling escape in batch step.",
        ));
    }
    if !matches!(state, State::Normal) {
        return Err(crate::error::AppError::bad_request(
            "Unterminated quote in batch step.",
        ));
    }
    if !current.is_empty() || saw_boundary {
        tokens.push(current);
    }
    Ok(tokens)
}
