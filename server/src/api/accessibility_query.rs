fn query_compact_elements(
    snapshot: &Value,
    selector: &ElementSelectorPayload,
    limit: usize,
) -> Vec<Value> {
    let mut matches = Vec::new();
    if let Some(roots) = snapshot.get("roots").and_then(Value::as_array) {
        for root in roots {
            let target_limit = selector.index.map(|index| index + 1).unwrap_or(limit);
            collect_query_matches(root, selector, target_limit, &mut matches);
            if matches.len() >= target_limit {
                break;
            }
        }
    }
    if let Some(index) = selector.index {
        return matches.into_iter().nth(index).into_iter().collect();
    }
    matches
}

fn collect_query_matches(
    node: &Value,
    selector: &ElementSelectorPayload,
    limit: usize,
    matches: &mut Vec<Value>,
) {
    if matches.len() >= limit {
        return;
    }
    if element_matches_selector(node, selector) {
        matches.push(compact_accessibility_node(node));
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            collect_query_matches(child, selector, limit, matches);
            if matches.len() >= limit {
                return;
            }
        }
    }
}

fn first_matching_element(snapshot: &Value, selector: &ElementSelectorPayload) -> Option<Value> {
    let roots = snapshot.get("roots")?.as_array()?;
    if let Some(index) = selector.index {
        let mut matches = Vec::new();
        for root in roots {
            collect_query_matches(root, selector, index + 1, &mut matches);
            if matches.len() > index {
                break;
            }
        }
        return matches.into_iter().nth(index);
    }
    for root in roots {
        if let Some(found) = first_matching_node(root, selector) {
            return Some(found.clone());
        }
    }
    None
}

fn first_matching_node<'a>(
    node: &'a Value,
    selector: &ElementSelectorPayload,
) -> Option<&'a Value> {
    if element_matches_selector(node, selector) {
        return Some(node);
    }
    for child in node
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(found) = first_matching_node(child, selector) {
            return Some(found);
        }
    }
    None
}

fn element_matches_selector(node: &Value, selector: &ElementSelectorPayload) -> bool {
    if selector_is_empty(selector) {
        return true;
    }
    let use_regex = selector.regex.unwrap_or(false);
    selector.element_type.as_ref().is_none_or(|expected| {
        string_fields_match(node, expected, use_regex, &["type", "role", "className"])
    }) && selector.id.as_ref().is_none_or(|expected| {
        string_fields_match(
            node,
            expected,
            use_regex,
            &[
                "AXIdentifier",
                "AXUniqueId",
                "inspectorId",
                "id",
                "identifier",
            ],
        )
    }) && selector.text.as_ref().is_none_or(|expected| {
        string_fields_match(
            node,
            expected,
            use_regex,
            &["AXLabel", "label", "title", "text", "name"],
        )
    }) && selector.label.as_ref().is_none_or(|expected| {
        string_fields_match(
            node,
            expected,
            use_regex,
            &["AXLabel", "label", "title", "text", "name"],
        )
    }) && selector.value.as_ref().is_none_or(|expected| {
        string_fields_match(node, expected, use_regex, &["AXValue", "value"])
    }) && selector.enabled.is_none_or(|expected| {
        bool_fields_match(
            node,
            expected,
            &[
                "enabled",
                "AXEnabled",
                "isEnabled",
                "isUserInteractionEnabled",
            ],
        )
    }) && selector.checked.is_none_or(|expected| {
        bool_or_state_fields_match(
            node,
            expected,
            &["checked", "isChecked", "AXChecked"],
            &["AXValue", "value"],
            &["1", "true", "yes", "on", "checked", "selected"],
        )
    }) && selector.focused.is_none_or(|expected| {
        bool_fields_match(node, expected, &["focused", "isFocused", "AXFocused"])
    }) && selector.selected.is_none_or(|expected| {
        bool_or_state_fields_match(
            node,
            expected,
            &["selected", "isSelected", "AXSelected"],
            &["AXValue", "value"],
            &["selected", "1", "true", "yes", "on"],
        )
    })
}

