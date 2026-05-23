fn resolve_tap_target(
    bridge: &NativeBridge,
    udid: &str,
    request: TapTargetRequest,
) -> Result<ResolvedTapTarget, crate::error::AppError> {
    if request.selector.is_empty() {
        let x = request.x.ok_or_else(|| {
            crate::error::AppError::bad_request("Tap requires x and y or a selector.")
        })?;
        let y = request.y.ok_or_else(|| {
            crate::error::AppError::bad_request("Tap requires x and y or a selector.")
        })?;
        let (x, y) = resolve_touch_point(bridge, udid, x, y, request.normalized)?;
        return Ok(ResolvedTapTarget { x, y, input: None });
    }

    let deadline = std::time::Instant::now() + Duration::from_millis(request.wait_timeout_ms);
    loop {
        let snapshot = bridge.accessibility_snapshot_with_options(udid, None, None, true)?;
        if let Some(target) = find_element_tap_target(&snapshot, &request.selector) {
            let input = bridge.create_input_session(udid)?;
            let (x, y) = if let Some((display_width, display_height)) = input.display_size() {
                normalize_accessibility_point_for_display(
                    target.x,
                    target.y,
                    target.root_width,
                    target.root_height,
                    display_width,
                    display_height,
                )
            } else {
                (
                    (target.x / target.root_width).clamp(0.0, 1.0),
                    (target.y / target.root_height).clamp(0.0, 1.0),
                )
            };
            return Ok(ResolvedTapTarget {
                x,
                y,
                input: Some(input),
            });
        }
        if request.wait_timeout_ms == 0 || std::time::Instant::now() >= deadline {
            return Err(crate::error::AppError::not_found(
                "No accessibility element matched the tap selector.",
            ));
        }
        sleep_ms(request.poll_interval_ms.max(10));
    }
}

fn find_element_tap_target(
    snapshot: &Value,
    selector: &ElementSelector,
) -> Option<ElementTapTarget> {
    let roots = snapshot.get("roots")?.as_array()?;
    let mut matches = Vec::new();
    for root in roots {
        let (root_width, root_height) = element_size(root)?;
        collect_matching_elements(root, selector, root_width, root_height, &mut matches);
    }
    let target = if let Some(index) = selector.index {
        matches.into_iter().nth(index)
    } else {
        matches
            .into_iter()
            .max_by_key(|target| is_actionable_element(target.node) as u8)
    };
    target.and_then(|target| {
        element_center(target.node).map(|(x, y)| ElementTapTarget {
            x,
            y,
            root_width: target.root_width,
            root_height: target.root_height,
        })
    })
}

struct MatchedElement<'a> {
    node: &'a Value,
    root_width: f64,
    root_height: f64,
}

fn collect_matching_elements<'a>(
    node: &'a Value,
    selector: &ElementSelector,
    root_width: f64,
    root_height: f64,
    matches: &mut Vec<MatchedElement<'a>>,
) {
    if element_matches(node, selector) {
        matches.push(MatchedElement {
            node,
            root_width,
            root_height,
        });
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            collect_matching_elements(child, selector, root_width, root_height, matches);
        }
    }
}

fn element_matches(node: &Value, selector: &ElementSelector) -> bool {
    if let Some(element_type) = &selector.element_type {
        let node_type = string_field(node, "type").or_else(|| string_field(node, "role"));
        if !node_type
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case(element_type))
            .unwrap_or(false)
        {
            return false;
        }
    }
    if let Some(id) = &selector.id {
        return [
            "AXUniqueId",
            "AXIdentifier",
            "id",
            "identifier",
            "inspectorId",
        ]
        .iter()
        .filter_map(|key| string_field(node, key))
        .any(|value| value == *id);
    }
    if let Some(label) = &selector.label {
        return ["AXLabel", "label", "title", "name"]
            .iter()
            .filter_map(|key| string_field(node, key))
            .any(|value| value == *label);
    }
    if let Some(expected_value) = &selector.value {
        return ["AXValue", "value"]
            .iter()
            .filter_map(|key| string_field(node, key))
            .any(|value| value == *expected_value);
    }
    true
}

fn string_field(node: &Value, key: &str) -> Option<String> {
    node.get(key)?.as_str().map(str::to_owned)
}

fn element_center(node: &Value) -> Option<(f64, f64)> {
    let frame = node.get("frame")?;
    let x = frame.get("x")?.as_f64()?;
    let y = frame.get("y")?.as_f64()?;
    let width = frame.get("width")?.as_f64()?;
    let height = frame.get("height")?.as_f64()?;
    (width > 0.0 && height > 0.0).then_some((x + width / 2.0, y + height / 2.0))
}

fn element_size(node: &Value) -> Option<(f64, f64)> {
    let frame = node.get("frame")?;
    let width = frame.get("width")?.as_f64()?;
    let height = frame.get("height")?.as_f64()?;
    (width > 0.0 && height > 0.0).then_some((width, height))
}

fn normalize_accessibility_point_for_display(
    x: f64,
    y: f64,
    root_width: f64,
    root_height: f64,
    display_width: f64,
    display_height: f64,
) -> (f64, f64) {
    let normalized_x = (x / root_width).clamp(0.0, 1.0);
    let normalized_y = (y / root_height).clamp(0.0, 1.0);
    let root_is_landscape = root_width > root_height;
    let display_is_landscape = display_width > display_height;
    if root_is_landscape != display_is_landscape {
        return (normalized_y, normalized_x);
    }
    (normalized_x, normalized_y)
}

fn is_actionable_element(node: &Value) -> bool {
    let haystack = format!(
        "{} {}",
        string_field(node, "type").unwrap_or_default(),
        string_field(node, "role").unwrap_or_default()
    )
    .to_lowercase();
    ["button", "textfield", "switch", "link", "cell"]
        .iter()
        .any(|needle| haystack.contains(needle))
}
