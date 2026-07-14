use clap::ValueEnum;
use serde_json::{json, Map, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum AccessibilitySource {
    #[value(name = "auto")]
    Auto,
    #[value(name = "nativescript", alias = "ns")]
    NativeScript,
    #[value(name = "react-native", alias = "reactnative", alias = "rn")]
    ReactNative,
    #[value(name = "flutter", alias = "fl")]
    Flutter,
    #[value(name = "swiftui", alias = "swift-ui")]
    SwiftUI,
    #[value(name = "uikit", alias = "in-app-inspector")]
    UIKit,
    #[value(name = "native-ax", alias = "ax", alias = "native-accessibility")]
    NativeAX,
    #[value(name = "android-uiautomator")]
    AndroidUiautomator,
}

impl AccessibilitySource {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("auto").trim().to_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "nativescript" | "ns" => Ok(Self::NativeScript),
            "react-native" | "reactnative" | "rn" => Ok(Self::ReactNative),
            "flutter" | "fl" => Ok(Self::Flutter),
            "swiftui" | "swift-ui" => Ok(Self::SwiftUI),
            "uikit" | "in-app-inspector" => Ok(Self::UIKit),
            "ax" | "native-ax" | "native-accessibility" => Ok(Self::NativeAX),
            "android-uiautomator" => Ok(Self::AndroidUiautomator),
            source => Err(format!(
                "Unsupported accessibility hierarchy source `{source}`."
            )),
        }
    }

    pub fn as_query_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::NativeScript => "nativescript",
            Self::ReactNative => "react-native",
            Self::Flutter => "flutter",
            Self::SwiftUI => "swiftui",
            Self::UIKit => "uikit",
            Self::NativeAX => "native-ax",
            Self::AndroidUiautomator => "android-uiautomator",
        }
    }
}

/// Rotate device-native (portrait) accessibility frames into the current
/// display (interface) orientation, in place.
///
/// The private CoreSimulator accessibility API reports element frames in the
/// device's *native* coordinate space. For an app that renders rotated (e.g. a
/// landscape-locked iPad app, or the SpringBoard home screen), descendant
/// frames come back in portrait axes even though the video/display is
/// landscape — so `describe` annotations and selector taps land 90° off.
///
/// Not every app is affected: apps that lay their views out natively in the
/// current orientation (e.g. Settings) already report display-space frames.
/// We therefore rotate a root's subtree only when the subtree itself proves it
/// is in portrait space: a descendant extends below the landscape display
/// height, which is only valid on the native portrait canvas. Conversely, a
/// descendant that extends past the portrait width proves the subtree is
/// already in display space. Ambiguous small left-side subtrees are left alone
/// rather than guessing. The top-level application element reports its own
/// frame in display space regardless, so it (and any oversized outlier) is left
/// untouched.
///
/// Only landscape orientations (odd quarter turns) are handled. Portrait (0)
/// needs no change, and upside-down portrait (2) is left as-is.
pub fn normalize_native_ax_orientation(snapshot: &mut Value, quarter_turns: i32) {
    let quarter_turns = quarter_turns.rem_euclid(4);
    if quarter_turns != 1 && quarter_turns != 3 {
        return;
    }
    let Some(roots) = snapshot.get_mut("roots").and_then(Value::as_array_mut) else {
        return;
    };
    // In landscape the display is wider than it is tall. Derive the display
    // dimensions (in points) from the largest root frame, which the AX API
    // reports in display space.
    let Some((display_width, display_height)) = display_size_from_roots(roots) else {
        return;
    };
    for root in roots.iter_mut() {
        match subtree_orientation(root, display_height) {
            SubtreeOrientation::PortraitNative => {
                rotate_descendant_frames(root, quarter_turns, display_width, display_height);
            }
            SubtreeOrientation::DisplaySpace | SubtreeOrientation::Ambiguous => {}
        }
    }
}

fn frame_rect(node: &Value) -> Option<(f64, f64, f64, f64)> {
    let frame = node.get("frame")?;
    Some((
        frame.get("x")?.as_f64()?,
        frame.get("y")?.as_f64()?,
        frame.get("width")?.as_f64()?,
        frame.get("height")?.as_f64()?,
    ))
}