fn selector_is_empty(selector: &ElementSelectorPayload) -> bool {
    selector.text.is_none()
        && selector.id.is_none()
        && selector.label.is_none()
        && selector.value.is_none()
        && selector.element_type.is_none()
        && selector.enabled.is_none()
        && selector.checked.is_none()
        && selector.focused.is_none()
        && selector.selected.is_none()
        && selector.index.is_none()
}

fn string_fields_match(node: &Value, expected: &str, use_regex: bool, fields: &[&str]) -> bool {
    let regex = use_regex.then(|| Regex::new(expected).ok()).flatten();
    fields
        .iter()
        .filter_map(|field| node.get(*field).and_then(Value::as_str))
        .any(|value| {
            if let Some(regex) = regex.as_ref() {
                regex.is_match(value)
            } else {
                value == expected
            }
        })
}

fn bool_fields_match(node: &Value, expected: bool, fields: &[&str]) -> bool {
    fields
        .iter()
        .find_map(|field| node.get(*field).and_then(Value::as_bool))
        .is_some_and(|value| value == expected)
}

fn bool_or_state_fields_match(
    node: &Value,
    expected: bool,
    bool_fields: &[&str],
    string_fields: &[&str],
    truthy_values: &[&str],
) -> bool {
    if bool_fields_match(node, expected, bool_fields) {
        return true;
    }
    string_fields
        .iter()
        .filter_map(|field| node.get(*field).and_then(Value::as_str))
        .any(|value| {
            let truthy = truthy_values
                .iter()
                .any(|truthy| value.eq_ignore_ascii_case(truthy));
            truthy == expected
        })
}

fn tap_point_from_snapshot(
    snapshot: &Value,
    selector: &ElementSelectorPayload,
) -> Result<(f64, f64), AppError> {
    let roots = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::not_found("Accessibility snapshot does not contain roots."))?;
    let mut seen_matches = 0usize;
    for root in roots {
        let root_frame = root
            .get("frame")
            .or_else(|| root.get("frameInScreen"))
            .ok_or_else(|| AppError::not_found("Accessibility root does not expose a frame."))?;
        let root_width = number_field(root_frame, "width")?;
        let root_height = number_field(root_frame, "height")?;
        let node = if selector.index.is_some() {
            indexed_matching_node(root, selector, &mut seen_matches)
        } else {
            best_tappable_matching_node(root, selector, root_width, root_height)
        };
        if let Some(node) = node {
            let frame = node
                .get("frame")
                .or_else(|| node.get("frameInScreen"))
                .ok_or_else(|| AppError::not_found("Matched element does not expose a frame."))?;
            let x = number_field(frame, "x")? + number_field(frame, "width")? / 2.0;
            let y = number_field(frame, "y")? + number_field(frame, "height")? / 2.0;
            return Ok((
                (x / root_width).clamp(0.0, 1.0),
                (y / root_height).clamp(0.0, 1.0),
            ));
        }
    }
    Err(AppError::not_found("No accessibility element matched."))
}

fn best_tappable_matching_node<'a>(
    root: &'a Value,
    selector: &ElementSelectorPayload,
    root_width: f64,
    root_height: f64,
) -> Option<&'a Value> {
    let mut matches = Vec::new();
    collect_matching_nodes(root, selector, &mut matches);
    let mut best = None;
    let mut best_rank = None;
    for node in matches {
        let rank = tap_candidate_rank(node, root_width, root_height);
        if best_rank.is_none_or(|current| rank > current) {
            best = Some(node);
            best_rank = Some(rank);
        }
    }
    best
}

fn collect_matching_nodes<'a>(
    node: &'a Value,
    selector: &ElementSelectorPayload,
    matches: &mut Vec<&'a Value>,
) {
    if element_matches_selector(node, selector) {
        matches.push(node);
    }
    for child in node
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        collect_matching_nodes(child, selector, matches);
    }
}

