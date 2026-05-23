use serde_json::{Map, Value};

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
