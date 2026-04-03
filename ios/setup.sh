#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

echo "=== FunctionGemma iOS Runner Setup ==="
echo ""

# Check for Xcode
if ! command -v xcodebuild &> /dev/null; then
    echo "ERROR: Xcode is not installed. Install it from the App Store."
    exit 1
fi

# Check for XcodeGen
if ! command -v xcodegen &> /dev/null; then
    echo "XcodeGen not found. Installing via Homebrew..."
    if ! command -v brew &> /dev/null; then
        echo "ERROR: Homebrew is not installed. Install from https://brew.sh"
        exit 1
    fi
    brew install xcodegen
fi

# Check for CocoaPods
if ! command -v pod &> /dev/null; then
    echo "CocoaPods not found. Installing..."
    sudo gem install cocoapods
fi

echo ""
echo "--- Generating Xcode project ---"
xcodegen generate

echo ""
echo "--- Installing CocoaPods dependencies ---"
pod install

echo ""
echo "=== Setup complete! ==="
echo ""
echo "Open FunctionGemmaRunner.xcworkspace in Xcode:"
echo "  open FunctionGemmaRunner.xcworkspace"
echo ""
echo "Then select an iOS Simulator target and press Cmd+R to build & run."
echo "The app will automatically download the model on first launch."
echo ""
echo "Alternatively, pre-download the model with:"
echo "  ./download_model.sh"
