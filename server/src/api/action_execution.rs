async fn perform_tap_payload(
    state: AppState,
    udid: String,
    payload: TapElementPayload,
) -> Result<Option<Value>, AppError> {
    let duration_ms = payload.duration_ms.unwrap_or(20);
    let expect = payload.expect.as_deref().cloned();
    let (x, y) = if selector_is_empty(&payload.selector) {
        let x = payload
            .x
            .ok_or_else(|| AppError::bad_request("Tap requires `x` and `y` or a selector."))?;
        let y = payload
            .y
            .ok_or_else(|| AppError::bad_request("Tap requires `x` and `y` or a selector."))?;
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            return Err(AppError::bad_request(
                "Tap coordinates must be finite non-negative numbers.",
            ));
        }
        if payload.normalized.unwrap_or(true) {
            (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))
        } else {
            let snapshot = cached_accessibility_tree_value(
                state.clone(),
                udid.clone(),
                payload.source.as_deref(),
                payload.max_depth,
                payload.include_hidden.unwrap_or(false),
                false,
            )
            .await?;
            normalize_screen_point_from_snapshot(&snapshot, x, y)?
        }
    } else {
        let wait_payload = WaitForPayload {
            selector: payload.selector.clone(),
            source: payload.source.clone(),
            max_depth: payload.max_depth,
            include_hidden: payload.include_hidden,
            timeout_ms: payload.wait_timeout_ms,
            poll_ms: payload.poll_ms,
        };
        let cached_snapshot = state
            .accessibility_cache
            .latest_interactive(&udid)
            .filter(|snapshot| tap_point_from_snapshot(snapshot, &payload.selector).is_ok());
        let snapshot = if let Some(snapshot) = cached_snapshot {
            snapshot
        } else {
            wait_for_tap_snapshot_match(state.clone(), udid.clone(), wait_payload).await?
        };
        tap_point_from_snapshot(&snapshot, &payload.selector)?
    };

    let cache = state.accessibility_cache.clone();
    let cache_udid = udid.clone();
    cache.invalidate(&cache_udid);
    let action_state = state.clone();
    let action_udid = udid.clone();
    let result = if android::is_android_id(&udid) {
        run_android_action(action_state, move |android| {
            android.send_touch(&action_udid, x, y, "began")?;
            if duration_ms > 0 {
                std::thread::sleep(Duration::from_millis(duration_ms));
            }
            android.send_touch(&action_udid, x, y, "ended")
        })
        .await
    } else {
        run_bridge_action(action_state, move |bridge| {
            if bridge_simulator_is_tvos(&bridge, &action_udid) {
                return press_tvos_remote_key(&bridge, &action_udid, HID_KEY_ENTER);
            }
            let input = bridge.create_input_session(&action_udid)?;
            input.send_touch(x, y, "began")?;
            if duration_ms > 0 {
                std::thread::sleep(Duration::from_millis(duration_ms));
            }
            input.send_touch(x, y, "ended")
        })
        .await
    };
    result?;
    cache.invalidate(&cache_udid);

    if let Some(mut expect) = expect {
        if expect.source.is_none() {
            expect.source = Some(SOURCE_NATIVE_AX.to_owned());
        }
        if expect.max_depth.is_none() {
            expect.max_depth = Some(8);
        }
        let snapshot = wait_for_snapshot_match(state, udid, expect.clone()).await?;
        let found = first_matching_element(&snapshot, &expect.selector)
            .ok_or_else(|| AppError::not_found("No accessibility element matched."))?;
        return Ok(Some(compact_accessibility_node(&found)));
    }

    Ok(None)
}

async fn perform_back_payload(
    state: AppState,
    udid: String,
    timeout_ms: Option<u64>,
    poll_ms: Option<u64>,
    fallback_swipe: Option<bool>,
) -> Result<Value, AppError> {
    if android::is_android_id(&udid) {
        run_android_action(state, move |android| android.press_button(&udid, "back", 0)).await?;
        return Ok(json_value!({ "action": "back", "method": "androidButton" }));
    }

    let timeout_ms = timeout_ms.unwrap_or(5_000);
    let poll_ms = poll_ms.unwrap_or(100);
    let id_timeout_ms = timeout_ms.min(2_000);
    let id_result = perform_tap_payload(
        state.clone(),
        udid.clone(),
        TapElementPayload {
            selector: ElementSelectorPayload {
                id: Some("BackButton".to_owned()),
                ..Default::default()
            },
            source: Some(SOURCE_NATIVE_AX.to_owned()),
            max_depth: Some(8),
            include_hidden: Some(false),
            wait_timeout_ms: Some(id_timeout_ms),
            poll_ms: Some(poll_ms),
            duration_ms: Some(20),
            ..Default::default()
        },
    )
    .await;
    if id_result.is_ok() {
        return Ok(json_value!({ "action": "back", "method": "backButtonId" }));
    }

    let label_result = perform_tap_payload(
        state.clone(),
        udid.clone(),
        TapElementPayload {
            selector: ElementSelectorPayload {
                label: Some("Back".to_owned()),
                ..Default::default()
            },
            source: Some(SOURCE_NATIVE_AX.to_owned()),
            max_depth: Some(8),
            include_hidden: Some(false),
            wait_timeout_ms: Some(timeout_ms.saturating_sub(id_timeout_ms)),
            poll_ms: Some(poll_ms),
            duration_ms: Some(20),
            ..Default::default()
        },
    )
    .await;
    if label_result.is_ok() {
        return Ok(json_value!({ "action": "back", "method": "backLabel" }));
    }

    if fallback_swipe.unwrap_or(true) {
        let plan = ScrollInputPlan {
            backend: ScrollInputBackend::Native,
            swipe: NormalizedSwipe {
                start_x: 0.02,
                start_y: 0.5,
                end_x: 0.85,
                end_y: 0.5,
                duration_ms: 350,
                steps: 12,
            },
        };
        perform_scroll_input(state, udid, plan).await?;
        return Ok(json_value!({ "action": "back", "method": "edgeSwipe" }));
    }

    label_result.map(|_| json_value!({ "action": "back", "method": "backLabel" }))
}

