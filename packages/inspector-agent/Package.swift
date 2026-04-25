// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "SimDeckInspectorAgent",
    platforms: [
        .iOS(.v13)
    ],
    products: [
        .library(
            name: "SimDeckInspectorAgent",
            targets: ["SimDeckInspectorAgent"]
        )
    ],
    targets: [
        .target(
            name: "SimDeckInspectorAgent",
            path: "Sources/SimDeckInspectorAgent"
        )
    ]
)
