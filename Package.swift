// swift-tools-version: 5.9
import PackageDescription

// This file is automatically updated by CI after each release.
// The URL and checksum below are updated to point to the latest .xcframework.zip.
let package = Package(
    name: "MinigrafKit",
    platforms: [
        .iOS(.v16),
    ],
    products: [
        .library(
            name: "MinigrafKit",
            targets: ["minigrafFFI", "MinigrafKit"]
        ),
    ],
    targets: [
        .binaryTarget(
            name: "minigrafFFI",
            // Updated by CI: release-upload-mobile job
            url: "https://github.com/project-minigraf/minigraf/releases/download/v0.24.0/MinigrafKit-v0.24.0.xcframework.zip",
            checksum: "e19b69cf8c1e73106fc005dd25adca10782e2f5a9a93e75d71a03102eb34212c"
        ),
        .target(
            name: "MinigrafKit",
            dependencies: [.target(name: "minigrafFFI")],
            path: "swift/Sources/MinigrafKit"
        ),
    ]
)