fn tap_candidate_rank(node: &Value, root_width: f64, root_height: f64) -> (u8, u8, u64) {
    let role_rank = tappable_role_rank(node);
    let Some((x, y, width, height)) = node_rect(node) else {
        return (role_rank, 0, 0);
    };
    if width <= 0.0 || height <= 0.0 {
        return (role_rank, 0, 0);
    }
    let center_x = x + width / 2.0;
    let center_y = y + height / 2.0;
    let fully_inside = x >= 0.0 && y >= 0.0 && x + width <= root_width && y + height <= root_height;
    let center_inside = center_x >= 0.0
        && center_y >= 0.0
        && center_x <= root_width
        && center_y <= root_height;
    let intersects = x < root_width && y < root_height && x + width > 0.0 && y + height > 0.0;
    let visibility_rank = if fully_inside {
        3
    } else if center_inside && x >= 0.0 && y >= 0.0 {
        2
    } else if intersects {
        1
    } else {
        0
    };
    let visible_width = (x + width).min(root_width) - x.max(0.0);
    let visible_height = (y + height).min(root_height) - y.max(0.0);
    let visible_area = if visible_width > 0.0 && visible_height > 0.0 {
        (visible_width * visible_height).round() as u64
    } else {
        0
    };
    (role_rank, visibility_rank, visible_area)
}

fn tappable_role_rank(node: &Value) -> u8 {
    let role = node
        .get("role")
        .or_else(|| node.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if [
        "button", "cell", "checkbox", "link", "switch", "toggle", "slider", "textfield",
    ]
    .iter()
    .any(|needle| role.contains(needle))
    {
        2
    } else if role.contains("application") || role.contains("window") || role.contains("group") {
        0
    } else {
        1
    }
}

fn node_rect(node: &Value) -> Option<(f64, f64, f64, f64)> {
    let frame = node.get("frame").or_else(|| node.get("frameInScreen"))?;
    Some((
        number_field(frame, "x").ok()?,
        number_field(frame, "y").ok()?,
        number_field(frame, "width").ok()?,
        number_field(frame, "height").ok()?,
    ))
}

fn indexed_matching_node<'a>(
    node: &'a Value,
    selector: &ElementSelectorPayload,
    seen_matches: &mut usize,
) -> Option<&'a Value> {
    if element_matches_selector(node, selector) {
        if selector.index.unwrap_or(0) == *seen_matches {
            return Some(node);
        }
        *seen_matches += 1;
    }
    for child in node
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(found) = indexed_matching_node(child, selector, seen_matches) {
            return Some(found);
        }
    }
    None
}

fn normalize_screen_point_from_snapshot(
    snapshot: &Value,
    x: f64,
    y: f64,
) -> Result<(f64, f64), AppError> {
    let root = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .and_then(|roots| roots.first())
        .ok_or_else(|| AppError::not_found("Accessibility snapshot does not contain a root."))?;
    let frame = root
        .get("frame")
        .or_else(|| root.get("frameInScreen"))
        .ok_or_else(|| AppError::not_found("Accessibility root does not expose a frame."))?;
    let width = number_field(frame, "width")?;
    let height = number_field(frame, "height")?;
    if width <= 0.0 || height <= 0.0 {
        return Err(AppError::not_found("Accessibility root frame is empty."));
    }
    Ok(((x / width).clamp(0.0, 1.0), (y / height).clamp(0.0, 1.0)))
}

