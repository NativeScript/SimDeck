fn print_describe_ui(snapshot: &Value, format: DescribeUiFormat) -> anyhow::Result<()> {
    match format {
        DescribeUiFormat::Json => println_json(snapshot),
        DescribeUiFormat::CompactJson => {
            println!(
                "{}",
                serde_json::to_string(&compact_accessibility_snapshot(snapshot))?
            );
            Ok(())
        }
        DescribeUiFormat::Agent => {
            print!("{}", render_agent_accessibility_tree(snapshot));
            Ok(())
        }
    }
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
    let mut object = serde_json::Map::new();
    object.insert(
        "source".to_owned(),
        snapshot
            .get("source")
            .cloned()
            .unwrap_or_else(|| Value::String("unknown".to_owned())),
    );
    object.insert("roots".to_owned(), Value::Array(roots));
    for field in ["availableSources", "fallbackReason", "fallbackSource"] {
        if let Some(value) = snapshot.get(field) {
            object.insert(field.to_owned(), value.clone());
        }
    }
    Value::Object(object)
}

fn compact_accessibility_node(node: &Value) -> Value {
    let mut object = serde_json::Map::new();
    insert_string_alias(node, &mut object, "role", &["type", "role", "className"]);
    insert_string_alias(
        node,
        &mut object,
        "id",
        &["AXIdentifier", "AXUniqueId", "inspectorId", "id"],
    );
    insert_string_alias(
        node,
        &mut object,
        "label",
        &["AXLabel", "label", "title", "text", "name"],
    );
    insert_string_alias(
        node,
        &mut object,
        "value",
        &["AXValue", "value", "placeholder"],
    );
    if let Some(frame) = compact_frame(node.get("frame").or_else(|| node.get("frameInScreen"))) {
        object.insert("frame".to_owned(), frame);
    }
    if truthy_field(node, "hidden").unwrap_or(false)
        || truthy_field(node, "isHidden").unwrap_or(false)
    {
        object.insert("hidden".to_owned(), Value::Bool(true));
    }
    if let Some(false) = truthy_field(node, "enabled") {
        object.insert("enabled".to_owned(), Value::Bool(false));
    }
    if let Some(actions) = node
        .get("custom_actions")
        .or_else(|| {
            node.get("control")
                .and_then(|control| control.get("actions"))
        })
        .and_then(Value::as_array)
    {
        let actions = actions
            .iter()
            .filter_map(Value::as_str)
            .map(|action| Value::String(action.to_owned()))
            .collect::<Vec<_>>();
        if !actions.is_empty() {
            object.insert("actions".to_owned(), Value::Array(actions));
        }
    }
    if let Some(source_location) = node.get("sourceLocation").filter(|value| !value.is_null()) {
        object.insert("sourceLocation".to_owned(), source_location.clone());
    }
    let children = node
        .get("children")
        .and_then(Value::as_array)
        .map(|children| {
            children
                .iter()
                .map(compact_accessibility_node)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !children.is_empty() {
        object.insert("children".to_owned(), Value::Array(children));
    }
    Value::Object(object)
}

fn insert_string_alias(
    source: &Value,
    target: &mut serde_json::Map<String, Value>,
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

fn compact_frame(frame: Option<&Value>) -> Option<Value> {
    let frame = frame?;
    let x = frame.get("x")?.as_f64()?;
    let y = frame.get("y")?.as_f64()?;
    let width = frame.get("width")?.as_f64()?;
    let height = frame.get("height")?.as_f64()?;
    Some(serde_json::json!([
        round_frame_value(x),
        round_frame_value(y),
        round_frame_value(width),
        round_frame_value(height)
    ]))
}

fn round_frame_value(value: f64) -> Value {
    let rounded = (value * 10.0).round() / 10.0;
    serde_json::Number::from_f64(rounded)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn truthy_field(node: &Value, field: &str) -> Option<bool> {
    node.get(field).and_then(Value::as_bool)
}

fn render_agent_accessibility_tree(snapshot: &Value) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "source: {}",
        snapshot
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    ));
    if let Some(sources) = snapshot.get("availableSources").and_then(Value::as_array) {
        let sources = sources
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(",");
        if !sources.is_empty() {
            lines.push(format!("available: {sources}"));
        }
    }
    if let Some(reason) = snapshot.get("fallbackReason").and_then(Value::as_str) {
        lines.push(format!("fallback: {}", compact_text(reason)));
    }
    if let Some(roots) = snapshot.get("roots").and_then(Value::as_array) {
        let mut next_ref = 1usize;
        for root in roots {
            render_agent_node(root, 0, &mut lines, &mut next_ref);
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn render_agent_node(node: &Value, depth: usize, lines: &mut Vec<String>, next_ref: &mut usize) {
    let compact = compact_accessibility_node(node);
    let object = compact.as_object();
    let field = |name| {
        object
            .and_then(|object| object.get(name))
            .and_then(Value::as_str)
            .map(compact_text)
            .filter(|value| !value.is_empty())
    };
    let role = field("role").unwrap_or_else(|| "View".to_owned());
    let id = field("id");
    let label = field("label");
    let value = field("value");
    let agent_ref = *next_ref;
    *next_ref += 1;
    let mut line = format!("{}- @e{} {}", "  ".repeat(depth), agent_ref, role);
    if let Some(id) = id {
        line.push_str(" #");
        line.push_str(&id);
    }
    if let Some(label) = label.as_ref() {
        line.push_str(": ");
        line.push_str(label);
    }
    if let Some(value) = value.filter(|value| Some(value) != label.as_ref()) {
        line.push_str(" = ");
        line.push_str(&value);
    }
    if let Some(frame) = object
        .and_then(|object| object.get("frame"))
        .and_then(Value::as_array)
        .filter(|frame| frame.len() == 4)
    {
        line.push_str(&format!(
            " @{},{} {}x{}",
            frame_value(&frame[0]),
            frame_value(&frame[1]),
            frame_value(&frame[2]),
            frame_value(&frame[3])
        ));
    }
    let mut flags = Vec::new();
    if object
        .and_then(|object| object.get("hidden"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        flags.push("hidden");
    }
    if object
        .and_then(|object| object.get("enabled"))
        .and_then(Value::as_bool)
        == Some(false)
    {
        flags.push("disabled");
    }
    if let Some(actions) = object
        .and_then(|object| object.get("actions"))
        .and_then(Value::as_array)
    {
        let actions = actions.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        if !actions.is_empty() {
            line.push_str(" actions=");
            line.push_str(&actions.join(","));
        }
    }
    if !flags.is_empty() {
        line.push(' ');
        line.push_str(&flags.join(","));
    }
    lines.push(line);

    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            render_agent_node(child, depth + 1, lines, next_ref);
        }
    }
}

fn compact_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn frame_value(value: &Value) -> String {
    value
        .as_f64()
        .map(|value| {
            if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                format!("{value:.1}")
            }
        })
        .unwrap_or_else(|| "?".to_owned())
}