/// Landscape display size `(width, height)` with `width >= height`, taken from
/// the largest root frame (reported by the AX API in display space).
fn display_size_from_roots(roots: &[Value]) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for root in roots {
        if let Some((_, _, width, height)) = frame_rect(root) {
            if width > 0.0 && height > 0.0 && best.is_none_or(|(bw, bh)| width * height > bw * bh) {
                best = Some((width, height));
            }
        }
    }
    let (width, height) = best?;
    Some((width.max(height), width.min(height)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubtreeOrientation {
    PortraitNative,
    DisplaySpace,
    Ambiguous,
}

/// Classify the coordinate space of a root's descendants.
///
/// In landscape, the natural portrait canvas is `display_height` points wide
/// and `display_width` points tall. A descendant whose right edge exceeds
/// `display_height` must already be in display space. A descendant whose bottom
/// edge exceeds `display_height` proves portrait-native coordinates, because
/// that point would be outside the visible landscape display.
fn subtree_orientation(root: &Value, display_height: f64) -> SubtreeOrientation {
    fn visit(node: &Value, threshold: f64) -> SubtreeOrientation {
        let Some(children) = node.get("children").and_then(Value::as_array) else {
            return SubtreeOrientation::Ambiguous;
        };
        let mut orientation = SubtreeOrientation::Ambiguous;
        for child in children {
            if let Some((x, y, width, height)) = frame_rect(child) {
                if x + width > threshold {
                    return SubtreeOrientation::DisplaySpace;
                }
                if y + height > threshold {
                    orientation = SubtreeOrientation::PortraitNative;
                }
            }
            match visit(child, threshold) {
                SubtreeOrientation::DisplaySpace => return SubtreeOrientation::DisplaySpace,
                SubtreeOrientation::PortraitNative => {
                    orientation = SubtreeOrientation::PortraitNative;
                }
                SubtreeOrientation::Ambiguous => {}
            }
        }
        orientation
    }
    // +1pt slack so an element spanning exactly the portrait width is not a
    // false positive.
    visit(root, display_height + 1.0)
}

fn rotate_descendant_frames(root: &mut Value, quarter_turns: i32, display_w: f64, display_h: f64) {
    let Some(children) = root.get_mut("children").and_then(Value::as_array_mut) else {
        return;
    };
    for child in children.iter_mut() {
        rotate_node_frame(child, quarter_turns, display_w, display_h);
    }
}

fn rotate_node_frame(node: &mut Value, quarter_turns: i32, display_w: f64, display_h: f64) {
    if let Some((x, y, width, height)) = frame_rect(node) {
        // Only rotate frames that fit the portrait canvas (portrait width ==
        // display height, portrait height == display width). Anything larger is
        // an already-display-space outlier and is left untouched.
        if width <= display_h + 1.0 && height <= display_w + 1.0 {
            let (nx, ny, nw, nh) =
                rotate_rect(x, y, width, height, quarter_turns, display_w, display_h);
            if let Some(frame) = node.get_mut("frame").and_then(Value::as_object_mut) {
                frame.insert("x".to_owned(), json!(round_tenths(nx)));
                frame.insert("y".to_owned(), json!(round_tenths(ny)));
                frame.insert("width".to_owned(), json!(round_tenths(nw)));
                frame.insert("height".to_owned(), json!(round_tenths(nh)));
            }
        }
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children.iter_mut() {
            rotate_node_frame(child, quarter_turns, display_w, display_h);
        }
    }
}

/// Map a rect from device-native portrait axes into the landscape display,
/// swapping width/height. Portrait canvas is `display_h` wide by `display_w`
/// tall.
fn rotate_rect(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    quarter_turns: i32,
    display_w: f64,
    display_h: f64,
) -> (f64, f64, f64, f64) {
    match quarter_turns {
        3 => (y, display_h - x - width, height, width),
        1 => (display_w - y - height, x, height, width),
        _ => (x, y, width, height),
    }
}

fn round_tenths(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

pub fn interactive_accessibility_snapshot(snapshot: &Value) -> Value {
    let mut output = snapshot.as_object().cloned().unwrap_or_default();
    let roots = snapshot
        .get("roots")
        .and_then(Value::as_array)
        .map(|roots| {
            roots
                .iter()
                .filter_map(interactive_accessibility_node)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    output.insert("roots".to_owned(), Value::Array(roots));
    output.insert("interactiveOnly".to_owned(), Value::Bool(true));
    Value::Object(output)
}

fn interactive_accessibility_node(node: &Value) -> Option<Value> {
    let object = node.as_object()?;
    let children = node
        .get("children")
        .and_then(Value::as_array)
        .map(|children| {
            children
                .iter()
                .filter_map(interactive_accessibility_node)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if !is_interactive_accessibility_node(node) && children.is_empty() {
        return None;
    }

    let mut output = object.clone();
    if children.is_empty() {
        output.remove("children");
    } else {
        output.insert("children".to_owned(), Value::Array(children));
    }
    Some(Value::Object(output))
}

fn is_interactive_accessibility_node(node: &Value) -> bool {
    if bool_field(node, &["hidden", "isHidden"]).unwrap_or(false) {
        return false;
    }
    if numeric_field(node, &["alpha"]).is_some_and(|alpha| alpha <= 0.01) {
        return false;
    }

    if has_actionable_action(node) {
        return true;
    }
    if bool_field(
        node,
        &[
            "clickable",
            "focusable",
            "isUserInteractionEnabled",
            "scrollable",
            "checked",
            "selected",
        ],
    )
    .unwrap_or(false)
    {
        return true;
    }

    string_field(
        node,
        &[
            "type",
            "role",
            "className",
            "elementType",
            "displayName",
            "widgetType",
        ],
    )
    .is_some_and(|role| role_looks_interactive(&role))
}

fn has_actionable_action(node: &Value) -> bool {
    for actions in [
        node.get("actions"),
        node.get("custom_actions"),
        node.get("control")
            .and_then(|control| control.get("actions")),
    ]
    .into_iter()
    .flatten()
    {
        if actions
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(action_looks_interactive)
        {
            return true;
        }
    }
    false
}

fn action_looks_interactive(action: &str) -> bool {
    let action = action.trim().to_ascii_lowercase();
    !action.is_empty()
        && !matches!(
            action.as_str(),
            "describe" | "getproperties" | "get_properties" | "highlight"
        )
}

fn role_looks_interactive(role: &str) -> bool {
    let role = role.to_ascii_lowercase();
    [
        "button",
        "cell",
        "checkbox",
        "collection",
        "combobox",
        "control",
        "edittext",
        "link",
        "menu",
        "picker",
        "radio",
        "scroll",
        "search",
        "segmented",
        "select",
        "slider",
        "stepper",
        "switch",
        "tab",
        "table",
        "textfield",
        "text field",
        "textinput",
        "text input",
        "toggle",
        "webview",
    ]
    .iter()
    .any(|needle| role.contains(needle))
}

fn bool_field(node: &Value, fields: &[&str]) -> Option<bool> {
    fields.iter().find_map(|field| nested_bool(node, field))
}

fn numeric_field(node: &Value, fields: &[&str]) -> Option<f64> {
    fields.iter().find_map(|field| nested_number(node, field))
}

fn string_field(node: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| nested_string(node, field))
}

fn nested_bool(node: &Value, field: &str) -> Option<bool> {
    node.get(field)
        .and_then(Value::as_bool)
        .or_else(|| {
            nested_object(node, "accessibility").and_then(|object| bool_from_map(object, field))
        })
        .or_else(|| nested_object(node, "control").and_then(|object| bool_from_map(object, field)))
}

fn nested_number(node: &Value, field: &str) -> Option<f64> {
    node.get(field)
        .and_then(Value::as_f64)
        .or_else(|| {
            nested_object(node, "accessibility").and_then(|object| number_from_map(object, field))
        })
        .or_else(|| {
            nested_object(node, "control").and_then(|object| number_from_map(object, field))
        })
}

fn nested_string(node: &Value, field: &str) -> Option<String> {
    node.get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            nested_object(node, "accessibility").and_then(|object| string_from_map(object, field))
        })
        .or_else(|| {
            nested_object(node, "control").and_then(|object| string_from_map(object, field))
        })
}

fn nested_object<'a>(node: &'a Value, field: &str) -> Option<&'a Map<String, Value>> {
    node.get(field).and_then(Value::as_object)
}

fn bool_from_map(object: &Map<String, Value>, field: &str) -> Option<bool> {
    object.get(field).and_then(Value::as_bool)
}

fn number_from_map(object: &Map<String, Value>, field: &str) -> Option<f64> {
    object.get(field).and_then(Value::as_f64)
}

fn string_from_map(object: &Map<String, Value>, field: &str) -> Option<String> {
    object.get(field).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod orientation_tests {
    use super::normalize_native_ax_orientation;
    use serde_json::{json, Value};

    fn frame(node: &Value, path: &[usize]) -> (f64, f64, f64, f64) {
        let mut current = node;
        for &index in path {
            current = &current["children"][index];
        }
        let frame = &current["frame"];
        (
            frame["x"].as_f64().unwrap(),
            frame["y"].as_f64().unwrap(),
            frame["width"].as_f64().unwrap(),
            frame["height"].as_f64().unwrap(),
        )
    }

    // A real SpringBoard capture (iPad Pro 11", rotationQuarterTurns == 3):
    // the application root reports display-space (1210x834) but the icon
    // children are in device-native portrait axes.
    fn springboard_snapshot() -> Value {
        json!({
            "source": "native-ax",
            "roots": [{
                "role": "Application",
                "frame": { "x": 0, "y": 0, "width": 1210, "height": 834 },
                "children": [
                    { "role": "Button", "AXLabel": "Fitness",
                      "frame": { "x": 657, "y": 979, "width": 95.5, "height": 72 } },
                    { "role": "Button", "AXLabel": "Messages",
                      "frame": { "x": 27.5, "y": 342.5, "width": 68, "height": 68 } }
                ]
            }]
        })
    }

    // A real Settings capture (same device/orientation): the app lays out in
    // landscape, so its frames are already in display space (the detail pane
    // button spans nearly the full 1210 width).
    fn settings_snapshot() -> Value {
        json!({
            "source": "native-ax",
            "roots": [{
                "role": "Application",
                "frame": { "x": 0, "y": 0, "width": 1210, "height": 834 },
                "children": [
                    { "role": "Group", "AXLabel": "Sidebar",
                      "frame": { "x": 10, "y": 32, "width": 320, "height": 792 } },
                    { "role": "Button", "AXLabel": "About",
                      "frame": { "x": 346, "y": 326.5, "width": 848, "height": 53 } }
                ]
            }]
        })
    }

    #[test]
    fn rotates_portrait_subtree_for_quarter_turns_3() {
        let mut snapshot = springboard_snapshot();
        normalize_native_ax_orientation(&mut snapshot, 3);
        let root = &snapshot["roots"][0];
        // Root stays in display space (it is not a portrait-canvas frame).
        assert_eq!(frame(root, &[]), (0.0, 0.0, 1210.0, 834.0));
        // Fitness: (x,y,w,h) -> (y, Hd - x - w, h, w) with Hd = 834.
        assert_eq!(frame(root, &[0]), (979.0, 81.5, 72.0, 95.5));
        // Messages sits at portrait-left (x~27) => landscape bottom (y~738).
        assert_eq!(frame(root, &[1]), (342.5, 738.5, 68.0, 68.0));
    }

    #[test]
    fn rotates_portrait_subtree_for_quarter_turns_1() {
        let mut snapshot = springboard_snapshot();
        normalize_native_ax_orientation(&mut snapshot, 1);
        let root = &snapshot["roots"][0];
        // (x,y,w,h) -> (Wd - y - h, x, h, w) with Wd = 1210.
        assert_eq!(
            frame(root, &[0]),
            (1210.0 - 979.0 - 72.0, 657.0, 72.0, 95.5)
        );
        assert_eq!(frame(root, &[1]), (1210.0 - 342.5 - 68.0, 27.5, 68.0, 68.0));
    }

    #[test]
    fn leaves_display_space_subtree_untouched() {
        let mut snapshot = settings_snapshot();
        let before = snapshot.clone();
        normalize_native_ax_orientation(&mut snapshot, 3);
        // The "About" button (x+width = 1194 > portrait width) proves the whole
        // subtree is already display-space, so nothing is rotated.
        assert_eq!(snapshot, before);
    }

    #[test]
    fn leaves_ambiguous_left_side_subtree_untouched() {
        let mut snapshot = json!({
            "source": "native-ax",
            "roots": [{
                "role": "Application",
                "frame": { "x": 0, "y": 0, "width": 1210, "height": 834 },
                "children": [
                    { "role": "Button", "AXLabel": "Narrow",
                      "frame": { "x": 24, "y": 96, "width": 240, "height": 56 } }
                ]
            }]
        });
        let before = snapshot.clone();
        normalize_native_ax_orientation(&mut snapshot, 3);
        assert_eq!(snapshot, before);
    }

    #[test]
    fn portrait_and_upside_down_are_noops() {
        for quarter_turns in [0, 2, 4] {
            let mut snapshot = springboard_snapshot();
            let before = snapshot.clone();
            normalize_native_ax_orientation(&mut snapshot, quarter_turns);
            assert_eq!(
                snapshot, before,
                "quarter_turns {quarter_turns} must be a no-op"
            );
        }
    }
}
