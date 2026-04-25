import UIKit

final class ViewInteractionPerformer {
    static func actions(for view: UIView) -> [String] {
        var actions = ["describe"]

        if view is UIControl || view.isAccessibilityElement {
            actions.append("tap")
        }
        if view.canBecomeFirstResponder {
            actions.append("focus")
        }
        if view.isFirstResponder {
            actions.append("resignFirstResponder")
        }
        if view is UITextField || view is UITextView {
            actions.append("setText")
        }
        if view is UISwitch {
            actions.append(contentsOf: ["toggle", "setValue"])
        }
        if view is UISlider || view is UISegmentedControl {
            actions.append("setValue")
        }
        if view is UIScrollView {
            actions.append(contentsOf: ["scrollBy", "scrollTo"])
        }
        if view.isAccessibilityElement {
            actions.append("accessibilityActivate")
        }

        return Array(Set(actions)).sorted()
    }

    func perform(action: String, on view: UIView, params: [String: JSONValue]) throws -> JSONValue {
        switch action {
        case "describe":
            return .object([
                "ok": .bool(true),
                "id": .string(ViewHierarchySnapshotter.id(for: view)),
                "actions": .array(Self.actions(for: view).map(JSONValue.string)),
            ])
        case "tap":
            return try tap(view)
        case "focus":
            return boolResult(view.becomeFirstResponder(), action: action)
        case "resignFirstResponder":
            return boolResult(view.resignFirstResponder(), action: action)
        case "accessibilityActivate":
            return boolResult(view.accessibilityActivate(), action: action)
        case "setText":
            guard let value = params.string("value") else {
                throw InspectorFailure.invalidRequest("View.perform setText requires params.value.")
            }
            return try setText(value, on: view)
        case "setValue":
            return try setValue(on: view, params: params)
        case "toggle":
            return try toggle(view)
        case "scrollBy":
            return try scroll(view, params: params, relative: true)
        case "scrollTo":
            return try scroll(view, params: params, relative: false)
        default:
            throw InspectorFailure.unsupportedAction(action)
        }
    }

    private func tap(_ view: UIView) throws -> JSONValue {
        if let `switch` = view as? UISwitch {
            `switch`.setOn(!`switch`.isOn, animated: true)
            `switch`.sendActions(for: .valueChanged)
            return boolResult(true, action: "tap")
        }

        if let control = view as? UIControl {
            control.sendActions(for: .primaryActionTriggered)
            control.sendActions(for: .touchUpInside)
            return boolResult(true, action: "tap")
        }

        if view.accessibilityActivate() {
            return boolResult(true, action: "tap")
        }

        if view.canBecomeFirstResponder {
            return boolResult(view.becomeFirstResponder(), action: "tap")
        }

        throw InspectorFailure.actionFailed("The target view does not expose a safe tap action.")
    }

    private func setText(_ text: String, on view: UIView) throws -> JSONValue {
        if let textField = view as? UITextField {
            textField.text = text
            textField.sendActions(for: .editingChanged)
            return boolResult(true, action: "setText")
        }

        if let textView = view as? UITextView {
            textView.text = text
            NotificationCenter.default.post(name: UITextView.textDidChangeNotification, object: textView)
            return boolResult(true, action: "setText")
        }

        throw InspectorFailure.actionFailed("setText is only supported for UITextField and UITextView.")
    }

    private func setValue(on view: UIView, params: [String: JSONValue]) throws -> JSONValue {
        guard let value = params["value"] else {
            throw InspectorFailure.invalidRequest("View.perform setValue requires params.value.")
        }

        if let `switch` = view as? UISwitch {
            guard let boolValue = value.boolValue else {
                throw InspectorFailure.invalidRequest("UISwitch setValue requires a boolean value.")
            }
            `switch`.setOn(boolValue, animated: true)
            `switch`.sendActions(for: .valueChanged)
            return boolResult(true, action: "setValue")
        }

        if let slider = view as? UISlider {
            guard let doubleValue = value.doubleValue else {
                throw InspectorFailure.invalidRequest("UISlider setValue requires a numeric value.")
            }
            slider.value = Float(doubleValue)
            slider.sendActions(for: .valueChanged)
            return boolResult(true, action: "setValue")
        }

        if let segmented = view as? UISegmentedControl {
            guard let index = value.intValue else {
                throw InspectorFailure.invalidRequest("UISegmentedControl setValue requires a numeric segment index.")
            }
            guard index >= 0, index < segmented.numberOfSegments else {
                throw InspectorFailure.invalidRequest("Segment index \(index) is out of bounds.")
            }
            segmented.selectedSegmentIndex = index
            segmented.sendActions(for: .valueChanged)
            return boolResult(true, action: "setValue")
        }

        throw InspectorFailure.actionFailed("setValue is only supported for UISwitch, UISlider, and UISegmentedControl.")
    }

    private func toggle(_ view: UIView) throws -> JSONValue {
        guard let `switch` = view as? UISwitch else {
            throw InspectorFailure.actionFailed("toggle is only supported for UISwitch.")
        }

        `switch`.setOn(!`switch`.isOn, animated: true)
        `switch`.sendActions(for: .valueChanged)
        return boolResult(true, action: "toggle")
    }

    private func scroll(_ view: UIView, params: [String: JSONValue], relative: Bool) throws -> JSONValue {
        guard let scrollView = view as? UIScrollView else {
            throw InspectorFailure.actionFailed("scroll actions are only supported for UIScrollView.")
        }

        let x = CGFloat(params.double("x") ?? 0)
        let y = CGFloat(params.double("y") ?? 0)
        let animated = params.bool("animated") ?? false
        let target: CGPoint
        if relative {
            target = CGPoint(
                x: scrollView.contentOffset.x + x,
                y: scrollView.contentOffset.y + y
            )
        } else {
            target = CGPoint(x: x, y: y)
        }

        scrollView.setContentOffset(clamped(target, for: scrollView), animated: animated)
        return .object([
            "ok": .bool(true),
            "action": .string(relative ? "scrollBy" : "scrollTo"),
            "contentOffset": .object([
                "x": .number(Double(scrollView.contentOffset.x)),
                "y": .number(Double(scrollView.contentOffset.y)),
            ]),
        ])
    }

    private func clamped(_ point: CGPoint, for scrollView: UIScrollView) -> CGPoint {
        let inset = scrollView.adjustedContentInset
        let minX = -inset.left
        let minY = -inset.top
        let maxX = max(minX, scrollView.contentSize.width - scrollView.bounds.width + inset.right)
        let maxY = max(minY, scrollView.contentSize.height - scrollView.bounds.height + inset.bottom)
        return CGPoint(
            x: min(max(point.x, minX), maxX),
            y: min(max(point.y, minY), maxY)
        )
    }

    private func boolResult(_ ok: Bool, action: String) -> JSONValue {
        .object([
            "ok": .bool(ok),
            "action": .string(action),
        ])
    }
}
