#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

MODEL_DIR="models"
MODEL_FILE="functiongemma-270m-ft-mobile-actions-q8.task"
MODEL_URL="https://huggingface.co/litert-community/FunctionGemma_270M_Mobile_Actions/resolve/main/${MODEL_FILE}"

mkdir -p "$MODEL_DIR"

if [ -f "$MODEL_DIR/$MODEL_FILE" ]; then
    echo "Model already downloaded at $MODEL_DIR/$MODEL_FILE"
    echo "Delete it and re-run this script to re-download."
    exit 0
fi

echo "=== Downloading FunctionGemma 270M Mobile Actions ==="
echo "Source: https://huggingface.co/litert-community/FunctionGemma_270M_Mobile_Actions"
echo "File:   $MODEL_FILE"
echo ""

# Try curl first, fall back to wget
if command -v curl &> /dev/null; then
    curl -L --progress-bar -o "$MODEL_DIR/$MODEL_FILE" "$MODEL_URL"
elif command -v wget &> /dev/null; then
    wget --show-progress -O "$MODEL_DIR/$MODEL_FILE" "$MODEL_URL"
else
    echo "ERROR: Neither curl nor wget found. Install one and retry."
    exit 1
fi

FILE_SIZE=$(wc -c < "$MODEL_DIR/$MODEL_FILE" | tr -d ' ')
echo ""
echo "Download complete: $MODEL_DIR/$MODEL_FILE ($FILE_SIZE bytes)"
echo ""
echo "To bundle it into the iOS app at build time, add it to Xcode as a resource,"
echo "or let the app download it automatically on first launch."
