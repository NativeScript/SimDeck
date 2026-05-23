fn gesture_coordinates(
    bridge: &NativeBridge,
    udid: &str,
    preset: &str,
    screen_width: Option<f64>,
    screen_height: Option<f64>,
    normalized: bool,
    delta: Option<f64>,
) -> Result<GestureCoordinates, crate::error::AppError> {
    let (width, height) = if normalized {
        (1.0, 1.0)
    } else {
        match (screen_width, screen_height) {
            (Some(width), Some(height)) => (width, height),
            _ => accessibility_root_size(bridge, udid)
                .or_else(|| chrome_screen_size(bridge, udid))
                .unwrap_or((390.0, 844.0)),
        }
    };
    let center_x = width / 2.0;
    let center_y = height / 2.0;
    let edge = if normalized { 0.02 } else { 20.0 };
    let distance = delta.unwrap_or(if normalized { 0.25 } else { 200.0 });
    let (start_x, start_y, end_x, end_y, duration_ms) = match preset {
        "scroll-up" => (
            center_x,
            center_y + distance / 2.0,
            center_x,
            center_y - distance / 2.0,
            500,
        ),
        "scroll-down" => (
            center_x,
            center_y - distance / 2.0,
            center_x,
            center_y + distance / 2.0,
            500,
        ),
        "scroll-left" => (
            center_x + distance / 2.0,
            center_y,
            center_x - distance / 2.0,
            center_y,
            500,
        ),
        "scroll-right" => (
            center_x - distance / 2.0,
            center_y,
            center_x + distance / 2.0,
            center_y,
            500,
        ),
        "swipe-from-left-edge" => (edge, center_y, width - edge, center_y, 300),
        "swipe-from-right-edge" => (width - edge, center_y, edge, center_y, 300),
        "swipe-from-top-edge" => (center_x, edge, center_x, height - edge, 300),
        "swipe-from-bottom-edge" => (center_x, height - edge, center_x, edge, 300),
        _ => {
            return Err(crate::error::AppError::bad_request(format!(
                "Unsupported gesture preset `{preset}`."
            )))
        }
    };
    let (start_x, start_y) = resolve_touch_point(bridge, udid, start_x, start_y, normalized)?;
    let (end_x, end_y) = resolve_touch_point(bridge, udid, end_x, end_y, normalized)?;
    Ok(GestureCoordinates {
        start_x,
        start_y,
        end_x,
        end_y,
        duration_ms,
    })
}

#[allow(clippy::too_many_arguments)]
fn pinch_frames(
    bridge: &NativeBridge,
    udid: &str,
    center_x: Option<f64>,
    center_y: Option<f64>,
    start_distance: f64,
    end_distance: f64,
    angle_degrees: f64,
    normalized: bool,
    steps: u32,
) -> Result<Vec<MultiTouchFrame>, crate::error::AppError> {
    if start_distance < 0.0 || end_distance < 0.0 {
        return Err(crate::error::AppError::bad_request(
            "Pinch distances must be non-negative.",
        ));
    }
    let (width, height) = gesture_surface_size(bridge, udid, normalized);
    let center_x = center_x.unwrap_or(width / 2.0);
    let center_y = center_y.unwrap_or(height / 2.0);
    let angle = angle_degrees.to_radians();
    let unit_x = angle.cos();
    let unit_y = angle.sin();
    let count = steps.max(2);
    let mut frames = Vec::with_capacity(count as usize);
    for step in 0..count {
        let t = if count == 1 {
            1.0
        } else {
            f64::from(step) / f64::from(count - 1)
        };
        let distance = lerp(start_distance, end_distance, t) / 2.0;
        let p1x = center_x - unit_x * distance;
        let p1y = center_y - unit_y * distance;
        let p2x = center_x + unit_x * distance;
        let p2y = center_y + unit_y * distance;
        let (x1, y1) = resolve_touch_point(bridge, udid, p1x, p1y, normalized)?;
        let (x2, y2) = resolve_touch_point(bridge, udid, p2x, p2y, normalized)?;
        frames.push(MultiTouchFrame { x1, y1, x2, y2 });
    }
    Ok(frames)
}