fn accessibility_point_snapshot(snapshot: &Value, x: f64, y: f64) -> Result<Value, AppError> {
    let roots = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::not_found("Accessibility snapshot does not contain roots."))?;
    let node = roots
        .iter()
        .rev()
        .find_map(|root| deepest_node_at_point(root, x, y))
        .ok_or_else(|| AppError::not_found("No accessibility element contains the point."))?;
    let mut node = node.clone();
    if let Some(object) = node.as_object_mut() {
        object.remove("children");
    }

    let mut response = Map::new();
    for key in [
        "source",
        "availableSources",
        "requestedSource",
        "fallbackReason",
        "inspector",
        "includeHidden",
    ] {
        if let Some(value) = snapshot.get(key) {
            response.insert(key.to_owned(), value.clone());
        }
    }
    response.insert("roots".to_owned(), Value::Array(vec![node]));
    Ok(Value::Object(response))
}

fn point_snapshot_looks_like_local_widget_coordinates(snapshot: &Value, x: f64, y: f64) -> bool {
    let Some(roots) = snapshot.get("roots").and_then(Value::as_array) else {
        return false;
    };
    if roots.len() != 1 {
        return false;
    }

    let Some(frame) = roots[0]
        .get("frame")
        .or_else(|| roots[0].get("frameInScreen"))
    else {
        return false;
    };
    let Ok(frame_x) = number_field(frame, "x") else {
        return false;
    };
    let Ok(frame_y) = number_field(frame, "y") else {
        return false;
    };
    let Ok(width) = number_field(frame, "width") else {
        return false;
    };
    let Ok(height) = number_field(frame, "height") else {
        return false;
    };

    if width <= 0.0 || height <= 0.0 || frame_x > 64.0 || frame_y > 64.0 {
        return false;
    }

    let compact_local_frame = width <= 240.0 && height <= 240.0;
    let point_outside_frame = x > frame_x + width || y > frame_y + height;
    compact_local_frame || point_outside_frame
}

fn deepest_node_at_point(node: &Value, x: f64, y: f64) -> Option<&Value> {
    let has_frame = node
        .get("frame")
        .or_else(|| node.get("frameInScreen"))
        .is_some();
    if has_frame && !node_frame_contains_point(node, x, y).unwrap_or(false) {
        return None;
    }
    for child in node
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
    {
        if let Some(found) = deepest_node_at_point(child, x, y) {
            return Some(found);
        }
    }
    has_frame.then_some(node)
}

fn node_frame_contains_point(node: &Value, x: f64, y: f64) -> Result<bool, AppError> {
    let frame = node
        .get("frame")
        .or_else(|| node.get("frameInScreen"))
        .ok_or_else(|| AppError::not_found("Accessibility node does not expose a frame."))?;
    let frame_x = number_field(frame, "x")?;
    let frame_y = number_field(frame, "y")?;
    let width = number_field(frame, "width")?;
    let height = number_field(frame, "height")?;
    Ok(width > 0.0
        && height > 0.0
        && x >= frame_x
        && y >= frame_y
        && x < frame_x + width
        && y < frame_y + height)
}

fn number_field(value: &Value, field: &str) -> Result<f64, AppError> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| AppError::not_found(format!("Missing numeric frame field `{field}`.")))
}

fn hid_for_character(character: char) -> Option<(u16, u32)> {
    let shift: u32 = 1;
    let mapping = match character {
        'a'..='z' => (character as u16 - b'a' as u16 + 4, 0),
        'A'..='Z' => (character as u16 - b'A' as u16 + 4, shift),
        '1' => (30, 0),
        '!' => (30, shift),
        '2' => (31, 0),
        '@' => (31, shift),
        '3' => (32, 0),
        '#' => (32, shift),
        '4' => (33, 0),
        '$' => (33, shift),
        '5' => (34, 0),
        '%' => (34, shift),
        '6' => (35, 0),
        '^' => (35, shift),
        '7' => (36, 0),
        '&' => (36, shift),
        '8' => (37, 0),
        '*' => (37, shift),
        '9' => (38, 0),
        '(' => (38, shift),
        '0' => (39, 0),
        ')' => (39, shift),
        '\n' | '\r' => (40, 0),
        '\t' => (43, 0),
        ' ' => (44, 0),
        '-' => (45, 0),
        '_' => (45, shift),
        '=' => (46, 0),
        '+' => (46, shift),
        '[' => (47, 0),
        '{' => (47, shift),
        ']' => (48, 0),
        '}' => (48, shift),
        '\\' => (49, 0),
        '|' => (49, shift),
        ';' => (51, 0),
        ':' => (51, shift),
        '\'' => (52, 0),
        '"' => (52, shift),
        '`' => (53, 0),
        '~' => (53, shift),
        ',' => (54, 0),
        '<' => (54, shift),
        '.' => (55, 0),
        '>' => (55, shift),
        '/' => (56, 0),
        '?' => (56, shift),
        _ => return None,
    };
    Some(mapping)
}

