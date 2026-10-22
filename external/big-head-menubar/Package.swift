// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "BigHeadMenuBar",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(name: "BigHeadMenuBar", targets: ["BigHeadMenuBar"])
    ],
    targets: [
        .executableTarget(
            name: "BigHeadMenuBar",
            path: "Sources"
        )
    ]
)