fn rotate_gesture_frames(
    bridge: &NativeBridge,
    udid: &str,
    request: RotateGestureRequest,
) -> Result<Vec<MultiTouchFrame>, crate::error::AppError> {
    if request.radius < 0.0 {
        return Err(crate::error::AppError::bad_request(
            "Rotate gesture radius must be non-negative.",
        ));
    }
    let (width, height) = gesture_surface_size(bridge, udid, request.normalized);
    let center_x = request.center_x.unwrap_or(width / 2.0);
    let center_y = request.center_y.unwrap_or(height / 2.0);
    let count = request.steps.max(2);
    let mut frames = Vec::with_capacity(count as usize);
    for step in 0..count {
        let t = if count == 1 {
            1.0
        } else {
            f64::from(step) / f64::from(count - 1)
        };
        let angle = (request.degrees * t).to_radians();
        let unit_x = angle.cos();
        let unit_y = angle.sin();
        let p1x = center_x - unit_x * request.radius;
        let p1y = center_y - unit_y * request.radius;
        let p2x = center_x + unit_x * request.radius;
        let p2y = center_y + unit_y * request.radius;
        let (x1, y1) = resolve_touch_point(bridge, udid, p1x, p1y, request.normalized)?;
        let (x2, y2) = resolve_touch_point(bridge, udid, p2x, p2y, request.normalized)?;
        frames.push(MultiTouchFrame { x1, y1, x2, y2 });
    }
    Ok(frames)
}

fn gesture_surface_size(bridge: &NativeBridge, udid: &str, normalized: bool) -> (f64, f64) {
    if normalized {
        return (1.0, 1.0);
    }
    accessibility_root_size(bridge, udid)
        .or_else(|| chrome_screen_size(bridge, udid))
        .unwrap_or((390.0, 844.0))
}

fn parse_key_list(value: &str) -> Result<Vec<u16>, crate::error::AppError> {
    let mut keys = Vec::new();
    for token in value
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        keys.push(parse_hid_key(token)?);
    }
    if keys.is_empty() {
        return Err(crate::error::AppError::bad_request(
            "Key sequence must include at least one key.",
        ));
    }
    Ok(keys)
}

fn parse_hid_key(value: &str) -> Result<u16, crate::error::AppError> {
    if let Ok(code) = value.parse::<u16>() {
        return Ok(code);
    }
    let key = match value.to_lowercase().as_str() {
        "enter" | "return" => HID_KEY_ENTER,
        "escape" | "esc" => 41,
        "backspace" | "delete" => 42,
        "tab" => 43,
        "space" => 44,
        "right" | "arrow-right" => HID_KEY_ARROW_RIGHT,
        "left" | "arrow-left" => HID_KEY_ARROW_LEFT,
        "down" | "arrow-down" => HID_KEY_ARROW_DOWN,
        "up" | "arrow-up" => HID_KEY_ARROW_UP,
        "home" => 74,
        "end" => 77,
        other if other.len() == 1 => hid_for_character(other.chars().next().unwrap())
            .map(|(key, _)| key)
            .ok_or_else(|| {
                crate::error::AppError::bad_request(format!("Unsupported key `{value}`."))
            })?,
        _ => {
            return Err(crate::error::AppError::bad_request(format!(
                "Unsupported key `{value}`."
            )))
        }
    };
    Ok(key)
}

fn parse_modifier_mask(value: &str) -> Result<u32, crate::error::AppError> {
    let mut mask = 0;
    for token in value
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        mask |= match token.to_lowercase().as_str() {
            "shift" | "225" | "left-shift" => 1,
            "ctrl" | "control" | "224" | "left-control" => 1 << 1,
            "alt" | "option" | "226" | "left-option" => 1 << 2,
            "cmd" | "command" | "meta" | "227" | "left-command" => 1 << 3,
            "caps" | "caps-lock" | "57" => 1 << 4,
            other => {
                return Err(crate::error::AppError::bad_request(format!(
                    "Unsupported modifier `{other}`."
                )))
            }
        };
    }
    Ok(mask)
}
