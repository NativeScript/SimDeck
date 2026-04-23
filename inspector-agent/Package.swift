// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "XcodeCanvasInspectorAgent",
    platforms: [
        .iOS(.v13)
    ],
    products: [
        .library(
            name: "XcodeCanvasInspectorAgent",
            targets: ["XcodeCanvasInspectorAgent"]
        )
    ],
    targets: [
        .target(
            name: "XcodeCanvasInspectorAgent",
            path: "Sources/XcodeCanvasInspectorAgent"
        )
    ]
)