async fn query_element_payload(
    state: AppState,
    udid: String,
    payload: AccessibilityQueryPayload,
) -> Result<Value, AppError> {
    let snapshot = cached_accessibility_tree_value(
        state,
        udid,
        payload.source.as_deref(),
        payload.max_depth,
        payload.include_hidden.unwrap_or(false),
        false,
    )
    .await?;
    let matches = query_compact_elements(
        &snapshot,
        &payload.selector,
        payload.limit.unwrap_or(64).clamp(1, 512),
    );
    Ok(json_value!({
        "action": "query",
        "ok": true,
        "source": snapshot.get("source").cloned().unwrap_or(Value::Null),
        "count": matches.len(),
        "matches": matches,
    }))
}

async fn wait_for_absent_element_payload(
    state: AppState,
    udid: String,
    payload: WaitForPayload,
) -> Result<Json<Value>, AppError> {
    let started = Instant::now();
    let timeout_ms = payload.timeout_ms.unwrap_or(5_000);
    let poll_ms = payload.poll_ms.unwrap_or(100).max(10);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let snapshot = refresh_accessibility_tree_value(
            state.clone(),
            udid.clone(),
            payload.source.as_deref(),
            payload.max_depth,
            payload.include_hidden.unwrap_or(false),
            false,
        )
        .await?;
        if first_matching_element(&snapshot, &payload.selector).is_none() {
            return Ok(json(json_value!({
                "ok": true,
                "elapsedMs": started.elapsed().as_millis() as u64,
            })));
        }
        if timeout_ms == 0 || Instant::now() >= deadline {
            return Err(AppError::bad_request(
                "Accessibility element still matched the selector.",
            ));
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
}

async fn wait_for_snapshot_match(
    state: AppState,
    udid: String,
    payload: WaitForPayload,
) -> Result<Value, AppError> {
    let timeout_ms = payload.timeout_ms.unwrap_or(5_000);
    let poll_ms = payload.poll_ms.unwrap_or(100).max(10);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let prefer_interactive = selector_prefers_interactive(&payload.selector)
        && payload.include_hidden != Some(true)
        && payload.selector.index.is_none()
        && payload
            .source
            .as_deref()
            .is_none_or(|source| source == "auto" || source == SOURCE_NATIVE_AX);
    let interactive_max_depth = payload.max_depth.or(Some(8));
    let mut prefer_cache = true;
    loop {
        let snapshot = if prefer_interactive {
            if prefer_cache {
                cached_accessibility_tree_value(
                    state.clone(),
                    udid.clone(),
                    payload.source.as_deref(),
                    interactive_max_depth,
                    false,
                    true,
                )
                .await?
            } else {
                refresh_accessibility_tree_value(
                    state.clone(),
                    udid.clone(),
                    payload.source.as_deref(),
                    interactive_max_depth,
                    false,
                    true,
                )
                .await?
            }
        } else if prefer_cache {
            cached_accessibility_tree_value(
                state.clone(),
                udid.clone(),
                payload.source.as_deref(),
                payload.max_depth,
                payload.include_hidden.unwrap_or(false),
                false,
            )
            .await?
        } else {
            refresh_accessibility_tree_value(
                state.clone(),
                udid.clone(),
                payload.source.as_deref(),
                payload.max_depth,
                payload.include_hidden.unwrap_or(false),
                false,
            )
            .await?
        };
        prefer_cache = false;
        if first_matching_element(&snapshot, &payload.selector).is_some() {
            return Ok(snapshot);
        }
        if timeout_ms == 0 || Instant::now() >= deadline {
            if prefer_interactive {
                let snapshot = refresh_accessibility_tree_value(
                    state,
                    udid,
                    payload.source.as_deref(),
                    payload.max_depth,
                    payload.include_hidden.unwrap_or(false),
                    false,
                )
                .await?;
                if first_matching_element(&snapshot, &payload.selector).is_some() {
                    return Ok(snapshot);
                }
            }
            return Err(AppError::not_found("No accessibility element matched."));
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
}

fn selector_prefers_interactive(selector: &ElementSelectorPayload) -> bool {
    selector.id.is_some()
        || selector.enabled.is_some()
        || selector.checked.is_some()
        || selector.focused.is_some()
        || selector.selected.is_some()
        || selector
            .element_type
            .as_deref()
            .is_some_and(selector_type_looks_interactive)
}

fn selector_type_looks_interactive(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "button",
        "cell",
        "checkbox",
        "control",
        "link",
        "search",
        "slider",
        "switch",
        "textfield",
        "text field",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

async fn wait_for_tap_snapshot_match(
    state: AppState,
    udid: String,
    payload: WaitForPayload,
) -> Result<Value, AppError> {
    let timeout_ms = payload.timeout_ms.unwrap_or(0);
    let poll_ms = payload.poll_ms.unwrap_or(100).max(10);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let allow_slow_fallback = payload.selector.index.is_none()
        && payload.include_hidden != Some(true)
        && payload
            .source
            .as_deref()
            .is_none_or(|source| source == "auto" || source == SOURCE_NATIVE_AX);
    let mut prefer_cache = true;
    loop {
        let snapshot = if prefer_cache {
            cached_accessibility_tree_value(
                state.clone(),
                udid.clone(),
                payload.source.as_deref(),
                payload.max_depth,
                payload.include_hidden.unwrap_or(false),
                true,
            )
            .await?
        } else {
            refresh_accessibility_tree_value(
                state.clone(),
                udid.clone(),
                payload.source.as_deref(),
                payload.max_depth,
                payload.include_hidden.unwrap_or(false),
                true,
            )
            .await?
        };
        prefer_cache = false;
        if first_matching_element(&snapshot, &payload.selector).is_some() {
            return Ok(snapshot);
        }
        let timed_out = timeout_ms == 0 || Instant::now() >= deadline;
        if timed_out {
            if allow_slow_fallback {
                let snapshot = refresh_accessibility_tree_value(
                    state,
                    udid,
                    payload.source.as_deref(),
                    payload.max_depth,
                    payload.include_hidden.unwrap_or(false),
                    false,
                )
                .await?;
                if first_matching_element(&snapshot, &payload.selector).is_some() {
                    return Ok(snapshot);
                }
            }
            return Err(AppError::not_found("No accessibility element matched."));
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
}

async fn scroll_until_visible_payload(
    state: AppState,
    udid: String,
    payload: ScrollUntilVisiblePayload,
) -> Result<Json<Value>, AppError> {
    let started = Instant::now();
    let timeout_ms = payload.timeout_ms.unwrap_or(10_000);
    let poll_ms = payload.poll_ms.unwrap_or(100).max(10);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut scroll_count = 0usize;
    loop {
        let snapshot = refresh_accessibility_tree_value(
            state.clone(),
            udid.clone(),
            payload.source.as_deref(),
            payload.max_depth,
            payload.include_hidden.unwrap_or(false),
            false,
        )
        .await?;
        if let Some(found) = first_matching_element(&snapshot, &payload.selector) {
            return Ok(json(json_value!({
                "ok": true,
                "elapsedMs": started.elapsed().as_millis() as u64,
                "scrollCount": scroll_count,
                "match": compact_accessibility_node(&found),
            })));
        }
        if timeout_ms == 0 || Instant::now() >= deadline {
            return Err(AppError::not_found("No accessibility element matched."));
        }
        let scroll_plan = scroll_input_plan_for_udid(&udid, &payload)?;
        perform_scroll_input(state.clone(), udid.clone(), scroll_plan).await?;
        state.accessibility_cache.invalidate(&udid);
        scroll_count += 1;
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
}

async fn run_batch_steps(
    state: AppState,
    udid: String,
    payload: BatchPayload,
) -> Result<Value, AppError> {
    if payload.steps.is_empty() {
        return Err(AppError::bad_request(
            "Batch action must include at least one step.",
        ));
    }
    if payload.steps.len() > 256 {
        return Err(AppError::bad_request(
            "Batch action cannot contain more than 256 steps.",
        ));
    }

    let continue_on_error = payload.continue_on_error.unwrap_or(false);
    let mut results = Vec::new();
    let mut failure_count = 0usize;
    let mut should_warm_after_batch = false;
    for (index, step) in payload.steps.into_iter().enumerate() {
        let invalidates_ax_cache = batch_step_invalidates_ax_cache(&step);
        let should_warm_ax = batch_step_should_warm_ax(&step);
        if invalidates_ax_cache {
            state.accessibility_cache.invalidate(&udid);
        }
        let started = Instant::now();
        let result = run_batch_step(state.clone(), udid.clone(), step).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(value) => {
                if should_warm_ax {
                    should_warm_after_batch = true;
                }
                results.push(json_value!({
                    "index": index,
                    "ok": true,
                    "elapsedMs": elapsed_ms,
                    "result": value,
                }));
            }
            Err(error) => {
                failure_count += 1;
                let message = error.to_string();
                results.push(json_value!({
                    "index": index,
                    "ok": false,
                    "elapsedMs": elapsed_ms,
                    "error": message,
                }));
                if !continue_on_error {
                    return Err(AppError::bad_request(format!(
                        "Batch step {} failed: {}",
                        index + 1,
                        message
                    )));
                }
            }
        }
    }
    if should_warm_after_batch {
        spawn_accessibility_warmup(state.clone(), udid.clone());
    }
    Ok(json_value!({
        "ok": failure_count == 0,
        "failureCount": failure_count,
        "steps": results,
    }))
}

fn batch_step_invalidates_ax_cache(step: &BatchStep) -> bool {
    !matches!(
        step,
        BatchStep::Sleep { .. }
            | BatchStep::Tap(_)
            | BatchStep::WaitFor(_)
            | BatchStep::Assert(_)
            | BatchStep::AssertNot(_)
            | BatchStep::Query(_)
            | BatchStep::ScrollUntilVisible(_)
            | BatchStep::Describe { .. }
    )
}

fn batch_step_should_warm_ax(step: &BatchStep) -> bool {
    !matches!(
        step,
        BatchStep::Sleep { .. }
            | BatchStep::WaitFor(_)
            | BatchStep::Assert(_)
            | BatchStep::AssertNot(_)
            | BatchStep::Query(_)
            | BatchStep::Launch { .. }
            | BatchStep::OpenUrl { .. }
            | BatchStep::Describe { .. }
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NormalizedSwipe {
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    duration_ms: u64,
    steps: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollInputBackend {
    Android,
    Native,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollInputPlan {
    backend: ScrollInputBackend,
    swipe: NormalizedSwipe,
}

fn scroll_input_plan_for_udid(
    udid: &str,
    payload: &ScrollUntilVisiblePayload,
) -> Result<ScrollInputPlan, AppError> {
    let (start_x, start_y, end_x, end_y) =
        normalized_scroll_coordinates(payload.direction.as_deref())?;
    Ok(ScrollInputPlan {
        backend: if android::is_android_id(udid) {
            ScrollInputBackend::Android
        } else {
            ScrollInputBackend::Native
        },
        swipe: NormalizedSwipe {
            start_x,
            start_y,
            end_x,
            end_y,
            duration_ms: payload.duration_ms.unwrap_or(350),
            steps: payload.steps.unwrap_or(12).max(1),
        },
    })
}

async fn perform_scroll_input(
    state: AppState,
    udid: String,
    plan: ScrollInputPlan,
) -> Result<(), AppError> {
    let swipe = plan.swipe;
    match plan.backend {
        ScrollInputBackend::Android => {
            run_android_action(state, move |android| {
                android.send_swipe(
                    &udid,
                    swipe.start_x,
                    swipe.start_y,
                    swipe.end_x,
                    swipe.end_y,
                    swipe.duration_ms,
                )
            })
            .await
        }
        ScrollInputBackend::Native => {
            run_bridge_action(state, move |bridge| {
                if bridge_simulator_is_tvos(&bridge, &udid) {
                    let key_code = tvos_remote_key_for_touch_motion(
                        swipe.start_x,
                        swipe.start_y,
                        swipe.end_x,
                        swipe.end_y,
                    );
                    return press_tvos_remote_key(&bridge, &udid, key_code);
                }
                let input = bridge.create_input_session(&udid)?;
                let delay = Duration::from_millis(swipe.duration_ms / u64::from(swipe.steps));
                input.send_touch(swipe.start_x, swipe.start_y, "began")?;
                for step in 1..swipe.steps {
                    let t = f64::from(step) / f64::from(swipe.steps);
                    input.send_touch(
                        swipe.start_x + (swipe.end_x - swipe.start_x) * t,
                        swipe.start_y + (swipe.end_y - swipe.start_y) * t,
                        "moved",
                    )?;
                    std::thread::sleep(delay);
                }
                input.send_touch(swipe.end_x, swipe.end_y, "ended")
            })
            .await
        }
    }
}

fn normalized_scroll_coordinates(
    direction: Option<&str>,
) -> Result<(f64, f64, f64, f64), AppError> {
    match direction.unwrap_or("down").to_ascii_lowercase().as_str() {
        "down" => Ok((0.5, 0.78, 0.5, 0.22)),
        "up" => Ok((0.5, 0.22, 0.5, 0.78)),
        "left" => Ok((0.78, 0.5, 0.22, 0.5)),
        "right" => Ok((0.22, 0.5, 0.78, 0.5)),
        other => Err(AppError::bad_request(format!(
            "Unsupported scroll direction `{other}`."
        ))),
    }
}

async fn run_batch_step(state: AppState, udid: String, step: BatchStep) -> Result<Value, AppError> {
    match step {
        BatchStep::Sleep { ms, seconds } => {
            let duration =
                ms.unwrap_or_else(|| ((seconds.unwrap_or(0.0) * 1000.0).max(0.0)) as u64);
            tokio::time::sleep(Duration::from_millis(duration)).await;
            Ok(json_value!({ "action": "sleep", "durationMs": duration }))
        }
        BatchStep::Tap(payload) => {
            let expected = perform_tap_payload(state, udid, payload).await?;
            let mut result = json_value!({ "action": "tap" });
            if let (Some(object), Some(expected)) = (result.as_object_mut(), expected) {
                object.insert("expectation".to_owned(), expected);
            }
            Ok(result)
        }
        BatchStep::Back {
            timeout_ms,
            poll_ms,
            fallback_swipe,
        } => {
            perform_back_payload(state, udid, timeout_ms, poll_ms, fallback_swipe).await
        }
        BatchStep::WaitFor(payload) => {
            let snapshot = wait_for_snapshot_match(state, udid, payload.clone()).await?;
            let found = first_matching_element(&snapshot, &payload.selector)
                .ok_or_else(|| AppError::not_found("No accessibility element matched."))?;
            Ok(json_value!({ "action": "waitFor", "match": compact_accessibility_node(&found) }))
        }
        BatchStep::Assert(payload) => {
            let snapshot = wait_for_snapshot_match(state, udid, payload.clone()).await?;
            let found = first_matching_element(&snapshot, &payload.selector)
                .ok_or_else(|| AppError::not_found("No accessibility element matched."))?;
            Ok(json_value!({ "action": "assert", "match": compact_accessibility_node(&found) }))
        }
        BatchStep::Query(payload) => query_element_payload(state, udid, payload).await,
        BatchStep::AssertNot(payload) => {
            let Json(_) = wait_for_absent_element_payload(state, udid, payload).await?;
            Ok(json_value!({ "action": "assertNot" }))
        }
        BatchStep::ScrollUntilVisible(payload) => {
            let Json(result) = scroll_until_visible_payload(state, udid, payload).await?;
            Ok(json_value!({
                "action": "scrollUntilVisible",
                "match": result.get("match").cloned().unwrap_or(Value::Null),
                "scrollCount": result.get("scrollCount").cloned().unwrap_or(Value::Null),
            }))
        }
        BatchStep::Key {
            key_code,
            modifiers,
        } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.send_key(&udid, key_code, modifiers.unwrap_or(0))
                })
                .await?;
                return Ok(json_value!({ "action": "key" }));
            }
            run_bridge_action(state, move |bridge| {
                bridge.send_key(&udid, key_code, modifiers.unwrap_or(0))
            })
            .await?;
            Ok(json_value!({ "action": "key" }))
        }
        BatchStep::KeySequence {
            key_codes,
            delay_ms,
        } => {
            if key_codes.is_empty() {
                return Err(AppError::bad_request("keySequence requires keyCodes."));
            }
            if key_codes.len() > 512 {
                return Err(AppError::bad_request(
                    "keySequence cannot contain more than 512 key codes.",
                ));
            }
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    let delay_ms = delay_ms.unwrap_or(0);
                    let key_count = key_codes.len();
                    for (index, key_code) in key_codes.into_iter().enumerate() {
                        android.send_key(&udid, key_code, 0)?;
                        if delay_ms > 0 && index + 1 < key_count {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                        }
                    }
                    Ok(())
                })
                .await?;
                return Ok(json_value!({ "action": "keySequence" }));
            }
            run_bridge_action(state, move |bridge| {
                let input = bridge.create_input_session(&udid)?;
                let delay_ms = delay_ms.unwrap_or(0);
                let key_count = key_codes.len();
                for (index, key_code) in key_codes.into_iter().enumerate() {
                    input.send_key(key_code, 0)?;
                    if delay_ms > 0 && index + 1 < key_count {
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
                Ok(())
            })
            .await?;
            Ok(json_value!({ "action": "keySequence" }))
        }
        BatchStep::Touch {
            x,
            y,
            phase,
            down,
            up,
            delay_ms,
        } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(AppError::bad_request(
                    "touch requires finite normalized x and y.",
                ));
            }
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    let x = x.clamp(0.0, 1.0);
                    let y = y.clamp(0.0, 1.0);
                    if down.unwrap_or(false) || up.unwrap_or(false) {
                        if down.unwrap_or(false) {
                            android.send_touch(&udid, x, y, "began")?;
                        }
                        if down.unwrap_or(false) && up.unwrap_or(false) {
                            std::thread::sleep(Duration::from_millis(delay_ms.unwrap_or(100)));
                        }
                        if up.unwrap_or(false) {
                            android.send_touch(&udid, x, y, "ended")?;
                        }
                    } else {
                        android.send_touch(&udid, x, y, phase.as_deref().unwrap_or("began"))?;
                    }
                    Ok(())
                })
                .await?;
                return Ok(json_value!({ "action": "touch" }));
            }
            run_bridge_action(state, move |bridge| {
                if bridge_simulator_is_tvos(&bridge, &udid) {
                    if down.unwrap_or(false) || up.unwrap_or(false) {
                        if up.unwrap_or(false) {
                            return press_tvos_remote_key(&bridge, &udid, HID_KEY_ENTER);
                        }
                        return Ok(());
                    }
                    return handle_tvos_touch_phase(
                        &bridge,
                        &udid,
                        phase.as_deref().unwrap_or("began"),
                    );
                }
                let input = bridge.create_input_session(&udid)?;
                let x = x.clamp(0.0, 1.0);
                let y = y.clamp(0.0, 1.0);
                if down.unwrap_or(false) || up.unwrap_or(false) {
                    if down.unwrap_or(false) {
                        input.send_touch(x, y, "began")?;
                    }
                    if down.unwrap_or(false) && up.unwrap_or(false) {
                        std::thread::sleep(Duration::from_millis(delay_ms.unwrap_or(100)));
                    }
                    if up.unwrap_or(false) {
                        input.send_touch(x, y, "ended")?;
                    }
                } else {
                    input.send_touch(x, y, phase.as_deref().unwrap_or("began"))?;
                }
                Ok(())
            })
            .await?;
            Ok(json_value!({ "action": "touch" }))
        }
        BatchStep::TouchSequence { events } => {
            if events.is_empty() {
                return Err(AppError::bad_request("touchSequence requires events."));
            }
            if events.len() > 64 {
                return Err(AppError::bad_request(
                    "touchSequence cannot contain more than 64 events.",
                ));
            }
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    for event in events {
                        if !event.x.is_finite() || !event.y.is_finite() {
                            return Err(AppError::bad_request(
                                "touchSequence requires finite normalized x and y.",
                            ));
                        }
                        android.send_touch(
                            &udid,
                            event.x.clamp(0.0, 1.0),
                            event.y.clamp(0.0, 1.0),
                            &event.phase,
                        )?;
                        if let Some(delay_ms) =
                            event.delay_ms_after.filter(|delay_ms| *delay_ms > 0)
                        {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                        }
                    }
                    Ok(())
                })
                .await?;
                return Ok(json_value!({ "action": "touchSequence" }));
            }
            run_bridge_action(state, move |bridge| {
                if bridge_simulator_is_tvos(&bridge, &udid) {
                    let key_code = tvos_touch_sequence_key(&events)?;
                    return press_tvos_remote_key(&bridge, &udid, key_code);
                }
                let input = bridge.create_input_session(&udid)?;
                for event in events {
                    if !event.x.is_finite() || !event.y.is_finite() {
                        return Err(AppError::bad_request(
                            "touchSequence requires finite normalized x and y.",
                        ));
                    }
                    input.send_touch(
                        event.x.clamp(0.0, 1.0),
                        event.y.clamp(0.0, 1.0),
                        &event.phase,
                    )?;
                    if let Some(delay_ms) = event.delay_ms_after.filter(|delay_ms| *delay_ms > 0) {
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
                Ok(())
            })
            .await?;
            Ok(json_value!({ "action": "touchSequence" }))
        }
        BatchStep::EdgeTouch { x, y, phase, edge } => {
            if !x.is_finite() || !y.is_finite() {
                return Err(AppError::bad_request(
                    "edgeTouch requires finite normalized x and y.",
                ));
            }
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.send_touch(&udid, x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), &phase)
                })
                .await?;
                return Ok(json_value!({ "action": "edgeTouch" }));
            }
            let edge = edge_name_to_hid_value(edge.as_str()).ok_or_else(|| {
                AppError::bad_request(
                    "`edge` must be `left`, `top`, `bottom`, `right`, or `none`.",
                )
            })?;
            run_bridge_action(state, move |bridge| {
                if bridge_simulator_is_tvos(&bridge, &udid) {
                    return Err(AppError::bad_request(
                        "Edge touch input is not supported for tvOS simulators.",
                    ));
                }
                let input = bridge.create_input_session(&udid)?;
                input.send_edge_touch(x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), &phase, edge)
            })
            .await?;
            Ok(json_value!({ "action": "edgeTouch" }))
        }
        BatchStep::MultiTouch {
            x1,
            y1,
            x2,
            y2,
            phase,
        } => {
            if !x1.is_finite() || !y1.is_finite() || !x2.is_finite() || !y2.is_finite() {
                return Err(AppError::bad_request(
                    "multiTouch requires finite normalized coordinates.",
                ));
            }
            if android::is_android_id(&udid) {
                return Err(AppError::bad_request(
                    "Multi-touch input is not supported for Android devices.",
                ));
            }
            run_bridge_action(state, move |bridge| {
                if bridge_simulator_is_tvos(&bridge, &udid) {
                    return Err(AppError::bad_request(
                        "Multi-touch input is not supported for tvOS simulators.",
                    ));
                }
                let input = bridge.create_input_session(&udid)?;
                input.send_multitouch(
                    x1.clamp(0.0, 1.0),
                    y1.clamp(0.0, 1.0),
                    x2.clamp(0.0, 1.0),
                    y2.clamp(0.0, 1.0),
                    &phase,
                )
            })
            .await?;
            Ok(json_value!({ "action": "multiTouch" }))
        }
        BatchStep::Swipe {
            start_x,
            start_y,
            end_x,
            end_y,
            duration_ms,
            steps,
        } => {
            if !start_x.is_finite()
                || !start_y.is_finite()
                || !end_x.is_finite()
                || !end_y.is_finite()
            {
                return Err(AppError::bad_request(
                    "swipe requires finite normalized coordinates.",
                ));
            }
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.send_swipe(
                        &udid,
                        start_x,
                        start_y,
                        end_x,
                        end_y,
                        duration_ms.unwrap_or(350),
                    )
                })
                .await?;
                return Ok(json_value!({ "action": "swipe" }));
            }
            run_bridge_action(state, move |bridge| {
                if bridge_simulator_is_tvos(&bridge, &udid) {
                    let key_code = tvos_remote_key_for_touch_motion(
                        start_x.clamp(0.0, 1.0),
                        start_y.clamp(0.0, 1.0),
                        end_x.clamp(0.0, 1.0),
                        end_y.clamp(0.0, 1.0),
                    );
                    return press_tvos_remote_key(&bridge, &udid, key_code);
                }
                let step_count = steps.unwrap_or(12).max(1);
                let delay =
                    Duration::from_millis(duration_ms.unwrap_or(350) / u64::from(step_count));
                let input = bridge.create_input_session(&udid)?;
                let start_x = start_x.clamp(0.0, 1.0);
                let start_y = start_y.clamp(0.0, 1.0);
                let end_x = end_x.clamp(0.0, 1.0);
                let end_y = end_y.clamp(0.0, 1.0);
                input.send_touch(start_x, start_y, "began")?;
                for step in 1..step_count {
                    let t = f64::from(step) / f64::from(step_count);
                    input.send_touch(
                        start_x + (end_x - start_x) * t,
                        start_y + (end_y - start_y) * t,
                        "moved",
                    )?;
                    std::thread::sleep(delay);
                }
                input.send_touch(end_x, end_y, "ended")
            })
            .await?;
            Ok(json_value!({ "action": "swipe" }))
        }
        BatchStep::Gesture {
            preset,
            duration_ms,
            delta,
            steps,
        } => {
            let (start_x, start_y, end_x, end_y, default_duration_ms) =
                normalized_gesture_coordinates(&preset, delta)?;
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.send_swipe(
                        &udid,
                        start_x,
                        start_y,
                        end_x,
                        end_y,
                        duration_ms.unwrap_or(default_duration_ms),
                    )
                })
                .await?;
                return Ok(json_value!({ "action": "gesture", "preset": preset }));
            }
            run_bridge_action(state, move |bridge| {
                if bridge_simulator_is_tvos(&bridge, &udid) {
                    let key_code = tvos_remote_key_for_touch_motion(start_x, start_y, end_x, end_y);
                    return press_tvos_remote_key(&bridge, &udid, key_code);
                }
                let step_count = steps.unwrap_or(12).max(1);
                let delay = Duration::from_millis(
                    duration_ms.unwrap_or(default_duration_ms) / u64::from(step_count),
                );
                let input = bridge.create_input_session(&udid)?;
                input.send_touch(start_x, start_y, "began")?;
                for step in 1..step_count {
                    let t = f64::from(step) / f64::from(step_count);
                    input.send_touch(
                        start_x + (end_x - start_x) * t,
                        start_y + (end_y - start_y) * t,
                        "moved",
                    )?;
                    std::thread::sleep(delay);
                }
                input.send_touch(end_x, end_y, "ended")
            })
            .await?;
            Ok(json_value!({ "action": "gesture", "preset": preset }))
        }
        BatchStep::Type { text, delay_ms } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    if delay_ms.is_some() {
                        for character in text.chars() {
                            android.type_text(&udid, &character.to_string())?;
                            if let Some(delay_ms) = delay_ms.filter(|delay_ms| *delay_ms > 0) {
                                std::thread::sleep(Duration::from_millis(delay_ms));
                            }
                        }
                        Ok(())
                    } else {
                        android.type_text(&udid, &text)
                    }
                })
                .await?;
                return Ok(json_value!({ "action": "type" }));
            }
            run_bridge_action(state, move |bridge| {
                let input = bridge.create_input_session(&udid)?;
                for character in text.chars() {
                    let Some((key_code, modifiers)) = hid_for_character(character) else {
                        return Err(AppError::bad_request(format!(
                            "Unsupported character for HID typing: {character:?}"
                        )));
                    };
                    input.send_key(key_code, modifiers)?;
                    if let Some(delay_ms) = delay_ms.filter(|delay_ms| *delay_ms > 0) {
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
                Ok(())
            })
            .await?;
            Ok(json_value!({ "action": "type" }))
        }
        BatchStep::Button {
            button,
            duration_ms,
            phase,
            usage_page,
            usage,
        } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    match phase.as_deref() {
                        Some("down" | "began") => Ok(()),
                        Some("up" | "ended" | "cancelled") | None => {
                            android.press_button(&udid, &button, duration_ms.unwrap_or(0))
                        }
                        Some(_) => Err(AppError::bad_request(
                            "`phase` must be `down`, `up`, `began`, `ended`, or `cancelled`.",
                        )),
                    }
                })
                .await?;
                return Ok(json_value!({ "action": "button" }));
            }
            if let Some(phase) = phase {
                let pressed = match phase.as_str() {
                    "down" | "began" => true,
                    "up" | "ended" | "cancelled" => false,
                    _ => {
                        return Err(AppError::bad_request(
                            "`phase` must be `down`, `up`, `began`, `ended`, or `cancelled`.",
                        ))
                    }
                };
                run_bridge_action(state, move |bridge| {
                    bridge.send_button(&udid, &button, pressed, usage_page, usage)
                })
                .await?;
            } else {
                run_bridge_action(state, move |bridge| {
                    bridge.press_button(&udid, &button, duration_ms.unwrap_or(0))
                })
                .await?;
            }
            Ok(json_value!({ "action": "button" }))
        }
        BatchStep::Crown { delta } => {
            run_bridge_action(state, move |bridge| bridge.rotate_crown(&udid, delta)).await?;
            Ok(json_value!({ "action": "crown" }))
        }
        BatchStep::Launch { bundle_id } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| {
                    android.launch_package(&udid, &bundle_id)
                })
                .await?;
                return Ok(json_value!({ "action": "launch" }));
            }
            let warm_state = state.clone();
            let warm_udid = udid.clone();
            run_bridge_action(state, move |bridge| bridge.launch_bundle(&udid, &bundle_id)).await?;
            spawn_accessibility_warmup(warm_state, warm_udid);
            Ok(json_value!({ "action": "launch" }))
        }
        BatchStep::OpenUrl { url } => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.open_url(&udid, &url)).await?;
                return Ok(json_value!({ "action": "openUrl" }));
            }
            let warm_state = state.clone();
            let warm_udid = udid.clone();
            run_bridge_action(state, move |bridge| bridge.open_url(&udid, &url)).await?;
            spawn_accessibility_warmup(warm_state, warm_udid);
            Ok(json_value!({ "action": "openUrl" }))
        }
        BatchStep::Home => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.press_home(&udid)).await?;
                return Ok(json_value!({ "action": "home" }));
            }
            run_bridge_action(state, move |bridge| bridge.press_home(&udid)).await?;
            Ok(json_value!({ "action": "home" }))
        }
        BatchStep::DismissKeyboard => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.dismiss_keyboard(&udid)).await?;
                return Ok(json_value!({ "action": "dismissKeyboard" }));
            }
            run_bridge_action(state, move |bridge| bridge.send_key(&udid, 41, 0)).await?;
            Ok(json_value!({ "action": "dismissKeyboard" }))
        }
        BatchStep::AppSwitcher => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.open_app_switcher(&udid)).await?;
                return Ok(json_value!({ "action": "appSwitcher" }));
            }
            run_bridge_action(state, move |bridge| bridge.open_app_switcher(&udid)).await?;
            Ok(json_value!({ "action": "appSwitcher" }))
        }
        BatchStep::RotateLeft => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.rotate_left(&udid)).await?;
                return Ok(json_value!({ "action": "rotateLeft" }));
            }
            run_bridge_action(state, move |bridge| bridge.rotate_left(&udid)).await?;
            Ok(json_value!({ "action": "rotateLeft" }))
        }
        BatchStep::RotateRight => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.rotate_right(&udid)).await?;
                return Ok(json_value!({ "action": "rotateRight" }));
            }
            run_bridge_action(state, move |bridge| bridge.rotate_right(&udid)).await?;
            Ok(json_value!({ "action": "rotateRight" }))
        }
        BatchStep::ToggleAppearance => {
            if android::is_android_id(&udid) {
                run_android_action(state, move |android| android.toggle_appearance(&udid)).await?;
                return Ok(json_value!({ "action": "toggleAppearance" }));
            }
            run_bridge_action(state, move |bridge| bridge.toggle_appearance(&udid)).await?;
            Ok(json_value!({ "action": "toggleAppearance" }))
        }
        BatchStep::Describe {
            source,
            max_depth,
            include_hidden,
        } => {
            let snapshot = cached_accessibility_tree_value(
                state,
                udid,
                source.as_deref(),
                max_depth,
                include_hidden.unwrap_or(false),
                false,
            )
            .await?;
            Ok(json_value!({
                "action": "describe",
                "snapshot": compact_accessibility_snapshot(&snapshot),
            }))
        }
    }
}
