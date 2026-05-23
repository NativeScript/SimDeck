#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct ElementSelectorPayload {
    text: Option<String>,
    id: Option<String>,
    label: Option<String>,
    value: Option<String>,
    #[serde(alias = "type")]
    element_type: Option<String>,
    index: Option<usize>,
    enabled: Option<bool>,
    checked: Option<bool>,
    focused: Option<bool>,
    selected: Option<bool>,
    regex: Option<bool>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AccessibilityQueryPayload {
    #[serde(default)]
    selector: ElementSelectorPayload,
    source: Option<String>,
    max_depth: Option<usize>,
    include_hidden: Option<bool>,
    limit: Option<usize>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WaitForPayload {
    #[serde(default)]
    selector: ElementSelectorPayload,
    source: Option<String>,
    max_depth: Option<usize>,
    include_hidden: Option<bool>,
    timeout_ms: Option<u64>,
    poll_ms: Option<u64>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScrollUntilVisiblePayload {
    #[serde(default)]
    selector: ElementSelectorPayload,
    source: Option<String>,
    max_depth: Option<usize>,
    include_hidden: Option<bool>,
    timeout_ms: Option<u64>,
    poll_ms: Option<u64>,
    direction: Option<String>,
    duration_ms: Option<u64>,
    steps: Option<u32>,
}

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct TapElementPayload {
    x: Option<f64>,
    y: Option<f64>,
    normalized: Option<bool>,
    #[serde(default)]
    selector: ElementSelectorPayload,
    source: Option<String>,
    max_depth: Option<usize>,
    include_hidden: Option<bool>,
    wait_timeout_ms: Option<u64>,
    poll_ms: Option<u64>,
    duration_ms: Option<u64>,
    expect: Option<Box<WaitForPayload>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchPayload {
    steps: Vec<BatchStep>,
    continue_on_error: Option<bool>,
}

#[derive(Deserialize, Clone)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum BatchStep {
    Sleep {
        ms: Option<u64>,
        seconds: Option<f64>,
    },
    Tap(TapElementPayload),
    Back {
        timeout_ms: Option<u64>,
        poll_ms: Option<u64>,
        fallback_swipe: Option<bool>,
    },
    WaitFor(WaitForPayload),
    Assert(WaitForPayload),
    #[serde(alias = "waitForNot")]
    AssertNot(WaitForPayload),
    Query(AccessibilityQueryPayload),
    ScrollUntilVisible(ScrollUntilVisiblePayload),
    Key {
        key_code: u16,
        modifiers: Option<u32>,
    },
    KeySequence {
        key_codes: Vec<u16>,
        delay_ms: Option<u64>,
    },
    Touch {
        x: f64,
        y: f64,
        phase: Option<String>,
        down: Option<bool>,
        up: Option<bool>,
        delay_ms: Option<u64>,
    },
    TouchSequence {
        events: Vec<TouchSequenceEvent>,
    },
    EdgeTouch {
        x: f64,
        y: f64,
        phase: String,
        edge: String,
    },
    MultiTouch {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        phase: String,
    },
    Swipe {
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration_ms: Option<u64>,
        steps: Option<u32>,
    },
    Gesture {
        preset: String,
        duration_ms: Option<u64>,
        delta: Option<f64>,
        steps: Option<u32>,
    },
    Type {
        text: String,
        delay_ms: Option<u64>,
    },
    Button {
        button: String,
        duration_ms: Option<u32>,
        phase: Option<String>,
        usage_page: Option<u32>,
        usage: Option<u32>,
    },
    Crown {
        delta: f64,
    },
    Launch {
        bundle_id: String,
    },
    OpenUrl {
        url: String,
    },
    Home,
    DismissKeyboard,
    AppSwitcher,
    RotateLeft,
    RotateRight,
    ToggleAppearance,
    Describe {
        source: Option<String>,
        max_depth: Option<usize>,
        include_hidden: Option<bool>,
    },
}