fn normalized_gesture_coordinates(
    preset: &str,
    delta: Option<f64>,
) -> Result<(f64, f64, f64, f64, u64), AppError> {
    let center_x = 0.5;
    let center_y = 0.5;
    let distance = delta.unwrap_or(0.45).clamp(0.05, 0.95);
    let edge = 0.02;
    let coordinates = match preset {
        "scroll-up" => (
            center_x,
            center_y - distance / 2.0,
            center_x,
            center_y + distance / 2.0,
            500,
        ),
        "scroll-down" => (
            center_x,
            center_y + distance / 2.0,
            center_x,
            center_y - distance / 2.0,
            500,
        ),
        "scroll-left" => (
            center_x - distance / 2.0,
            center_y,
            center_x + distance / 2.0,
            center_y,
            500,
        ),
        "scroll-right" => (
            center_x + distance / 2.0,
            center_y,
            center_x - distance / 2.0,
            center_y,
            500,
        ),
        "swipe-from-left-edge" => (edge, center_y, 1.0 - edge, center_y, 300),
        "swipe-from-right-edge" => (1.0 - edge, center_y, edge, center_y, 300),
        "swipe-from-top-edge" => (center_x, edge, center_x, 1.0 - edge, 300),
        "swipe-from-bottom-edge" => (center_x, 1.0 - edge, center_x, edge, 300),
        _ => {
            return Err(AppError::bad_request(format!(
                "Unsupported gesture preset `{preset}`."
            )))
        }
    };
    Ok(coordinates)
}

fn compact_accessibility_snapshot(snapshot: &Value) -> Value {
    let roots = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .map(|roots| {
            roots
                .iter()
                .map(compact_accessibility_node)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json_value!({
        "source": snapshot.get("source").cloned().unwrap_or(Value::Null),
        "roots": roots,
    })
}

fn compact_accessibility_node(node: &Value) -> Value {
    let mut object = Map::new();
    copy_first_string(node, &mut object, "role", &["type", "role", "className"]);
    copy_first_string(
        node,
        &mut object,
        "id",
        &[
            "AXIdentifier",
            "AXUniqueId",
            "inspectorId",
            "id",
            "identifier",
        ],
    );
    copy_first_string(
        node,
        &mut object,
        "label",
        &["AXLabel", "label", "title", "text", "name"],
    );
    copy_first_string(node, &mut object, "value", &["AXValue", "value"]);
    if let Some(frame) = node.get("frame").or_else(|| node.get("frameInScreen")) {
        object.insert("frame".to_owned(), frame.clone());
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        let children = children
            .iter()
            .map(compact_accessibility_node)
            .collect::<Vec<_>>();
        if !children.is_empty() {
            object.insert("children".to_owned(), Value::Array(children));
        }
    }
    Value::Object(object)
}

fn copy_first_string(
    source: &Value,
    target: &mut Map<String, Value>,
    output_key: &str,
    input_keys: &[&str],
) {
    if let Some(value) = input_keys
        .iter()
        .filter_map(|key| source.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
    {
        target.insert(output_key.to_owned(), Value::String(value.to_owned()));
    }
}
